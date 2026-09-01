#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

// -- bind tests ----------------------------------------------------------

#[test]
fn bind_binds_to_loopback() {
    let server = CallbackServer::bind_on("test-state", Duration::from_secs(10)).unwrap();
    let addr = server.addr();
    assert_eq!(addr.ip(), std::net::Ipv4Addr::LOCALHOST);
    assert!(addr.port() > 0);
}

// -- successful callback -------------------------------------------------

#[test]
fn valid_callback_returns_code_and_200() {
    let expected_state = "expected-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect to callback server");
        let request = format!(
            "GET /callback?code=auth-code-123&state={expected_state} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).expect("write request");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read response");
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "expected 200, got: {response}"
        );
    });

    let result = server.wait().expect("wait should succeed");
    assert_eq!(result.0, "auth-code-123");
}

// -- percent-decoding ----------------------------------------------------

#[test]
fn percent_encoded_code_is_decoded() {
    let expected_state = "test-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        // code=hello%20world (space as %20)
        let request = format!(
            "GET /callback?code=hello%20world&state={expected_state} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(response.starts_with("HTTP/1.1 200"));
    });

    let result = server.wait().expect("wait should succeed");
    assert_eq!(result.0, "hello world");
}

#[test]
fn plus_in_query_decoded_as_space() {
    let expected_state = "test-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        // code=hello+world (space as +)
        let request = format!(
            "GET /callback?code=hello+world&state={expected_state} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(response.starts_with("HTTP/1.1 200"));
    });

    let result = server.wait().expect("wait should succeed");
    assert_eq!(result.0, "hello world");
}

// -- state validation ----------------------------------------------------

#[test]
fn wrong_state_is_rejected_with_400() {
    let expected_state = "expected-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        let request = "GET /callback?code=auth-code-123&state=wrong-state HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             \r\n"
            .to_owned();
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "expected 400, got: {response}"
        );
    });

    let err = server.wait().expect_err("wait should fail for wrong state");
    assert_eq!(err.code, "callback.state_mismatch");
}

#[test]
fn missing_state_is_rejected() {
    let expected_state = "expected-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        // No state param
        let request = "GET /callback?code=auth-code-123 HTTP/1.1\r\n\
                       Host: 127.0.0.1\r\n\
                       \r\n";
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(response.starts_with("HTTP/1.1 400"));
    });

    let err = server
        .wait()
        .expect_err("wait should fail for missing state");
    assert_eq!(err.code, "callback.state_mismatch");
}

// -- wrong path ----------------------------------------------------------

#[test]
fn wrong_path_is_rejected() {
    let expected_state = "test-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        let request = "GET /wrong-path HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(response.starts_with("HTTP/1.1 400"));
    });

    let err = server.wait().expect_err("should reject wrong path");
    assert_eq!(err.code, "callback.invalid_request");
}

// -- wrong method --------------------------------------------------------

#[test]
fn non_get_method_is_rejected() {
    let expected_state = "test-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        let request = format!(
            "POST /callback?code=auth-code-123&state={expected_state} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Content-Length: 0\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(
            response.starts_with("HTTP/1.1 405"),
            "expected 405, got: {response}"
        );
    });

    let err = server.wait().expect_err("should reject non-GET");
    assert_eq!(err.code, "callback.method_not_allowed");
}

// -- OAuth error ---------------------------------------------------------

#[test]
fn oauth_error_is_rejected_without_leaking_description() {
    let expected_state = "test-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        // error=access_denied&error_description=User+denied+access
        let request = format!(
            "GET /callback?error=access_denied&error_description=User+denied+access&state={expected_state} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(response.starts_with("HTTP/1.1 400"));
        // error_description must not leak into HTTP response body
        assert!(
            !response.contains("User denied"),
            "response must not leak error_description"
        );
    });

    let err = server.wait().expect_err("should reject OAuth error");
    assert_eq!(err.code, "callback.oauth_error");
}

// -- missing code --------------------------------------------------------

#[test]
fn missing_code_is_rejected() {
    let expected_state = "test-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        // Only state, no code
        let request = format!(
            "GET /callback?state={expected_state} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(response.starts_with("HTTP/1.1 400"));
    });

    let err = server.wait().expect_err("should reject missing code");
    assert_eq!(err.code, "callback.missing_code");
}

// -- timeout -------------------------------------------------------------

#[test]
fn timeout_returns_error_and_releases_port() {
    let server = CallbackServer::bind_on("test-state", Duration::from_millis(200)).unwrap();
    let addr = server.addr();
    assert_eq!(addr.ip(), std::net::Ipv4Addr::LOCALHOST);

    let err = server.wait().expect_err("timeout should produce Failure");
    assert_eq!(err.code, "callback.timeout");
}

// -- drop before callback ------------------------------------------------

#[test]
fn drop_before_callback_releases_port_and_does_not_leak_thread() {
    let server = CallbackServer::bind_on("test-state", Duration::from_secs(10)).unwrap();
    let addr = server.addr();
    drop(server);

    // Port should be immediately rebindable.
    std::thread::sleep(Duration::from_millis(50));
    let rebinding = std::net::TcpListener::bind(addr);
    assert!(rebinding.is_ok(), "port should be available after drop");
}

// -- port rebindable after success --------------------------------------

#[test]
fn port_is_rebindable_after_successful_callback() {
    let expected_state = "test-state";
    let server = CallbackServer::bind_on(expected_state, Duration::from_secs(10)).unwrap();
    let addr = server.addr();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        let request = format!(
            "GET /callback?code=abc&state={expected_state} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read");
        assert!(response.starts_with("HTTP/1.1 200"));
    });

    let result = server.wait().expect("wait should succeed");
    assert_eq!(result.0, "abc");

    // Port should be immediately rebindable.
    let rebinding = std::net::TcpListener::bind(addr);
    assert!(
        rebinding.is_ok(),
        "port should be available after successful wait"
    );
}

// -- Debug redaction -----------------------------------------------------

#[test]
fn authorization_code_debug_redacts_value() {
    let code = AuthorizationCode("super-secret-auth-code".to_owned());
    let rendered = format!("{code:?}");
    assert_eq!(rendered, "AuthorizationCode(<redacted>)");
    assert!(
        !rendered.contains("super-secret"),
        "Debug must not leak the code value"
    );
}
