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
use std::time::Duration;

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
                // Exit rather than serve an empty map from a live process.
                std::process::exit(1);
            }
        });
    }

    eprintln!("airspace: http://{bind}");
    accept_loop(listener, state, MAX_CONNECTIONS, REQUEST_DEADLINE).await
}

/// How long one whole request may take — head, body and reply together.
///
/// Per-read deadlines alone are not enough: a client that sends one byte every
/// nine seconds never trips a ten-second read timeout, and holds its task for
/// as long as it likes.
const REQUEST_DEADLINE: Duration = Duration::from_secs(15);

/// Connections served at once. Past this the collector stops accepting and
/// lets the kernel backlog hold the rest, rather than spawning a task and a
/// buffer for every socket anyone cares to open.
const MAX_CONNECTIONS: usize = 64;

async fn accept_loop(
    listener: TcpListener,
    state: Arc<State>,
    max: usize,
    deadline: Duration,
) -> Result<()> {
    let permits = Arc::new(tokio::sync::Semaphore::new(max));
    loop {
        let permit = permits.clone().acquire_owned().await?;
        let sock = match listener.accept().await {
            Ok((sock, _)) => sock,
            // Out of file descriptors, most likely. Retrying at once is a hot
            // loop that pegs a core while nothing gets served; back off.
            Err(_) => {
                drop(permit);
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(deadline, handle(sock, state)).await;
            drop(permit);
        });
    }
}

/// Sweep the local adapter forever, into shared state.
///
/// The error handling here is deliberate and was learned the hard way. This
/// loop used to read `if let Ok(obs) = radio.sweep().await`, which swallows a
/// dead D-Bus connection silently: the process stays up, systemd sees a healthy
/// unit, the page keeps rendering, and it shows an empty room forever. A
/// listening tool that has gone deaf must not look identical to a quiet room.
///
/// So failures are counted and reported, and a run of them ends the task —
/// which ends the process, which makes systemd restart it into a working
/// state. Crashing loudly beats serving a confident lie.
const GIVE_UP_AFTER: u32 = 10;

