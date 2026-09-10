//! AST symbol, call, and import extraction engine using Tree-sitter.
//!
//! Implements a 2-pass extraction pipeline:
//! - Pass 1: Extract definitions (symbols) and record their spans, signatures, docstrings, and visibility.
//! - Pass 2: Extract invocations (calls) and imports, attaching 5-tier confidence heuristic and provenance.

use std::collections::HashSet;
use streaming_iterator::StreamingIterator;
use thiserror::Error;
use tree_sitter::{Node, Query, QueryCursor};

use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::infra::graph::parser::{
    ParserError, SupportedLanguage, TreeSitterEngine, compile_rust_query, compile_tsx_query,
    compile_typescript_query,
};

/// Errors produced during AST extraction.
#[derive(Debug, Error)]
pub enum ExtractorError {
    #[error("Parser error: {0}")]
    Parser(#[from] ParserError),
    #[error("Tree-sitter query error: {0}")]
    Query(#[from] tree_sitter::QueryError),
    #[error("Unsupported language for AST extraction: {0:?}")]
    UnsupportedLanguage(SupportedLanguage),
}

/// The extracted symbols and edges from a single source file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtractedFile {
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
}

/// Extracts symbols, imports, and calls from a source file using Tree-sitter AST queries.
pub fn extract_file(
    repo: &str,
    _file_path: &str,
    content: &str,
    lang: SupportedLanguage,
) -> Result<ExtractedFile, ExtractorError> {
    let mut engine = TreeSitterEngine::new();
    let tree = engine.parse(lang, content)?;
    let root = tree.root_node();

    let query = match lang {
        SupportedLanguage::Rust => compile_rust_query()?,
        SupportedLanguage::TypeScript => compile_typescript_query()?,
        SupportedLanguage::Tsx => compile_tsx_query()?,
        _ => {
            return Ok(ExtractedFile {
                symbols: Vec::new(),
                edges: Vec::new(),
            });
        }
    };

    let source_bytes = content.as_bytes();

    // Pass 1: Extract definitions
    let mut symbols = extract_symbols(repo, root, &query, source_bytes, lang);
    let (route_symbols, http_edges) = extract_http(repo, root, source_bytes, lang);
    symbols.extend(route_symbols);
    let local_symbols: HashSet<String> = symbols.iter().map(|s| s.name.clone()).collect();

    // Pass 2: Extract imports and calls
    let mut edges = extract_edges(repo, root, &query, source_bytes, lang, &local_symbols);
    edges.extend(http_edges);

    Ok(ExtractedFile { symbols, edges })
}

fn extract_symbols(
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

fn is_http_route_method(method: &str) -> Option<&'static str> {
    match method {
        "get" => Some("GET"),
        "post" => Some("POST"),
        "put" => Some("PUT"),
        "patch" => Some("PATCH"),
        "delete" => Some("DELETE"),
        "options" => Some("OPTIONS"),
        "head" => Some("HEAD"),
        _ => None,
    }
}

/// Walks the tree once for HTTP. A route definition such as
/// `fastify.get('/x', handler)` becomes a route symbol; a client call such as
/// `apiRequest('/x', { method: 'POST' })` becomes an edge to `POST /x` that
/// cross-repo linking later points at the route.
fn extract_http(
    repo: &str,
    root: Node,
    source_bytes: &[u8],
    lang: SupportedLanguage,
) -> (Vec<Symbol>, Vec<Edge>) {
    if !matches!(lang, SupportedLanguage::TypeScript | SupportedLanguage::Tsx) {
        return (Vec::new(), Vec::new());
    }

    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    let mut seen_routes = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression" {
            let span = node_to_span(node);
            if let Some(request) = http_client_call(node, source_bytes) {
                edges.push(Edge {
                    id: None,
                    repo: repo.to_owned(),
                    file_id: None,
                    from_symbol_id: None,
                    to_symbol_id: None,
                    to_name: Some(request),
                    kind: EdgeKind::CrossCallsHttp,
                    provenance: Provenance::Inferred,
                    line: span.start_line,
                    col: span.start_col,
                    confidence: 0.8,
                });
            } else if let Some(symbol_name) = http_route_definition(node, source_bytes) {
                let route_key = (symbol_name.clone(), span.start_line, span.start_col);
                if seen_routes.insert(route_key) {
                    symbols.push(Symbol {
                        id: None,
                        file_id: None,
                        repo: repo.to_owned(),
                        name: symbol_name,
                        kind: SymbolKind::Other("route".to_owned()),
                        scope: None,
                        signature: None,
                        docstring: None,
                        span,
                        is_exported: false,
                        complexity: None,
                    });
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    (symbols, edges)
}

fn http_route_definition(node: Node, source_bytes: &[u8]) -> Option<String> {
    let function = node.child_by_field_name("function")?;
    if function.kind() != "member_expression" {
        return None;
    }
    let method_name = function
        .child_by_field_name("property")?
        .utf8_text(source_bytes)
        .ok()?;
    let http_method = is_http_route_method(method_name)?;
    let path_node = node.child_by_field_name("arguments")?.named_child(0)?;
    if path_node.kind() != "string" {
        return None;
    }
    let path = path_node
        .utf8_text(source_bytes)
        .ok()?
        .trim_matches(|c| c == '\'' || c == '"' || c == '`')
        .trim();
    Some(format!("{http_method} {path}"))
}

/// Recognises `fetch('/x')`, `apiRequest('/x', { method: 'POST' })` and
/// `axios.delete(`/users/${id}`)`, returning `METHOD /path` with every template
/// substitution written as `:param`.
fn http_client_call(node: Node, source_bytes: &[u8]) -> Option<String> {
    let function = node.child_by_field_name("function")?;
    let arguments = node.child_by_field_name("arguments")?;
    let method = match function.kind() {
        "identifier" => {
            let name = function.utf8_text(source_bytes).ok()?.to_ascii_lowercase();
            if !(name.contains("fetch") || name.contains("request")) {
                return None;
            }
            arguments
                .named_child(1)
                .and_then(|options| method_option(options, source_bytes))
                .unwrap_or_else(|| "GET".to_owned())
        }
        "member_expression" => {
            let receiver = function
                .child_by_field_name("object")?
                .utf8_text(source_bytes)
                .ok()?
                .rsplit('.')
                .next()?
                .to_ascii_lowercase();
            let is_client = matches!(receiver.as_str(), "axios" | "http" | "ky" | "$http")
                || receiver.ends_with("api")
                || receiver.ends_with("client");
            if !is_client {
                return None;
            }
            let property = function
                .child_by_field_name("property")?
                .utf8_text(source_bytes)
                .ok()?;
            is_http_route_method(property)?.to_owned()
        }
        _ => return None,
    };
    let path = client_path(arguments.named_child(0)?, source_bytes)?;
    Some(format!("{method} {path}"))
}

fn method_option(options: Node, source_bytes: &[u8]) -> Option<String> {
    if options.kind() != "object" {
        return None;
    }
    let unquote = |text: &str| text.trim_matches(|c| c == '\'' || c == '"').to_owned();
    let mut cursor = options.walk();
    let pair = options
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "pair")
        .find(|pair| {
            pair.child_by_field_name("key")
                .and_then(|key| key.utf8_text(source_bytes).ok())
                .is_some_and(|key| unquote(key) == "method")
        })?;
    let value = pair.child_by_field_name("value")?;
    if value.kind() != "string" {
        return None;
    }
    Some(unquote(value.utf8_text(source_bytes).ok()?).to_ascii_uppercase())
}

fn client_path(argument: Node, source_bytes: &[u8]) -> Option<String> {
    if !matches!(argument.kind(), "string" | "template_string") {
        return None;
    }
    let raw = argument
        .utf8_text(source_bytes)
        .ok()?
        .trim_matches(|c| c == '\'' || c == '"' || c == '`');
    let mut path = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some((before, after)) = rest.split_once("${") {
        path.push_str(before);
        path.push_str(":param");
        rest = after.split_once('}').map_or("", |(_, tail)| tail);
    }
    path.push_str(rest);
    let path = path.split(['?', '#']).next().unwrap_or_default();
    path.starts_with('/').then(|| path.to_owned())
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

fn extract_edges(
    repo: &str,
    root: Node,
    query: &Query,
    source_bytes: &[u8],
    lang: SupportedLanguage,
    local_symbols: &HashSet<String>,
) -> Vec<Edge> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source_bytes);

    let call_target_idx = query.capture_index_for_name("call.target");
    let call_receiver_idx = query.capture_index_for_name("call.receiver");
    let import_path_idx = query.capture_index_for_name("import.path");
    let import_source_idx = query.capture_index_for_name("import.source");
    let import_name_idx = query.capture_index_for_name("import.name");
    let type_ref_idx = query.capture_index_for_name("type.ref");

    let mut edges = Vec::new();
    let mut imported_names: HashSet<String> = HashSet::new();

    // First collect imports and build edges + imported_names set
    while let Some(m) = matches.next() {
        for cap in m.captures {
            if Some(cap.index) == import_path_idx || Some(cap.index) == import_source_idx {
                let raw_text = cap
                    .node
                    .utf8_text(source_bytes)
                    .unwrap_or("")
                    .trim_matches(&['"', '\'', ';', ' '][..]);
                let span = node_to_span(cap.node);

                // For Rust use paths (e.g. `foo::bar` or `bar`), record leaf name as imported
                if let Some(leaf) = raw_text.split("::").last()
                    && !leaf.contains('{')
                    && !leaf.contains('*')
                    && !leaf.is_empty()
                {
                    imported_names.insert(leaf.trim().to_owned());
                }

                edges.push(Edge {
                    id: None,
                    repo: repo.to_owned(),
                    file_id: None,
                    from_symbol_id: None,
                    to_symbol_id: None,
                    to_name: Some(raw_text.to_owned()),
                    kind: EdgeKind::Imports,
                    provenance: Provenance::Extracted,
                    line: span.start_line,
                    col: span.start_col,
                    confidence: 0.95,
                });
            } else if Some(cap.index) == import_name_idx {
                let name = cap.node.utf8_text(source_bytes).unwrap_or("").trim();
                if !name.is_empty() {
                    imported_names.insert(name.to_owned());
                    let span = node_to_span(cap.node);
                    edges.push(Edge {
                        id: None,
                        repo: repo.to_owned(),
                        file_id: None,
                        from_symbol_id: None,
                        to_symbol_id: None,
                        to_name: Some(name.to_owned()),
                        kind: EdgeKind::References,
                        provenance: Provenance::Extracted,
                        line: span.start_line,
                        col: span.start_col,
                        confidence: 0.95,
                    });
                }
            }
        }
    }

    // Now collect calls
    let mut cursor2 = QueryCursor::new();
    let mut matches2 = cursor2.matches(query, root, source_bytes);

    while let Some(m) = matches2.next() {
        let mut target_node = None;
        let mut receiver_node = None;

        for cap in m.captures {
            if Some(cap.index) == call_target_idx {
                target_node = Some(cap.node);
            } else if Some(cap.index) == call_receiver_idx {
                receiver_node = Some(cap.node);
            }
        }

        if let Some(t_node) = target_node {
            let target_name = t_node.utf8_text(source_bytes).unwrap_or("").to_owned();
            if target_name.is_empty() {
                continue;
            }

            let span = node_to_span(t_node);
            let receiver_name = receiver_node.and_then(|r| r.utf8_text(source_bytes).ok());

            let (to_name, provenance, confidence) = if local_symbols.contains(&target_name) {
                // Tier 1: Local definition
                (Some(target_name), Provenance::Extracted, 1.0)
            } else if imported_names.contains(&target_name) {
                // Tier 2: Imported symbol
                (Some(target_name), Provenance::Extracted, 0.95)
            } else if let Some(receiver) = receiver_name {
                // Tier 3: Method call with receiver
                (
                    Some(format!("{receiver}.{target_name}")),
                    Provenance::Inferred,
                    0.85,
                )
            } else {
                // Tier 4: General / dynamic / unresolved call
                (Some(target_name), Provenance::Inferred, 0.70)
            };

            edges.push(Edge {
                id: None,
                repo: repo.to_owned(),
                file_id: None,
                from_symbol_id: None,
                to_symbol_id: None,
                to_name,
                kind: EdgeKind::Calls,
                provenance,
                line: span.start_line,
                col: span.start_col,
                confidence,
            });
        }
    }

    if let Some(t_idx) = type_ref_idx {
        let mut cursor3 = QueryCursor::new();
        let mut matches3 = cursor3.matches(query, root, source_bytes);
        let mut seen_type_refs: HashSet<(usize, usize, String)> = HashSet::new();

        while let Some(m) = matches3.next() {
            for cap in m.captures {
                if cap.index == t_idx {
                    let node = cap.node;
                    let parent_kind = node.parent().map(|p| p.kind());

                    if matches!(
                        parent_kind,
                        Some("interface_declaration")
                            | Some("type_alias_declaration")
                            | Some("class_declaration")
                            | Some("enum_declaration")
                    ) && let Some(parent) = node.parent()
                        && let Some(name_node) = parent.child_by_field_name("name")
                        && name_node.id() == node.id()
                    {
                        continue;
                    }

                    if let Ok(raw_name) = node.utf8_text(source_bytes) {
                        let name = raw_name.trim();
                        if !name.is_empty() {
                            let span = node_to_span(node);
                            let key = (span.start_line, span.start_col, name.to_owned());
                            if !seen_type_refs.insert(key) {
                                continue;
                            }

                            edges.push(Edge {
                                id: None,
                                repo: repo.to_owned(),
                                file_id: None,
                                from_symbol_id: None,
                                to_symbol_id: None,
                                to_name: Some(name.to_owned()),
                                kind: EdgeKind::References,
                                provenance: Provenance::Extracted,
                                line: span.start_line,
                                col: span.start_col,
                                confidence: 0.95,
                            });
                        }
                    }
                }
            }
        }
    }

    let hierarchy_edges = extract_hierarchy_edges(repo, root, source_bytes, lang);
    edges.extend(hierarchy_edges);

    edges
}

fn extract_hierarchy_edges(
    repo: &str,
    root: Node,
    source_bytes: &[u8],
    lang: SupportedLanguage,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        match lang {
            SupportedLanguage::Rust => {
                if node.kind() == "impl_item"
                    && let Some(trait_node) = node.child_by_field_name("trait")
                    && let Ok(raw_trait) = trait_node.utf8_text(source_bytes)
                {
                    let trait_name = raw_trait.split('<').next().unwrap_or(raw_trait).trim();
                    if !trait_name.is_empty() {
                        let span = node_to_span(trait_node);
                        edges.push(Edge {
                            id: None,
                            repo: repo.to_owned(),
                            file_id: None,
                            from_symbol_id: None,
                            to_symbol_id: None,
                            to_name: Some(trait_name.to_owned()),
                            kind: EdgeKind::Implements,
                            provenance: Provenance::Extracted,
                            line: span.start_line,
                            col: span.start_col,
                            confidence: 1.0,
                        });
                    }
                }
            }
            SupportedLanguage::TypeScript | SupportedLanguage::Tsx => {
                if node.kind() == "class_declaration" {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "class_heritage" {
                            let mut h_cursor = child.walk();
                            for h_child in child.children(&mut h_cursor) {
                                if h_child.kind() == "extends_clause" {
                                    let mut e_cursor = h_child.walk();
                                    for target in h_child.children(&mut e_cursor) {
                                        if target.kind() != "extends"
                                            && !target.kind().starts_with("comment")
                                            && let Ok(raw_name) = target.utf8_text(source_bytes)
                                        {
                                            let name = raw_name
                                                .split('<')
                                                .next()
                                                .unwrap_or(raw_name)
                                                .trim();
                                            if !name.is_empty() {
                                                let span = node_to_span(target);
                                                edges.push(Edge {
                                                    id: None,
                                                    repo: repo.to_owned(),
                                                    file_id: None,
                                                    from_symbol_id: None,
                                                    to_symbol_id: None,
                                                    to_name: Some(name.to_owned()),
                                                    kind: EdgeKind::Inherits,
                                                    provenance: Provenance::Extracted,
                                                    line: span.start_line,
                                                    col: span.start_col,
                                                    confidence: 1.0,
                                                });
                                            }
                                        }
                                    }
                                } else if h_child.kind() == "implements_clause" {
                                    let mut i_cursor = h_child.walk();
                                    for target in h_child.children(&mut i_cursor) {
                                        if target.kind() != "implements"
                                            && target.kind() != ","
                                            && !target.kind().starts_with("comment")
                                            && let Ok(raw_name) = target.utf8_text(source_bytes)
                                        {
                                            let name = raw_name
                                                .split('<')
                                                .next()
                                                .unwrap_or(raw_name)
                                                .trim();
                                            if !name.is_empty() {
                                                let span = node_to_span(target);
                                                edges.push(Edge {
                                                    id: None,
                                                    repo: repo.to_owned(),
                                                    file_id: None,
                                                    from_symbol_id: None,
                                                    to_symbol_id: None,
                                                    to_name: Some(name.to_owned()),
                                                    kind: EdgeKind::Implements,
                                                    provenance: Provenance::Extracted,
                                                    line: span.start_line,
                                                    col: span.start_col,
                                                    confidence: 1.0,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if node.kind() == "interface_declaration" {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "extends_type_clause"
                            || child.kind() == "extends_clause"
                            || child.kind() == "interface_heritage"
                        {
                            let mut e_cursor = child.walk();
                            for target in child.children(&mut e_cursor) {
                                if target.kind() != "extends"
                                    && target.kind() != ","
                                    && !target.kind().starts_with("comment")
                                    && let Ok(raw_name) = target.utf8_text(source_bytes)
                                {
                                    let name =
                                        raw_name.split('<').next().unwrap_or(raw_name).trim();
                                    if !name.is_empty() {
                                        let span = node_to_span(target);
                                        edges.push(Edge {
                                            id: None,
                                            repo: repo.to_owned(),
                                            file_id: None,
                                            from_symbol_id: None,
                                            to_symbol_id: None,
                                            to_name: Some(name.to_owned()),
                                            kind: EdgeKind::Inherits,
                                            provenance: Provenance::Extracted,
                                            line: span.start_line,
                                            col: span.start_col,
                                            confidence: 1.0,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }

    edges
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

fn node_to_span(node: Node) -> Span {
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

#[cfg(test)]
#[path = "../../../tests/unit/store/graph/extractor.rs"]
mod tests;
