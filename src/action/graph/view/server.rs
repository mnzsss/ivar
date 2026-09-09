use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::error::ViewError;
use super::router::{parse_http_request, route_request, validate_security_headers, HttpResponse};
use super::types::ViewSeed;
use crate::store::graph::db::GraphDb;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_READ: usize = 8192;

#[derive(Debug)]
pub struct ViewerServer {
    listener: Option<TcpListener>,
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    db: Arc<Mutex<GraphDb>>,
    seed: ViewSeed,
}

impl ViewerServer {
    pub fn bind(db: GraphDb, seed: ViewSeed) -> Result<Self, ViewError> {
        Self::bind_loopback(db, seed, "127.0.0.1:0")
    }

    pub fn bind_loopback(db: GraphDb, seed: ViewSeed, addr_str: &str) -> Result<Self, ViewError> {
        let listener = TcpListener::bind(addr_str)
            .map_err(|e| ViewError::Bind(format!("failed to bind {addr_str}: {e}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| ViewError::Bind(format!("failed to set non-blocking listener: {e}")))?;
        let addr = listener
            .local_addr()
            .map_err(|e| ViewError::Bind(format!("failed to get local addr: {e}")))?;

        Ok(Self {
            listener: Some(listener),
            addr,
            shutdown: Arc::new(AtomicBool::new(false)),
            db: Arc::new(Mutex::new(db)),
            seed,
        })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        self.shutdown.clone()
    }

    pub fn db_clone(&self) -> Arc<Mutex<GraphDb>> {
        self.db.clone()
    }

    pub fn seed_clone(&self) -> ViewSeed {
        self.seed.clone()
    }

    pub fn handle_connection(&self, mut stream: TcpStream) -> Result<(), ViewError> {
        stream
            .set_nonblocking(false)
            .map_err(ViewError::Io)?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(ViewError::Io)?;
        stream
            .set_write_timeout(Some(READ_TIMEOUT))
            .map_err(ViewError::Io)?;

        let mut buf = [0u8; MAX_READ];
        let n = match stream.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                return Ok(());
            }
            Err(e) => return Err(ViewError::Io(e)),
        };

        let req_bytes = &buf[..n];
        let response = match parse_http_request(req_bytes) {
            Ok(req) => match validate_security_headers(&req, &self.addr) {
                Ok(()) => match self.db.lock() {
                    Ok(guard) => route_request(&req, &guard, &self.seed),
                    Err(poisoned) => route_request(&req, &poisoned.into_inner(), &self.seed),
                },
                Err(ViewError::Security(msg)) => HttpResponse::forbidden(&msg),
                Err(err) => HttpResponse::bad_request(&err.to_string()),
            },
            Err(ViewError::InvalidRequest(msg)) => HttpResponse::bad_request(&msg),
            Err(err) => HttpResponse::internal_error(&err.to_string()),
        };

        let resp_bytes = response.to_bytes();
        let _ = stream.write_all(&resp_bytes);
        let _ = stream.flush();
        Ok(())
    }

    pub fn serve(mut self) -> Result<(), ViewError> {
        let listener = match self.listener.take() {
            Some(l) => l,
            None => return Err(ViewError::Bind("listener already consumed".into())),
        };

        while !self.shutdown.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = self.handle_connection(stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return Err(ViewError::Io(e));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/view/server.rs"]
mod tests;
