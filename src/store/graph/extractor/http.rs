use std::collections::HashSet;

use tree_sitter::Node;

use super::symbols::node_to_span;
use crate::domain::graph::{Edge, EdgeKind, Provenance, Symbol, SymbolKind};
use crate::infra::graph::parser::SupportedLanguage;

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
pub(super) fn extract_http(
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

/// The TypeScript grammar parses `await apiRequest<User>('/me')` with the `await`
/// inside the callee, as `call_expression(function: await_expression(identifier))`.
fn callee(call: Node) -> Option<Node> {
    let function = call.child_by_field_name("function")?;
    if function.kind() == "await_expression" {
        function.named_child(0)
    } else {
        Some(function)
    }
}

fn http_route_definition(node: Node, source_bytes: &[u8]) -> Option<String> {
    let function = callee(node)?;
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
    let function = callee(node)?;
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
