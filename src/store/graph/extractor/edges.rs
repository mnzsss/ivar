use std::collections::HashSet;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor};

use super::symbols::node_to_span;
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span};
use crate::infra::graph::parser::SupportedLanguage;

pub(super) fn extract_edges(
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

    let mut import_edges = Vec::new();
    let mut imported_names: HashSet<String> = HashSet::new();
    let mut call_sites: Vec<(String, Span, Option<&str>)> = Vec::new();
    let mut type_ref_edges = Vec::new();
    let mut seen_type_refs: HashSet<(usize, usize, String)> = HashSet::new();

    // One pass over the matches collects every capture. Calls are resolved after
    // it, once every import in the file is known.
    while let Some(m) = matches.next() {
        let mut target_node = None;
        let mut receiver_node = None;

        for cap in m.captures {
            let index = Some(cap.index);
            if index == import_path_idx || index == import_source_idx {
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

                import_edges.push(Edge {
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
            } else if index == import_name_idx {
                let name = cap.node.utf8_text(source_bytes).unwrap_or("").trim();
                if !name.is_empty() {
                    imported_names.insert(name.to_owned());
                    let span = node_to_span(cap.node);
                    import_edges.push(Edge {
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
            } else if index == call_target_idx {
                target_node = Some(cap.node);
            } else if index == call_receiver_idx {
                receiver_node = Some(cap.node);
            } else if index == type_ref_idx {
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

                        type_ref_edges.push(Edge {
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

        if let Some(t_node) = target_node
            && let Ok(target_name) = t_node.utf8_text(source_bytes)
            && !target_name.is_empty()
        {
            let receiver_name = receiver_node.and_then(|r| r.utf8_text(source_bytes).ok());
            call_sites.push((target_name.to_owned(), node_to_span(t_node), receiver_name));
        }
    }

    let call_edges = call_sites
        .into_iter()
        .map(|(target_name, span, receiver_name)| {
            let (to_name, provenance, confidence) = if local_symbols.contains(&target_name) {
                // Tier 1: Local definition
                (target_name, Provenance::Extracted, 1.0)
            } else if imported_names.contains(&target_name) {
                // Tier 2: Imported symbol
                (target_name, Provenance::Extracted, 0.95)
            } else if let Some(receiver) = receiver_name {
                // Tier 3: Method call with receiver
                (
                    format!("{receiver}.{target_name}"),
                    Provenance::Inferred,
                    0.85,
                )
            } else {
                // Tier 4: General / dynamic / unresolved call
                (target_name, Provenance::Inferred, 0.70)
            };
            Edge {
                id: None,
                repo: repo.to_owned(),
                file_id: None,
                from_symbol_id: None,
                to_symbol_id: None,
                to_name: Some(to_name),
                kind: EdgeKind::Calls,
                provenance,
                line: span.start_line,
                col: span.start_col,
                confidence,
            }
        });

    let mut edges = import_edges;
    edges.extend(call_edges);
    edges.extend(type_ref_edges);
    edges.extend(extract_hierarchy_edges(repo, root, source_bytes, lang));
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
