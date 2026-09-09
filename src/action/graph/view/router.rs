use std::collections::HashMap;
use std::net::SocketAddr;

use super::error::ViewError;
use super::query::{
    collect_subgraph, expand_node, get_node_details, query_impact, query_path, search_symbols,
};
use super::types::ViewSeed;
use crate::store::graph::db::GraphDb;

const MAX_HEADER_SIZE: usize = 8192;

const CSP_HEADER: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none';";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query_string: Option<String>,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status_code: u16,
    pub reason: &'static str,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub extra_headers: Vec<(&'static str, &'static str)>,
}

impl HttpResponse {
    pub fn new(
        status_code: u16,
        reason: &'static str,
        content_type: &'static str,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status_code,
            reason,
            content_type,
            body,
            extra_headers: vec![
                ("Content-Security-Policy", CSP_HEADER),
                ("X-Content-Type-Options", "nosniff"),
                ("X-Frame-Options", "DENY"),
                ("Cache-Control", "no-cache, no-store, must-revalidate"),
            ],
        }
    }

    pub fn ok_json(body: Vec<u8>) -> Self {
        Self::new(200, "OK", "application/json", body)
    }

    pub fn ok_html(body: Vec<u8>) -> Self {
        Self::new(200, "OK", "text/html; charset=utf-8", body)
    }

    pub fn ok_css(body: Vec<u8>) -> Self {
        Self::new(200, "OK", "text/css; charset=utf-8", body)
    }

    pub fn ok_js(body: Vec<u8>) -> Self {
        Self::new(200, "OK", "application/javascript; charset=utf-8", body)
    }

    pub fn ok_font(body: Vec<u8>) -> Self {
        Self::new(200, "OK", "font/woff2", body)
    }

    pub fn not_found() -> Self {
        Self::new(
            404,
            "Not Found",
            "text/html; charset=utf-8",
            b"<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>404 Not Found</title></head><body><main><h1>404 Not Found</h1><p>The requested resource does not exist.</p><p><a href=\"/\">Return to Graph Viewer</a></p></main></body></html>".to_vec(),
        )
    }

    pub fn method_not_allowed() -> Self {
        Self::new(
            405,
            "Method Not Allowed",
            "text/plain; charset=utf-8",
            b"Method Not Allowed".to_vec(),
        )
    }

    pub fn bad_request(msg: &str) -> Self {
        Self::new(
            400,
            "Bad Request",
            "text/plain; charset=utf-8",
            msg.as_bytes().to_vec(),
        )
    }

    pub fn forbidden(msg: &str) -> Self {
        Self::new(
            403,
            "Forbidden",
            "text/plain; charset=utf-8",
            msg.as_bytes().to_vec(),
        )
    }

    pub fn internal_error(msg: &str) -> Self {
        Self::new(
            500,
            "Internal Server Error",
            "text/plain; charset=utf-8",
            msg.as_bytes().to_vec(),
        )
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(
            format!(
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                self.status_code,
                self.reason,
                self.content_type,
                self.body.len()
            )
            .as_bytes(),
        );

        for (k, v) in &self.extra_headers {
            out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
        }

        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.body);
        out
    }
}

pub fn parse_http_request(buf: &[u8]) -> Result<HttpRequest, ViewError> {
    if buf.len() > MAX_HEADER_SIZE {
        return Err(ViewError::InvalidRequest(
            "request exceeds 8 KiB limit".into(),
        ));
    }

    let text = std::str::from_utf8(buf)
        .map_err(|_| ViewError::InvalidRequest("non-utf8 request header".into()))?;

    let mut lines = text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| ViewError::InvalidRequest("empty request".into()))?;

    let mut req_parts = request_line.split_whitespace();
    let method = req_parts
        .next()
        .ok_or_else(|| ViewError::InvalidRequest("missing method".into()))?
        .to_owned();
    let raw_path = req_parts
        .next()
        .ok_or_else(|| ViewError::InvalidRequest("missing path".into()))?;

    let (path, query_string) = match raw_path.split_once('?') {
        Some((p, q)) => (p.to_owned(), Some(q.to_owned())),
        None => (raw_path.to_owned(), None),
    };

    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, val)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), val.trim().to_owned());
        }
    }

    Ok(HttpRequest {
        method,
        path,
        query_string,
        headers,
    })
}

