//! The collector: a listening node that also serves the picture.
//!
//! Hand-rolled HTTP/1.1 again — three routes, one of them static. A framework
//! here would be more moving parts than protocol.
//!
//! Two roles, one binary:
//!
//!   * `airspace serve` — listen locally, accept observations from other nodes,
//!     serve the page.
//!   * `airspace feed URL` — listen locally, send everything to a collector.
//!
//! The second one is what makes direction possible at all. One receiver hears
//! a distance; three receivers at known positions hear a place.

use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::model::Observation;
use crate::observe::Listener;
use crate::space::{Node, State};

const UI: &str = include_str!("ui.html");
const MAX_BODY: usize = 4 * 1024 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Batch {
    pub node: Node,
    pub obs: Vec<Observation>,
}

pub async fn serve(state: State, bind: &str) -> Result<()> {
    let listener = TcpListener::bind(bind).await?;
    let state = Arc::new(state);

    // The collector is also a node. A machine that serves the picture but does
    // not appear in it would be an odd blind spot at a known position.
    {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = listen_locally(&state).await {
                eprintln!("local radio stopped: {e}");
            }
        });
    }

    eprintln!("airspace: http://{bind}");
    loop {
        let (sock, _) = match listener.accept().await {
            Ok(x) => x,
            Err(_) => continue,
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _ = handle(sock, state).await;
        });
    }
}

/// Sweep the local adapter forever, into shared state.
pub async fn listen_locally(state: &State) -> Result<()> {
    let radio = Listener::new().await?;
    radio.start().await?;
    let node = state.config.node.clone();
    loop {
        tokio::time::sleep(crate::observe::SWEEP).await;
        radio.keep_alive().await;
        if let Ok(obs) = radio.sweep().await {
            state.ingest(&node, &obs);
        }
    }
}

/// Run as a remote ear: listen here, report there.
///
/// Plain HTTP on purpose — this is for a LAN or a tailnet, and a TLS stack
/// would be the largest thing in the binary by an order of magnitude. Do not
/// run it across the open internet; the token is not a substitute for a
/// tunnel you already trust.
pub async fn feed(state: State, url: &str, token: &str) -> Result<()> {
    let (host, path) = split_url(url)?;
    let radio = Listener::new().await?;
    radio.start().await?;
    let node = state.config.node.clone();
    eprintln!("feeding {host}{path} as node {:?} at ({}, {})", node.name, node.x, node.y);

    loop {
        tokio::time::sleep(crate::observe::SWEEP).await;
        radio.keep_alive().await;
        let obs = radio.sweep().await.unwrap_or_default();
        if obs.is_empty() {
            continue;
        }
        let body = serde_json::to_vec(&Batch { node: node.clone(), obs })?;
        if let Err(e) = post(&host, &path, token, &body).await {
            eprintln!("collector unreachable: {e}");
        }
    }
}

pub async fn post(host: &str, path: &str, token: &str, body: &[u8]) -> Result<()> {
    let mut sock = TcpStream::connect(host).await?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    sock.write_all(head.as_bytes()).await?;
    sock.write_all(body).await?;
    sock.flush().await?;
    let mut resp = [0u8; 64];
    let _ = sock.read(&mut resp).await;
    Ok(())
}

pub fn split_url(url: &str) -> Result<(String, String)> {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/ingest"),
    };
    let host = if host.contains(':') { host.to_string() } else { format!("{host}:9970") };
    Ok((host, path.to_string()))
}

async fn handle(mut sock: TcpStream, state: Arc<State>) -> Result<()> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 16 * 1024 {
            return reply(&mut sock, "400 Bad Request", "text/plain", b"").await;
        }
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let req = lines.next().unwrap_or_default();
    let mut parts = req.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default().split('?').next().unwrap_or_default();

    match (method, path) {
        ("GET", "/") => reply(&mut sock, "200 OK", "text/html; charset=utf-8", UI.as_bytes()).await,
        ("GET", "/api/state") => {
            let body = serde_json::to_vec(&state.snapshot())?;
            reply(&mut sock, "200 OK", "application/json", &body).await
        }
        ("POST", "/ingest") => {
            let want = state.config.collector.token.trim();
            let given = header(&head, "authorization")
                .and_then(|v| v.strip_prefix("Bearer ").map(str::to_string))
                .unwrap_or_default();
            // An ingest endpoint with no token lets anyone on the network draw
            // imaginary people into the room. Refuse rather than degrade.
            if want.is_empty() || !constant_time_eq(want.as_bytes(), given.as_bytes()) {
                return reply(&mut sock, "404 Not Found", "text/plain", b"").await;
            }
            let len: usize = header(&head, "content-length")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            if len > MAX_BODY {
                return reply(&mut sock, "413 Payload Too Large", "text/plain", b"").await;
            }
            let mut body = buf[head_end..].to_vec();
            while body.len() < len {
                let n = sock.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            match serde_json::from_slice::<Batch>(&body) {
                Ok(b) => {
                    state.ingest(&b.node, &b.obs);
                    reply(&mut sock, "204 No Content", "text/plain", b"").await
                }
                Err(_) => reply(&mut sock, "400 Bad Request", "text/plain", b"").await,
            }
        }
        _ => reply(&mut sock, "404 Not Found", "text/plain", b"").await,
    }
}

fn header(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

async fn reply(sock: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    sock.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        sock.write_all(body).await?;
    }
    sock.flush().await?;
    Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_collector_urls() {
        assert_eq!(
            split_url("http://big-brother:9970/ingest").unwrap(),
            ("big-brother:9970".into(), "/ingest".into())
        );
        // A bare host gets the default port and the default path.
        assert_eq!(split_url("carl").unwrap(), ("carl:9970".into(), "/ingest".into()));
    }

    #[test]
    fn reads_headers_case_insensitively() {
        let h = "POST /ingest HTTP/1.1\r\nAuthorization: Bearer abc\r\nContent-Length: 9\r\n\r\n";
        assert_eq!(header(h, "authorization").as_deref(), Some("Bearer abc"));
        assert_eq!(header(h, "CONTENT-LENGTH").as_deref(), Some("9"));
        assert_eq!(header(h, "missing"), None);
    }
}
