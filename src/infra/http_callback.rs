//! Temporary loopback HTTP listener for the OAuth callback (`R-CALLBACK`).
//!
//! Figma's authorization-code flow redirects the browser to a local URL
//! carrying the `code` and `state` query parameters. This module owns that
//! transient listener: it binds `127.0.0.1:19876`, accepts exactly one
//! `GET /callback`, validates `state`, and yields the authorization code.
//!
//! # Design
//!
//! - No async runtime, no HTTP framework — `std::net::TcpListener` on a
//!   worker thread.
//! - Bounded input (8 KiB read limit) to reject oversized requests.
//! - Accepted sockets are restored to blocking mode with a read/write timeout
//!   to prevent slow or hostile clients from pinning the worker thread.
//! - `Drop` signals the worker to stop and closes the listener so the
//!   port is released immediately.
//! - `wait()` consumes `self`, so the caller cannot accidentally use the
//!   server after the one-shot flow completes.
//!
//! # Secrets
//!
//! `AuthorizationCode` redacts its value in `Debug`. The HTTP response body
//! never contains the authorization code, state, or PKCE verifier.
//! Failures never carry these values in their `actual` field.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::Failure;

/// Default loopback address for the OAuth callback.
const LOCALHOST: &str = "127.0.0.1";

/// Default port for the OAuth callback listener.
const DEFAULT_PORT: u16 = 19876;

/// The redirect URI the listener above answers on — the one address a
/// provider config or a client registration must name for the exchange to
/// come back here.
///
/// The path (`/callback`) is the remote host's requirement, not OpenCode's
/// own default (`/mcp/oauth/callback`) — Figma's registration endpoint
/// returns `400 invalid_redirect_uri` for anything else (measured
/// 2026-08-26; see `plans/ivar-mcp-auth/analysis.md`). The port is
/// [`DEFAULT_PORT`], which is also OpenCode's own default callback port;
/// setting `oauth.redirectUri` overrides both OpenCode's callback server and
/// its authorize request, so one shared constant is what reconciles them
/// rather than leaving three copies to disagree.
pub const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:19876/callback";

/// Maximum bytes to read from a single HTTP request (request line + headers).
const MAX_READ: usize = 8192;

/// Timeout for reading/writing on the callback socket.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// An authorization code yielded by the callback listener. The value is
/// redacted from `Debug` to prevent leaks.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationCode(pub String);

impl std::fmt::Debug for AuthorizationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizationCode(<redacted>)")
    }
}

/// A one-shot loopback HTTP listener for the OAuth callback.
///
/// Bind it, print [`authorization_callback_url`](Self::authorization_callback_url)
/// for the user, then [`wait`](Self::wait) for the result.
pub struct CallbackServer {
    listener: Option<TcpListener>,
    shutdown: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<AuthorizationCode, Failure>>,
    worker: Option<thread::JoinHandle<()>>,
    addr: std::net::SocketAddr,
}

