use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor};

use crate::domain::graph::{Span, Symbol, SymbolKind};
use crate::infra::graph::parser::SupportedLanguage;

pub(super) fn extract_symbols(
    repo: &str,
    root: Node,
    query: &Query,
    source_bytes: &[u8],
    lang: SupportedLanguage,
) -> Vec<Symbol> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source_bytes);

    let symbol_kind_idx = query.capture_index_for_name("symbol.kind");
    let symbol_name_idx = query.capture_index_for_name("symbol.name");

    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        let mut kind_node = None;
        let mut name_node = None;

        for cap in m.captures {
            if Some(cap.index) == symbol_kind_idx {
                kind_node = Some(cap.node);
            } else if Some(cap.index) == symbol_name_idx {
                name_node = Some(cap.node);
            }
        }

        if let (Some(k_node), Some(n_node)) = (kind_node, name_node) {
            let name = n_node.utf8_text(source_bytes).unwrap_or("").to_owned();
            if name.is_empty() {
                continue;
            }

            // `variable_declarator` also matches locals inside function bodies;
            // only module-level bindings are definitions.
            if k_node.kind() == "variable_declarator" && !is_module_scope_declarator(k_node) {
                continue;
            }

            let kind = if k_node.kind() == "variable_declarator" {
                declarator_kind(k_node)
            } else {
                determine_symbol_kind(k_node.kind(), lang)
            };
            let span = node_to_span(k_node);
            let is_exported = check_exported(k_node, source_bytes, lang);
            let signature = extract_signature(k_node, source_bytes);
            let docstring = extract_docstring(k_node, source_bytes, lang);
            let complexity = matches!(kind, SymbolKind::Fn | SymbolKind::Method)
                .then(|| cyclomatic_complexity(k_node, source_bytes));

            symbols.push(Symbol {
                id: None,
                file_id: None,
                repo: repo.to_owned(),
                name,
                kind,
                scope: None,
                signature,
                docstring,
                span,
                is_exported,
                complexity,
            });
        }
    }

    symbols
}

fn determine_symbol_kind(node_kind: &str, _lang: SupportedLanguage) -> SymbolKind {
    match node_kind {
        "function_item" | "function_declaration" => SymbolKind::Fn,
        "method_definition" => SymbolKind::Method,
        "struct_item" => SymbolKind::Struct,
        "class_declaration" => SymbolKind::Class,
        "trait_item" => SymbolKind::Trait,
        "interface_declaration" => SymbolKind::Interface,
        "enum_item" | "enum_declaration" => SymbolKind::Enum,
        "type_alias_declaration" => SymbolKind::Other("type_alias".to_owned()),
        "impl_item" => SymbolKind::Other("impl".to_owned()),
        "mod_item" => SymbolKind::Mod,
        "const_item" | "static_item" => SymbolKind::Const,
        other => SymbolKind::Other(other.to_owned()),
    }
}

/// True when a `variable_declarator` sits at the top level of a module.
///
/// The chain is `variable_declarator` -> `lexical_declaration` (or
/// `variable_declaration`) -> optional `export_statement` -> `program`. Anything
/// deeper is a local binding inside a function or block.
fn is_module_scope_declarator(node: Node) -> bool {
    let Some(declaration) = node.parent() else {
        return false;
    };
    if !matches!(
        declaration.kind(),
        "lexical_declaration" | "variable_declaration"
    ) {
        return false;
    }
    let Some(container) = declaration.parent() else {
        return false;
    };
    match container.kind() {
        "program" => true,
        "export_statement" => container
            .parent()
            .is_some_and(|outer| outer.kind() == "program"),
        _ => false,
    }
}

/// Classifies a module-level `const`/`let` by the value it binds.
///
/// Arrow functions and function expressions count as functions, so they get
/// callers, callees, and complexity like a declared function.
fn declarator_kind(node: Node) -> SymbolKind {
    let is_function = node.child_by_field_name("value").is_some_and(|value| {
        matches!(
            value.kind(),
            "arrow_function" | "function_expression" | "function"
        )
    });
    if is_function {
        SymbolKind::Fn
    } else {
        SymbolKind::Const
    }
}

pub(super) fn node_to_span(node: Node) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    Span::new(start.row + 1, start.column + 1, end.row + 1, end.column + 1)
}

fn check_exported(node: Node, source_bytes: &[u8], lang: SupportedLanguage) -> bool {
    match lang {
        SupportedLanguage::Rust => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "visibility_modifier" {
                    return true;
                }
            }
            false
        }
        SupportedLanguage::TypeScript | SupportedLanguage::Tsx => {
            // A declaration sits directly under `export_statement`, but a
            // `variable_declarator` is one level deeper: the `export` keyword
            // belongs to the enclosing `lexical_declaration`. Walk both.
            let mut ancestor = node.parent();
            for _ in 0..2 {
                match ancestor {
                    Some(a) if a.kind() == "export_statement" => return true,
                    Some(a)
                        if matches!(a.kind(), "lexical_declaration" | "variable_declaration") =>
                    {
                        ancestor = a.parent();
                    }
                    _ => break,
                }
            }
            let text = node.utf8_text(source_bytes).unwrap_or("");
            text.starts_with("export ")
        }
        _ => false,
    }
}

fn extract_signature(node: Node, source_bytes: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameters" || child.kind() == "formal_parameters" {
            // If signature spans multiple lines or single line, extract from node start to parameters end
            let node_text = node.utf8_text(source_bytes).ok()?;
            if let Some(brace_pos) = node_text.find('{') {
                return Some(node_text[..brace_pos].trim().to_owned());
            } else if let Some(semi_pos) = node_text.find(';') {
                return Some(node_text[..semi_pos].trim().to_owned());
            }
        }
    }
    None
}

fn extract_docstring(node: Node, source_bytes: &[u8], _lang: SupportedLanguage) -> Option<String> {
    let mut prev = node.prev_sibling();
    let mut comments = Vec::new();

    while let Some(sibling) = prev {
        if sibling.kind() == "line_comment" || sibling.kind() == "comment" {
            if let Ok(comment_text) = sibling.utf8_text(source_bytes)
                && (comment_text.starts_with("///")
                    || comment_text.starts_with("/**")
                    || comment_text.starts_with("//"))
            {
                comments.push(comment_text.trim().to_owned());
            }
            prev = sibling.prev_sibling();
        } else {
            break;
        }
    }

    if comments.is_empty() {
        None
    } else {
        comments.reverse();
        Some(comments.join("\n"))
    }
}

fn cyclomatic_complexity(root: Node<'_>, source_bytes: &[u8]) -> u32 {
    let mut complexity = 1u32;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.id() != root.id() {
            if matches!(
                node.kind(),
                "if_expression"
                    | "if_statement"
                    | "match_arm"
                    | "while_expression"
                    | "while_statement"
                    | "for_expression"
                    | "for_statement"
                    | "for_in_statement"
                    | "for_of_statement"
                    | "do_statement"
                    | "switch_case"
                    | "catch_clause"
                    | "ternary_expression"
                    | "conditional_expression"
            ) {
                complexity = complexity.saturating_add(1);
            } else if node.kind() == "binary_expression" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if let Ok(text) = child.utf8_text(source_bytes)
                        && (text == "&&" || text == "||" || text == "??")
                    {
                        complexity = complexity.saturating_add(1);
                        break;
                    }
                }
            }
        }

        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    complexity
}