pub fn validate_security_headers(
    req: &HttpRequest,
    bound_addr: &SocketAddr,
) -> Result<(), ViewError> {
    let port = bound_addr.port();
    let valid_host_1 = format!("127.0.0.1:{port}");
    let valid_host_2 = format!("localhost:{port}");

    // Validate Host
    if let Some(host) = req.headers.get("host") {
        if host != &valid_host_1 && host != &valid_host_2 {
            return Err(ViewError::Security(format!("invalid host header: {host}")));
        }
    } else {
        return Err(ViewError::Security("missing host header".into()));
    }

    // Validate Origin if present
    if let Some(origin) = req.headers.get("origin") {
        let valid_origin_1 = format!("http://127.0.0.1:{port}");
        let valid_origin_2 = format!("http://localhost:{port}");
        if origin != &valid_origin_1 && origin != &valid_origin_2 {
            return Err(ViewError::Security(format!(
                "invalid origin header: {origin}"
            )));
        }
    }

    // Validate Sec-Fetch-Site if present
    if let Some(sec_fetch_site) = req.headers.get("sec-fetch-site")
        && sec_fetch_site != "same-origin"
        && sec_fetch_site != "none"
    {
        return Err(ViewError::Security(format!(
            "invalid sec-fetch-site: {sec_fetch_site}"
        )));
    }

    Ok(())
}

fn parse_query_params(query: Option<&str>) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                // simple percent decoding
                let decoded_val = percent_decode(v);
                let decoded_key = percent_decode(k);
                params.insert(decoded_key, decoded_val);
            }
        }
    }
    params
}

fn percent_decode(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2)
                && let Ok(val) = u8::from_str_radix(&format!("{}{}", h1 as char, h2 as char), 16)
            {
                bytes.push(val);
                continue;
            }
        } else if b == b'+' {
            bytes.push(b' ');
            continue;
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).to_string()
}

