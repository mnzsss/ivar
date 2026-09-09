//! End-to-end integration tests for `ivar graph view` server endpoints,
//! security headers, route filtering, and read-only graph access.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

#[path = "support/graph.rs"]
mod graph_support;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::time::Duration;

use graph_support::GraphHall;
use ivar::action::graph::view::{ViewSeed, ViewerServer};
use ivar::store::graph::db::GraphDb;

#[test]
fn test_graph_viewer_server_endpoints_and_security() {
    let hall = GraphHall::from_current_repo();

    // Verify read-only database opening
    let db = GraphDb::open_read_only(hall.db_path.as_std_path())
        .expect("GraphDb::open_read_only should succeed on indexed hall");

    // Bind server on ephemeral port with default repo seed
    let seed = ViewSeed::Repo(hall.repo_name.clone());
    let server =
        ViewerServer::bind(db, seed).expect("ViewerServer::bind should succeed on ephemeral port");

    let url = server.url();
    assert!(url.starts_with("http://127.0.0.1:"));
    let port: u16 = url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches('/')
        .parse()
        .expect("parse port");

    let shutdown = server.shutdown_handle();

    // Spawn server accept loop on a background thread
    let server_handle = std::thread::spawn(move || {
        let _ = server.serve();
    });

    let addr = format!("127.0.0.1:{port}");

    // Helper to send request and read response
    let exchange = |req: &str| -> String {
        let mut stream = TcpStream::connect_timeout(
            &addr.parse().expect("valid socket addr"),
            Duration::from_secs(5),
        )
        .expect("connect to server");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        stream.write_all(req.as_bytes()).unwrap();
        let _ = stream.flush();

        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        resp
    };

    // Helper to send request and read raw bytes
    let exchange_bytes = |req: &str| -> Vec<u8> {
        let mut stream = TcpStream::connect_timeout(
            &addr.parse().expect("valid socket addr"),
            Duration::from_secs(5),
        )
        .expect("connect to server");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        stream.write_all(req.as_bytes()).unwrap();
        let _ = stream.flush();

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        buf
    };

    // 1. Verify GET / and /index.html return HTML with strict CSP, nosniff, and frame-options DENY
    {
        let req = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        let resp = exchange(&req);
        assert!(
            resp.starts_with("HTTP/1.1 200 OK"),
            "Expected 200 OK, got: {resp}"
        );
        assert!(resp.contains("Content-Type: text/html; charset=utf-8"));
        assert!(resp.contains("Content-Security-Policy: default-src 'self'"));
        assert!(resp.contains("X-Content-Type-Options: nosniff"));
        assert!(resp.contains("X-Frame-Options: DENY"));
        assert!(resp.contains("<!DOCTYPE html>"));
        assert!(resp.contains("<canvas id=\"graph-canvas\""));

        let req_index = format!(
            "GET /index.html HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let resp_index = exchange(&req_index);
        assert!(resp_index.starts_with("HTTP/1.1 200 OK"));
        assert!(resp_index.contains("Content-Type: text/html; charset=utf-8"));
    }

    // 2. Verify GET /app.js and /viewer.js immutable static JS assets
    {
        for path in &["/app.js", "/viewer.js"] {
            let req = format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            );
            let resp = exchange(&req);
            assert!(
                resp.starts_with("HTTP/1.1 200 OK"),
                "Expected 200 OK for {path}"
            );
            assert!(resp.contains("Content-Type: application/javascript; charset=utf-8"));
            assert!(resp.contains("Content-Security-Policy: default-src 'self'"));
            assert!(resp.contains("X-Content-Type-Options: nosniff"));
        }
    }

    // 3. Verify GET /app.css and /viewer.css static CSS assets
    {
        for path in &["/app.css", "/viewer.css"] {
            let req = format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            );
            let resp = exchange(&req);
            assert!(
                resp.starts_with("HTTP/1.1 200 OK"),
                "Expected 200 OK for {path}"
            );
            assert!(resp.contains("Content-Type: text/css; charset=utf-8"));
            assert!(resp.contains("Content-Security-Policy: default-src 'self'"));
            assert!(resp.contains("X-Content-Type-Options: nosniff"));
        }
    }

    // 4. Verify GET /font.woff2 and /fonts/fira-code-400.woff2 binary font assets
    {
        for path in &["/font.woff2", "/fonts/fira-code-400.woff2"] {
            let req = format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            );
            let buf = exchange_bytes(&req);
            let header_end = buf
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .expect("headers present");
            let headers = String::from_utf8_lossy(&buf[..header_end]);
            assert!(
                headers.starts_with("HTTP/1.1 200 OK"),
                "Expected 200 OK for {path}"
            );
            assert!(headers.contains("Content-Type: font/woff2"));
            assert!(
                buf.len() > header_end + 4,
                "Binary font payload must not be empty"
            );
        }
    }

    // 5. Verify GET /api/subgraph and /api/graph return bounded JSON graph
    {
        for path in &["/api/subgraph", "/api/graph"] {
            let req = format!(
                "GET {path}?depth=1&limit=50 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            );
            let resp = exchange(&req);
            assert!(
                resp.starts_with("HTTP/1.1 200 OK"),
                "Expected 200 OK for {path}"
            );
            assert!(resp.contains("Content-Type: application/json"));
            assert!(resp.contains("\"nodes\":"));
            assert!(resp.contains("\"edges\":"));
            assert!(resp.contains("\"truncated\":"));
            assert!(resp.contains("\"depth\":"));
        }
    }
    // 6. Verify GET /api/search returns symbol search results
    {
        let req = format!(
            "GET /api/search?q=GraphDb HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let resp = exchange(&req);
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("Content-Type: application/json"));
        assert!(resp.contains("GraphDb"));
    }

    // 7. Verify GET /api/node returns details for a valid symbol
    {
        // First find a symbol ID via /api/search
        let req = format!(
            "GET /api/search?q=GraphDb HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let resp = exchange(&req);
        let json_start = resp.find("\r\n\r\n").expect("body delimiter") + 4;
        let search_json: serde_json::Value =
            serde_json::from_str(&resp[json_start..]).expect("parse search response");
        let results = search_json.as_array().expect("search results array");
        assert!(!results.is_empty(), "Must find GraphDb symbol");
        let symbol_id = results[0]["symbol"]["id"].as_i64().expect("symbol id");
        let node_req = format!(
            "GET /api/node?id={symbol_id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let node_resp = exchange(&node_req);
        assert!(node_resp.starts_with("HTTP/1.1 200 OK"));
        assert!(node_resp.contains("Content-Type: application/json"));
        assert!(node_resp.contains("\"node\":"));
        assert!(node_resp.contains("\"callers\":"));
        assert!(node_resp.contains("\"callees\":"));

        // Verify expand node
        let expand_req = format!(
            "GET /api/expand?id={symbol_id}&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let expand_resp = exchange(&expand_req);
        assert!(expand_resp.starts_with("HTTP/1.1 200 OK"));
        assert!(expand_resp.contains("Content-Type: application/json"));
        assert!(expand_resp.contains("\"nodes\":"));
        assert!(expand_resp.contains("\"edges\":"));
    }

    // 8. Verify GET /api/path returns shortest path
    {
        let req = format!(
            "GET /api/path?from=GraphDb&to=ViewerServer&max_hops=6 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let resp = exchange(&req);
        assert!(
            resp.starts_with("HTTP/1.1 200 OK"),
            "Expected 200 OK for /api/path, got: {resp}"
        );
        assert!(resp.contains("Content-Type: application/json"));
    }
    // 9. Verify GET /api/impact returns impact blast radius
    {
        let req = format!(
            "GET /api/impact?symbol=GraphDb&max_depth=2 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let resp = exchange(&req);
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"affected_symbols\":"));
    }

    // 10. Verify Security: Foreign Host header rejected with 403 Forbidden
    {
        let req = "GET /api/subgraph HTTP/1.1\r\nHost: evil.com\r\nConnection: close\r\n\r\n";
        let resp = exchange(req);
        assert!(
            resp.starts_with("HTTP/1.1 403 Forbidden"),
            "Foreign Host must be rejected with 403: {resp}"
        );
    }

    // 11. Verify Security: Foreign Origin header rejected with 403 Forbidden
    {
        let req = format!(
            "GET /api/subgraph HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: https://attacker.com\r\nConnection: close\r\n\r\n"
        );
        let resp = exchange(&req);
        assert!(
            resp.starts_with("HTTP/1.1 403 Forbidden"),
            "Foreign Origin must be rejected with 403: {resp}"
        );
    }

    // 12. Verify Security: POST method rejected with 405 Method Not Allowed
    {
        let req = format!(
            "POST /api/subgraph HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let resp = exchange(&req);
        assert!(
            resp.starts_with("HTTP/1.1 405 Method Not Allowed"),
            "POST must return 405: {resp}"
        );
    }

    // 13. Verify Unknown Route returns 404 Not Found
    {
        let req = format!(
            "GET /no-such-route HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let resp = exchange(&req);
        assert!(
            resp.starts_with("HTTP/1.1 404 Not Found"),
            "Unknown route must return 404: {resp}"
        );
    }

    // 14. Clean server shutdown
    shutdown.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect_timeout(
        &addr.parse().expect("valid socket addr"),
        Duration::from_millis(100),
    );
    server_handle.join().expect("server thread joins cleanly");
}