impl std::fmt::Debug for CallbackServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackServer")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl CallbackServer {
    /// Bind to `127.0.0.1:19876` — the production callback address.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] if the port is already in use.
    pub fn bind(expected_state: &str, timeout: Duration) -> Result<Self, Failure> {
        let addr = format!("{LOCALHOST}:{DEFAULT_PORT}");
        Self::bind_inner(expected_state, timeout, &addr)
    }

    /// Bind to an arbitrary address (e.g. `127.0.0.1:0`) — for tests that
    /// need the OS to pick an available port.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] if the address cannot be bound.
    pub fn bind_on(expected_state: &str, timeout: Duration) -> Result<Self, Failure> {
        let addr = format!("{LOCALHOST}:0");
        Self::bind_inner(expected_state, timeout, &addr)
    }

    fn bind_inner(expected_state: &str, timeout: Duration, addr: &str) -> Result<Self, Failure> {
        let listener = TcpListener::bind(addr).map_err(|e| {
            Failure::blocked(
                "callback.bind_failed",
                format!("could not bind callback listener on {addr}: {e}"),
            )
            .expected("the loopback port to be available")
            .actual(format!("{e}"))
        })?;
        let socket_addr = listener.local_addr().map_err(|e| {
            Failure::failed(
                "callback.addr_failed",
                format!("could not get listener address: {e}"),
            )
        })?;
        let expected = expected_state.to_owned();

        let shutdown = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        // Switch to non-blocking before spawning so the worker never hangs
        // on accept when Drop closes the listener.
        listener.set_nonblocking(true).map_err(|e| {
            Failure::failed(
                "callback.nonblocking_failed",
                format!("could not set non-blocking mode: {e}"),
            )
        })?;

        let worker_shutdown = Arc::clone(&shutdown);
        let listener_for_worker = listener.try_clone().map_err(|e| {
            Failure::failed(
                "callback.clone_failed",
                format!("could not clone listener: {e}"),
            )
        })?;

        let worker = thread::spawn(move || {
            Self::worker_loop(listener_for_worker, tx, worker_shutdown, expected, timeout);
        });

        Ok(Self {
            listener: Some(listener),
            shutdown,
            receiver: rx,
            worker: Some(worker),
            addr: socket_addr,
        })
    }

    /// The bound socket address (useful to know which port was assigned).
    #[must_use]
    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    /// The full authorization callback URL to print for the user.
    #[must_use]
    pub fn authorization_callback_url(&self) -> String {
        let addr = self.addr();
        format!("http://{addr}/callback")
    }

    /// Block until the callback arrives or the timeout expires.
    ///
    /// Consumes `self` — the server is single-use by design.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] on timeout, invalid request, state mismatch,
    /// missing code, or OAuth error.
    pub fn wait(mut self) -> Result<AuthorizationCode, Failure> {
        let result = match self.receiver.recv() {
            Ok(result) => result,
            Err(_) => Err(Failure::failed(
                "callback.worker_exited",
                "callback worker exited before sending a result",
            )),
        };

        // Ensure the worker has fully finished and the listener is closed.
        drop(self.listener.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }

        result
    }

    // -- worker -----------------------------------------------------------

    fn worker_loop(
        listener: TcpListener,
        tx: mpsc::Sender<Result<AuthorizationCode, Failure>>,
        shutdown: Arc<AtomicBool>,
        expected_state: String,
        timeout: Duration,
    ) {
        let deadline = Instant::now() + timeout;

        loop {
            if Instant::now() >= deadline || shutdown.load(Ordering::Acquire) {
                let _ = tx.send(Err(Failure::failed(
                    "callback.timeout",
                    "timed out waiting for OAuth callback",
                )));
                break;
            }

            match listener.accept() {
                Ok((stream, _)) => {
                    let result = Self::handle_connection(stream, &expected_state);
                    drop(listener); // Explicitly drop listener to close clone
                    let _ = tx.send(result);
                    break;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    }

    fn handle_connection(
        mut stream: TcpStream,
        expected_state: &str,
    ) -> Result<AuthorizationCode, Failure> {
        stream.set_nonblocking(false).map_err(|e| {
            Failure::failed(
                "callback.blocking_failed",
                format!("could not set stream to blocking mode: {e}"),
            )
        })?;
        stream.set_read_timeout(Some(READ_TIMEOUT)).map_err(|e| {
            Failure::failed(
                "callback.read_failed",
                format!("could not set read timeout: {e}"),
            )
        })?;
        stream.set_write_timeout(Some(READ_TIMEOUT)).map_err(|e| {
            Failure::failed(
                "callback.read_failed",
                format!("could not set write timeout: {e}"),
            )
        })?;

        let mut buf = Vec::new();
        let mut tmp_buf = [0u8; 1024];
        loop {
            let n = stream.read(&mut tmp_buf).map_err(|e| {
                Failure::failed(
                    "callback.read_failed",
                    format!("could not read request: {e}"),
                )
            })?;

            if n == 0 {
                return Err(Failure::failed(
                    "callback.read_failed",
                    "client closed connection before sending complete request headers",
                ));
            }

            buf.extend_from_slice(tmp_buf.get(..n).unwrap_or(&[]));

            if buf.len() > MAX_READ {
                return Err(Failure::failed(
                    "callback.request_too_large",
                    "request headers too large",
                ));
            }

            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        let request = String::from_utf8_lossy(&buf);
        let request = request.into_owned();

        let (method, path, query) = Self::parse_request_line(&request)?;

        if method != "GET" {
            Self::respond(
                &mut stream,
                405,
                "Method Not Allowed",
                "<html><body><h1>405</h1><p>Use GET.</p></body></html>",
            );
            return Err(Failure::failed(
                "callback.method_not_allowed",
                "OAuth callback must be a GET request",
            )
            .expected("GET")
            .actual(method));
        }

        if path != "/callback" {
            Self::respond(
                &mut stream,
                400,
                "Bad Request",
                "<html><body><h1>400</h1><p>Invalid callback path.</p></body></html>",
            );
            return Err(Failure::failed(
                "callback.invalid_request",
                "OAuth callback received on unexpected path",
            )
            .expected("/callback")
            .actual(path));
        }

        let params = Self::parse_query(&query);
        let state = params.get("state").map(String::as_str).unwrap_or("");
        let code = params.get("code").map(String::as_str);

        if params.contains_key("error") {
            Self::respond(
                &mut stream,
                400,
                "Bad Request",
                "<html><body><h1>400</h1><p>OAuth error.</p></body></html>",
            );
            return Err(
                Failure::failed("callback.oauth_error", "OAuth authorization error")
                    .expected("code")
                    .actual("error response"),
            );
        }

        if state != expected_state {
            Self::respond(
                &mut stream,
                400,
                "Bad Request",
                "<html><body><h1>400</h1><p>Invalid state parameter.</p></body></html>",
            );
            return Err(Failure::failed(
                "callback.state_mismatch",
                "OAuth state parameter did not match",
            )
            .expected("state to match"));
        }

        match code {
            Some(c) if !c.is_empty() => {
                Self::respond(
                    &mut stream,
                    200,
                    "OK",
                    "<html><body><h1>Authenticated</h1><p>You may return to the terminal.</p></body></html>",
                );
                Ok(AuthorizationCode(c.to_owned()))
            }
            _ => {
                Self::respond(
                    &mut stream,
                    400,
                    "Bad Request",
                    "<html><body><h1>400</h1><p>Missing authorization code.</p></body></html>",
                );
                Err(Failure::failed(
                    "callback.missing_code",
                    "OAuth callback did not contain an authorization code",
                )
                .expected("code parameter")
                .actual("no code in callback"))
            }
        }
    }

    pub(crate) fn parse_request_line(request: &str) -> Result<(&str, String, String), Failure> {
        let first_line = request.lines().next().unwrap_or("");
        let mut parts = first_line.splitn(3, ' ');
        let method = parts.next().unwrap_or("");
        let full_path = parts.next().unwrap_or("");
        let (path_raw, query_raw) = match full_path.split_once('?') {
            Some((p, q)) => (p, q),
            None => (full_path, ""),
        };
        // Decode the path so %2F becomes /, etc.
        let path = url_decode(path_raw);
        let query = query_raw.to_owned();
        Ok((method, path, query))
    }

    pub(crate) fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
        let mut params = std::collections::HashMap::new();
        if query.is_empty() {
            return params;
        }
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                params.insert(url_decode(key), url_decode(value));
            }
        }
        params
    }

    fn respond(stream: &mut TcpStream, status: u16, reason: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len(),
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        let _ = stream.shutdown(Shutdown::Write);
    }
}

impl Drop for CallbackServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // Drop the listener explicitly — this closes this handle, and
        // triggers the closing of the socket (if the worker's handle is
        // already closed), and unblocks the worker's `accept()`.
        drop(self.listener.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Percent-decode a query-string component: `%XX` → byte, `+` → space.
fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes.get(i) {
            Some(b'+') => {
                out.push(b' ');
                i += 1;
            }
            Some(b'%') if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (
                    hex_val(*bytes.get(i + 1).unwrap_or(&0)),
                    hex_val(*bytes.get(i + 2).unwrap_or(&0)),
                ) {
                    out.push(hi * 16 + lo);
                    i += 3;
                } else if let Some(b) = bytes.get(i) {
                    out.push(*b);
                    i += 1;
                }
            }
            Some(b) => {
                out.push(*b);
                i += 1;
            }
            None => break,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/infra/http_callback.rs"]
mod tests;