pub fn route_request(req: &HttpRequest, db: &GraphDb, seed: &ViewSeed) -> HttpResponse {
    if req.method != "GET" {
        return HttpResponse::method_not_allowed();
    }

    let params = parse_query_params(req.query_string.as_deref());

    match req.path.as_str() {
        "/" | "/index.html" => HttpResponse::ok_html(super::assets::INDEX_HTML.as_bytes().to_vec()),
        "/robots.txt" => HttpResponse::new(
            200,
            "OK",
            "text/plain; charset=utf-8",
            b"User-agent: *\nDisallow: /\n".to_vec(),
        ),
        "/favicon.svg" | "/favicon.ico" => HttpResponse::new(
            200,
            "OK",
            "image/svg+xml",
            br##"<svg viewBox="0 0 32 32"><circle cx="16" cy="16" r="14" fill="#151515" stroke="#f2ebdd" stroke-width="2"/><circle cx="10" cy="12" r="3" fill="#00d8b4"/><circle cx="22" cy="12" r="3" fill="#ff79c6"/><circle cx="16" cy="22" r="3" fill="#bd93f9"/><line x1="10" y1="12" x2="16" y2="22" stroke="#6272a4" stroke-width="1.5"/><line x1="22" y1="12" x2="16" y2="22" stroke="#6272a4" stroke-width="1.5"/></svg>"##.to_vec(),
        ),
        "/app.js" | "/viewer.js" => {
            HttpResponse::ok_js(super::assets::VIEWER_JS.as_bytes().to_vec())
        }
        "/cytoscape.js" | "/cytoscape.min.js" => {
            HttpResponse::ok_js(super::assets::CYTOSCAPE_JS.as_bytes().to_vec())
        }
        "/app.css" | "/viewer.css" => {
            HttpResponse::ok_css(super::assets::VIEWER_CSS.as_bytes().to_vec())
        }
        "/font.woff2" | "/fonts/fira-code-400.woff2" => {
            HttpResponse::ok_font(super::assets::FONT_WOFF2.to_vec())
        }
        "/api/subgraph" | "/api/graph" => {
            let depth = params
                .get("depth")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1);
            let limit = params
                .get("limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(400);

            // If seed params are in query, prefer them; otherwise fallback to server seed
            let active_seed = if let Some(repo) = params.get("repo") {
                ViewSeed::Repo(repo.clone())
            } else if let Some(sym) = params.get("symbol") {
                ViewSeed::Symbol(sym.clone())
            } else if let Some(file) = params.get("file") {
                ViewSeed::File(file.clone())
            } else if let Some(impact) = params.get("impact") {
                ViewSeed::Impact(impact.clone())
            } else {
                seed.clone()
            };

            match collect_subgraph(db, &active_seed, depth, limit) {
                Ok(graph) => match serde_json::to_vec(&graph) {
                    Ok(json) => HttpResponse::ok_json(json),
                    Err(e) => HttpResponse::internal_error(&e.to_string()),
                },
                Err(ViewError::SeedNotFound(msg)) => HttpResponse::bad_request(&msg),
                Err(ViewError::InvalidParam(msg)) => HttpResponse::bad_request(&msg),
                Err(err) => HttpResponse::internal_error(&err.to_string()),
            }
        }
        "/api/node" => {
            let symbol_id = match params.get("id").and_then(|v| v.parse::<i64>().ok()) {
                Some(id) => id,
                None => return HttpResponse::bad_request("missing or invalid 'id' parameter"),
            };
            match get_node_details(db, symbol_id) {
                Ok(details) => match serde_json::to_vec(&details) {
                    Ok(json) => HttpResponse::ok_json(json),
                    Err(e) => HttpResponse::internal_error(&e.to_string()),
                },
                Err(ViewError::NotFound) => HttpResponse::not_found(),
                Err(err) => HttpResponse::internal_error(&err.to_string()),
            }
        }
        "/api/expand" => {
            let symbol_id = match params.get("id").and_then(|v| v.parse::<i64>().ok()) {
                Some(id) => id,
                None => return HttpResponse::bad_request("missing or invalid 'id' parameter"),
            };
            let limit = params
                .get("limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(50);
            match expand_node(db, symbol_id, limit) {
                Ok(graph) => match serde_json::to_vec(&graph) {
                    Ok(json) => HttpResponse::ok_json(json),
                    Err(e) => HttpResponse::internal_error(&e.to_string()),
                },
                Err(err) => HttpResponse::internal_error(&err.to_string()),
            }
        }
        "/api/search" => {
            let query = match params.get("q") {
                Some(q) if !q.is_empty() => q,
                _ => return HttpResponse::bad_request("missing or empty 'q' parameter"),
            };
            let repo = params.get("repo").map(|s| s.as_str());
            let limit = params
                .get("limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(50);
            match search_symbols(db, query, repo, limit) {
                Ok(results) => match serde_json::to_vec(&results) {
                    Ok(json) => HttpResponse::ok_json(json),
                    Err(e) => HttpResponse::internal_error(&e.to_string()),
                },
                Err(err) => HttpResponse::internal_error(&err.to_string()),
            }
        }
        "/api/path" => {
            let from = match params.get("from") {
                Some(f) if !f.is_empty() => f,
                _ => return HttpResponse::bad_request("missing or empty 'from' parameter"),
            };
            let to = match params.get("to") {
                Some(t) if !t.is_empty() => t,
                _ => return HttpResponse::bad_request("missing or empty 'to' parameter"),
            };
            let max_hops = params
                .get("max_hops")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(6);
            match query_path(db, from, to, max_hops) {
                Ok(path) => match serde_json::to_vec(&path) {
                    Ok(json) => HttpResponse::ok_json(json),
                    Err(e) => HttpResponse::internal_error(&e.to_string()),
                },
                Err(err) => HttpResponse::internal_error(&err.to_string()),
            }
        }
        "/api/impact" => {
            let symbol = match params.get("symbol") {
                Some(s) if !s.is_empty() => s,
                _ => return HttpResponse::bad_request("missing or empty 'symbol' parameter"),
            };
            let repo = params.get("repo").map(|s| s.as_str());
            let max_depth = params
                .get("max_depth")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(3);
            match query_impact(db, symbol, repo, max_depth) {
                Ok(impact) => match serde_json::to_vec(&impact) {
                    Ok(json) => HttpResponse::ok_json(json),
                    Err(e) => HttpResponse::internal_error(&e.to_string()),
                },
                Err(err) => HttpResponse::internal_error(&err.to_string()),
            }
        }
        _ => HttpResponse::not_found(),
    }
}