pub async fn listen_locally(state: &State) -> Result<()> {
    let radio = Listener::new().await?;
    radio.start().await?;
    let node = state.config.node.clone();
    let mut failures: u32 = 0;
    loop {
        tokio::time::sleep(crate::observe::SWEEP).await;
        let _ = radio.keep_alive().await;
        match radio.sweep().await {
            Ok(obs) => {
                if failures > 0 {
                    eprintln!("radio recovered after {failures} failed sweeps");
                    failures = 0;
                }
                state.ingest(&node, &obs);
            }
            Err(e) => {
                failures += 1;
                eprintln!("sweep failed ({failures}/{GIVE_UP_AFTER}): {e}");
                if failures >= GIVE_UP_AFTER {
                    anyhow::bail!("radio unreadable after {GIVE_UP_AFTER} attempts; exiting so it can be restarted");
                }
            }
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
        let _ = radio.keep_alive().await;
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
    let head_deadline = std::time::Duration::from_secs(10);
    let head_end = loop {
        let n = match tokio::time::timeout(head_deadline, sock.read(&mut chunk)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return reply(&mut sock, "408 Request Timeout", "text/plain", b"").await,
        };
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

    // DNS rebinding: a web page you visit can point its own hostname at
    // 127.0.0.1 and read the map as if it were same-origin. The browser still
    // sends the page's hostname, so anything that is not a name this machine
    // answers to is refused. /ingest is exempt: it is token-checked, and the
    // ears address the collector by whatever name their network gives it.
    if path != "/ingest"
        && !host_allowed(header(&head, "host").as_deref(), &state.config.collector.hosts)
    {
        return reply(&mut sock, "421 Misdirected Request", "text/plain", b"").await;
    }

    match (method, path) {
        ("GET", "/") => reply(&mut sock, "200 OK", "text/html; charset=utf-8", UI.as_bytes()).await,
        ("GET", "/api/state") => {
            let body = serde_json::to_vec(&state.snapshot())?;
            reply(&mut sock, "200 OK", "application/json", &body).await
        }
        // Fit a path-loss model to samples the console collected.
        //
        // The console does the measuring and owns the file; the collector does
        // the arithmetic. That split keeps the fit in the one place it is
        // tested, and keeps the daemon from ever needing write access to a
        // home directory its unit mounts read-only.
        // Room dimensions, so the console can draw and reshape it.
        ("GET", "/api/room") => {
            let body = serde_json::to_vec(&state.config.room)?;
            reply(&mut sock, "200 OK", "application/json", &body).await
        }
        ("POST", "/api/fit") => {
            let len: usize = header(&head, "content-length")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            if len == 0 || len > 64 * 1024 {
                return reply(&mut sock, "400 Bad Request", "text/plain", b"").await;
            }
            let mut body = buf[head_end..].to_vec();
            let deadline = std::time::Duration::from_secs(10);
            while body.len() < len {
                let n = match tokio::time::timeout(deadline, sock.read(&mut chunk)).await {
                    Ok(Ok(n)) => n,
                    _ => break,
                };
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            #[derive(serde::Deserialize)]
            struct Req {
                samples: Vec<(f32, i16)>,
            }
            match serde_json::from_slice::<Req>(&body) {
                Ok(r) => match crate::model::fit_full(&r.samples) {
                    Some(f) => {
                        let out = serde_json::json!({
                            "rssi_at_1m": f.rssi_at_1m,
                            "exponent": f.exponent,
                            "rms_db": f.rms_db,
                            "samples": r.samples.len(),
                        });
                        reply(&mut sock, "200 OK", "application/json",
                              &serde_json::to_vec(&out)?).await
                    }
                    // Refusing is the useful answer: it means the readings do
                    // not describe a radio getting weaker with distance, and
                    // storing a fit anyway would bake that in.
                    None => reply(&mut sock, "422 Unprocessable Content", "text/plain",
                                  b"those readings do not fit a path-loss model").await,
                },
                Err(_) => reply(&mut sock, "400 Bad Request", "text/plain", b"").await,
            }
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
            // A client that announces a large Content-Length and then sends
            // nothing holds this task open forever. That is slowloris: a
            // handful of connections and the collector stops answering, with
            // no error anywhere because nothing has failed — it is still
            // politely waiting. Every read gets a deadline.
            let mut body = buf[head_end..].to_vec();
            let deadline = std::time::Duration::from_secs(10);
            while body.len() < len {
                let n = match tokio::time::timeout(deadline, sock.read(&mut chunk)).await {
                    Ok(Ok(n)) => n,
                    Ok(Err(_)) => break,
                    Err(_) => {
                        return reply(&mut sock, "408 Request Timeout", "text/plain", b"").await
                    }
                };
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

/// Is this `Host` header one the page may legitimately be reached by?
///
/// An IP literal is always fine — rebinding needs a hostname to rebind. So is
/// `localhost`, this machine's own hostname, and anything listed in
/// `collector.hosts`.
fn host_allowed(host: Option<&str>, extra: &[String]) -> bool {
    let Some(h) = host.map(str::trim).filter(|h| !h.is_empty()) else { return false };
    if let Some(rest) = h.strip_prefix('[') {
        return rest
            .split_once(']')
            .is_some_and(|(v6, _)| v6.parse::<std::net::Ipv6Addr>().is_ok());
    }
    let name = h.rsplit_once(':').map_or(h, |(n, _)| n);
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name.parse::<std::net::Ipv4Addr>().is_ok() || name == "localhost" {
        return true;
    }
    let me = crate::space::hostname().to_ascii_lowercase();
    if !me.is_empty() && (name == me || name == format!("{me}.local")) {
        return true;
    }
    extra.iter().any(|e| e.trim().trim_end_matches('.').eq_ignore_ascii_case(&name))
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
    fn only_names_this_machine_answers_to_are_served() {
        let extra = vec!["airspace.example.net".to_string()];
        for ok in ["127.0.0.1:9970", "localhost:9970", "LOCALHOST", "192.168.1.5",
                   "[::1]:9970", "airspace.example.net:9970", "airspace.example.net."] {
            assert!(host_allowed(Some(ok), &extra), "{ok} should be allowed");
        }
        for bad in ["attacker.example", "attacker.example:9970", "127.0.0.1.attacker.example",
                    "localhost.attacker.example", "[evil]:9970", ""] {
            assert!(!host_allowed(Some(bad), &extra), "{bad} should be refused");
        }
        assert!(!host_allowed(None, &extra));
    }

    async fn start(max: usize, deadline: Duration) -> std::net::SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let a = l.local_addr().unwrap();
        let st = Arc::new(State::new(crate::space::Config::default()));
        tokio::spawn(accept_loop(l, st, max, deadline));
        a
    }

    async fn get(a: std::net::SocketAddr, host: &str, path: &str) -> String {
        let mut s = TcpStream::connect(a).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out).await;
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn a_rebound_hostname_gets_nothing() {
        let a = start(8, Duration::from_secs(5)).await;
        assert!(get(a, "attacker.example", "/api/state").await.starts_with("HTTP/1.1 421"));
        assert!(get(a, "attacker.example", "/").await.starts_with("HTTP/1.1 421"));
        let ok = get(a, &a.to_string(), "/api/state").await;
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        // /ingest is token-checked instead, whatever the Host says.
        assert!(get(a, "attacker.example", "/ingest").await.starts_with("HTTP/1.1 404"));
    }

    #[tokio::test]
    async fn a_trickled_request_is_cut_off_by_the_overall_deadline() {
        let a = start(8, Duration::from_millis(800)).await;
        let mut s = TcpStream::connect(a).await.unwrap();
        let started = std::time::Instant::now();
        s.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
        // One header byte every 200 ms: never slow enough for a per-read
        // timeout, forever if nothing bounds the whole request.
        let mut closed = false;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if s.write_all(b"X").await.is_err() {
                closed = true;
                break;
            }
            let mut b = [0u8; 16];
            if let Ok(Ok(0)) = tokio::time::timeout(Duration::from_millis(1), s.read(&mut b)).await {
                closed = true;
                break;
            }
        }
        assert!(closed, "the connection was still open after 6 s");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn connections_past_the_cap_wait_their_turn() {
        let a = start(1, Duration::from_secs(10)).await;
        // Hold the only slot with a connection that says nothing.
        let hog = TcpStream::connect(a).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let waiting = tokio::spawn(get(a, "localhost", "/api/room"));
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!waiting.is_finished(), "a second connection was served past the cap");
        drop(hog);
        let r = tokio::time::timeout(Duration::from_secs(3), waiting).await.unwrap().unwrap();
        assert!(r.starts_with("HTTP/1.1 200"), "{r}");
    }

    #[test]
    fn reads_headers_case_insensitively() {
        let h = "POST /ingest HTTP/1.1\r\nAuthorization: Bearer abc\r\nContent-Length: 9\r\n\r\n";
        assert_eq!(header(h, "authorization").as_deref(), Some("Bearer abc"));
        assert_eq!(header(h, "CONTENT-LENGTH").as_deref(), Some("9"));
        assert_eq!(header(h, "missing"), None);
    }
}
