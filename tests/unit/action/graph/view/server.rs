use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::action::graph::view::error::ViewError;
use crate::action::graph::view::router::{parse_http_request, route_request, validate_security_headers, HttpRequest};
use crate::action::graph::view::server::ViewerServer;
use crate::action::graph::view::types::ViewSeed;
use crate::store::graph::db::GraphDb;

#[test]
fn parse_http_request_extracts_method_path_query_and_headers() {
    let raw = b"GET /api/subgraph?depth=2&limit=300 HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nUser-Agent: Test\r\n\r\n";
    let req = parse_http_request(raw).expect("valid request parses");
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/api/subgraph");
    assert_eq!(req.query_string.as_deref(), Some("depth=2&limit=300"));
    assert_eq!(req.headers.get("host").map(String::as_str), Some("127.0.0.1:8080"));
}

#[test]
fn parse_http_request_rejects_oversized_payload() {
    let mut large = b"GET /api/subgraph HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nX-Padding: ".to_vec();
    large.resize(8192 + 100, b'A');
    large.extend_from_slice(b"\r\n\r\n");
    let res = parse_http_request(&large);
    assert!(matches!(res, Err(ViewError::InvalidRequest(_))));
}

#[test]
fn security_validator_accepts_valid_loopback_host_and_same_origin() {
    let bound: std::net::SocketAddr = "127.0.0.1:45678".parse().unwrap();
    let mut headers = HashMap::new();
    headers.insert("host".into(), "127.0.0.1:45678".into());
    headers.insert("origin".into(), "http://127.0.0.1:45678".into());

    let req = HttpRequest {
        method: "GET".into(),
        path: "/api/subgraph".into(),
        query_string: None,
        headers,
    };

    assert!(validate_security_headers(&req, &bound).is_ok());
}

#[test]
fn security_validator_rejects_wrong_host_and_foreign_origin() {
    let bound: std::net::SocketAddr = "127.0.0.1:45678".parse().unwrap();

    // Wrong Host (DNS rebinding attempt)
    let mut bad_host = HashMap::new();
    bad_host.insert("host".into(), "evil.com:45678".into());
    let req_bad_host = HttpRequest {
        method: "GET".into(),
        path: "/api/subgraph".into(),
        query_string: None,
        headers: bad_host,
    };
    assert!(matches!(
        validate_security_headers(&req_bad_host, &bound),
        Err(ViewError::Security(_))
    ));

    // Foreign Origin
    let mut foreign_origin = HashMap::new();
    foreign_origin.insert("host".into(), "127.0.0.1:45678".into());
    foreign_origin.insert("origin".into(), "http://attacker.com".into());
    let req_foreign_origin = HttpRequest {
        method: "GET".into(),
        path: "/api/subgraph".into(),
        query_string: None,
        headers: foreign_origin,
    };
    assert!(matches!(
        validate_security_headers(&req_foreign_origin, &bound),
        Err(ViewError::Security(_))
    ));
}

#[test]
fn route_request_rejects_non_get_method_with_405() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("graph.db");
    let db = GraphDb::open(&db_path).unwrap();

    let req = HttpRequest {
        method: "POST".into(),
        path: "/api/subgraph".into(),
        query_string: None,
        headers: HashMap::new(),
    };

    let resp = route_request(&req, &db, &ViewSeed::Default);
    assert_eq!(resp.status_code, 405);
}

#[test]
fn route_request_returns_security_headers_and_not_found_on_unknown_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("graph.db");
    let db = GraphDb::open(&db_path).unwrap();

    let req = HttpRequest {
        method: "GET".into(),
        path: "/unknown/secret.txt".into(),
        query_string: None,
        headers: HashMap::new(),
    };

    let resp = route_request(&req, &db, &ViewSeed::Default);
    assert_eq!(resp.status_code, 404);
    let has_csp = resp
        .extra_headers
        .iter()
        .any(|(k, v)| *k == "Content-Security-Policy" && v.contains("default-src 'self'"));
    let has_nosniff = resp
        .extra_headers
        .iter()
        .any(|(k, v)| *k == "X-Content-Type-Options" && *v == "nosniff");
    assert!(has_csp);
    assert!(has_nosniff);
}

#[test]
fn server_binds_to_ephemeral_loopback_and_serves_valid_api_response() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("graph.db");
    let db = GraphDb::open(&db_path).unwrap();

    let server = ViewerServer::bind_loopback(db, ViewSeed::Default, "127.0.0.1:0")
        .expect("ephemeral port binds successfully");
    let addr = server.addr();
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    assert!(addr.port() > 0);
    assert_eq!(server.url(), format!("http://127.0.0.1:{}", addr.port()));

    let shutdown = server.shutdown_handle();
    let handle = std::thread::spawn(move || {
        server.serve().unwrap();
    });

    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("connects to server");
    let request_data = format!(
        "GET /api/subgraph HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\n\r\n",
        addr, addr
    );
    stream.write_all(request_data.as_bytes()).unwrap();

    let mut response_buf = Vec::new();
    let mut temp = [0u8; 1024];
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    if let Ok(n) = stream.read(&mut temp) {
        response_buf.extend_from_slice(&temp[..n]);
    }
    let response_text = String::from_utf8_lossy(&response_buf);
    assert!(response_text.starts_with("HTTP/1.1 200 OK"));
    assert!(response_text.contains("Content-Type: application/json"));
    assert!(response_text.contains("Content-Security-Policy:"));

    // Also test `/` and `/app.js` routes
    let mut stream_root = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("connects to server");
    let req_root = format!("GET / HTTP/1.1\r\nHost: {}\r\n\r\n", addr);
    stream_root.write_all(req_root.as_bytes()).unwrap();
    let mut root_buf = Vec::new();
    stream_root.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    if let Ok(n) = stream_root.read(&mut temp) {
        root_buf.extend_from_slice(&temp[..n]);
    }
    let root_text = String::from_utf8_lossy(&root_buf);
    assert!(root_text.starts_with("HTTP/1.1 200 OK"));
    assert!(root_text.contains("Content-Type: text/html"));

    // Signal shutdown and join server thread
    shutdown.store(true, Ordering::SeqCst);
    // Connect once to unblock accept loop
    let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(100));
    handle.join().unwrap();
}
