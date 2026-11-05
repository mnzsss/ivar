#![allow(clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

pub(crate) struct FakeTelegram {
    pub(crate) api: String,
    handle: JoinHandle<Vec<String>>,
}

impl FakeTelegram {
    pub(crate) fn start(replies: Vec<(u16, &'static str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            replies
                .into_iter()
                .map(|(status, body)| {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&mut stream);
                    stream
                        .write_all(
                            format!(
                                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    request
                })
                .collect()
        });
        Self { api, handle }
    }

    pub(crate) fn requests(self) -> Vec<String> {
        self.handle.join().unwrap()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&request);
        let Some(head_end) = text.find("\r\n\r\n") else {
            continue;
        };
        let length = text[..head_end]
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if request.len() >= head_end + 4 + length {
            break;
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}
