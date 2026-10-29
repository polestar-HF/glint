//! A tiny HTTP/1.1 mock server on a tokio `TcpListener`, for the integration tests.
//!
//! It records every request, answers through a closure, and always closes the connection after
//! the reply. Bodies can be sent in chunks with delays between them (to exercise streaming) or
//! left hanging until the client disconnects (to test cancellation).

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    /// Path and query string, as sent.
    pub target: String,
    /// Header names are lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn query(&self) -> Vec<(String, String)> {
        let q = self.target.split_once('?').map(|(_, q)| q).unwrap_or("");
        parse_urlencoded(q)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("request body is JSON")
    }

    pub fn form(&self) -> Vec<(String, String)> {
        parse_urlencoded(&String::from_utf8_lossy(&self.body))
    }

    pub fn form_value(&self, key: &str) -> String {
        self.form().into_iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or_default()
    }

    pub fn query_value(&self, key: &str) -> Option<String> {
        self.query().into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

fn parse_urlencoded(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 3 <= bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub enum Body {
    Full(Vec<u8>),
    /// Chunks with a delay (milliseconds) before each one.
    Chunks(Vec<(u64, Vec<u8>)>),
    /// Send these chunks, then keep the connection open until the client goes away.
    HangAfter(Vec<(u64, Vec<u8>)>),
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Body::Full(body.as_bytes().to_vec()),
        }
    }

    pub fn text(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: Body::Full(body.as_bytes().to_vec()),
        }
    }

    /// A server-sent-events reply, with the stream cut into `pieces` (each sent as its own
    /// HTTP chunk, 15 ms apart).
    pub fn sse(pieces: &[&str]) -> Reply {
        Reply {
            status: 200,
            headers: vec![("Content-Type".into(), "text/event-stream".into())],
            body: Body::Chunks(pieces.iter().map(|p| (15, p.as_bytes().to_vec())).collect()),
        }
    }

    pub fn sse_hanging(pieces: &[&str]) -> Reply {
        Reply {
            status: 200,
            headers: vec![("Content-Type".into(), "text/event-stream".into())],
            body: Body::HangAfter(pieces.iter().map(|p| (0, p.as_bytes().to_vec())).collect()),
        }
    }

    pub fn with_header(mut self, k: &str, v: &str) -> Reply {
        self.headers.push((k.into(), v.into()));
        self
    }
}

/// Cut `s` into pieces of at most `n` bytes, on char boundaries.
pub fn split_every(s: &str, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if cur.len() + c.len_utf8() > n && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub struct Mock {
    /// `http://127.0.0.1:PORT`
    pub base: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    disconnects: Arc<AtomicUsize>,
}

impl Mock {
    /// Start a server. `handler` gets each request and its zero-based index.
    pub async fn start<F>(handler: F) -> Mock
    where
        F: Fn(&Recorded, usize) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let disconnects = Arc::new(AtomicUsize::new(0));
        let handler: Arc<dyn Fn(&Recorded, usize) -> Reply + Send + Sync> = Arc::new(handler);
        let counter = Arc::new(AtomicUsize::new(0));
        {
            let requests = requests.clone();
            let disconnects = disconnects.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else { break };
                    let requests = requests.clone();
                    let disconnects = disconnects.clone();
                    let handler = handler.clone();
                    let counter = counter.clone();
                    tokio::spawn(async move {
                        serve(stream, handler, counter, requests, disconnects).await;
                    });
                }
            });
        }
        Mock { base: format!("http://127.0.0.1:{port}"), requests, disconnects }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn count(&self) -> usize {
        self.requests().len()
    }

    /// How many hanging replies ended because the client closed the connection.
    pub fn disconnects(&self) -> usize {
        self.disconnects.load(Ordering::SeqCst)
    }
}

async fn serve(
    mut stream: TcpStream,
    handler: Arc<dyn Fn(&Recorded, usize) -> Reply + Send + Sync>,
    counter: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Recorded>>>,
    disconnects: Arc<AtomicUsize>,
) {
    let Some(rec) = read_request(&mut stream).await else { return };
    let index = counter.fetch_add(1, Ordering::SeqCst);
    requests.lock().unwrap_or_else(|p| p.into_inner()).push(rec.clone());
    let reply = handler(&rec, index);

    let reason = match reply.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        456 => "Quota Exceeded",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let mut head = format!("HTTP/1.1 {} {}\r\nConnection: close\r\n", reply.status, reason);
    for (k, v) in &reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    match reply.body {
        Body::Full(bytes) => {
            head.push_str(&format!("Content-Length: {}\r\n\r\n", bytes.len()));
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&bytes).await;
        }
        Body::Chunks(chunks) => {
            head.push_str("Transfer-Encoding: chunked\r\n\r\n");
            if send_chunks(&mut stream, &head, chunks).await {
                let _ = stream.write_all(b"0\r\n\r\n").await;
            }
        }
        Body::HangAfter(chunks) => {
            head.push_str("Transfer-Encoding: chunked\r\n\r\n");
            if send_chunks(&mut stream, &head, chunks).await {
                // Wait for the client to hang up; that is how cancellation shows on the wire.
                let mut buf = [0u8; 64];
                loop {
                    match tokio::time::timeout(Duration::from_secs(20), stream.read(&mut buf)).await {
                        Ok(Ok(0)) | Ok(Err(_)) => {
                            disconnects.fetch_add(1, Ordering::SeqCst);
                            break;
                        }
                        Ok(Ok(_)) => continue,
                        Err(_) => break,
                    }
                }
            }
            return;
        }
    }
    let _ = stream.shutdown().await;
}

/// Write the head and then each chunk after its delay. False if the client went away.
async fn send_chunks(stream: &mut TcpStream, head: &str, chunks: Vec<(u64, Vec<u8>)>) -> bool {
    if stream.write_all(head.as_bytes()).await.is_err() {
        return false;
    }
    for (delay, data) in chunks {
        tokio::time::sleep(Duration::from_millis(delay)).await;
        let frame = [format!("{:x}\r\n", data.len()).into_bytes(), data, b"\r\n".to_vec()].concat();
        if stream.write_all(&frame).await.is_err() || stream.flush().await.is_err() {
            return false;
        }
    }
    true
}

async fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        if let Some(p) = find(&buf, b"\r\n\r\n") {
            break p;
        }
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(len);
    Some(Recorded { method, target, headers, body })
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
