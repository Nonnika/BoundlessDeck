//! The endpoint: HTTP/1.1 on a Unix socket, readable by the user alone,
//! each request carrying the deck's token. POST /mcp takes JSON-RPC,
//! GET /health says the deck is there.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;

use super::rpc::{self, Runner};
use super::tools::SERVER_NAME;

const MAX_BODY: usize = 8 << 20;

pub struct Server {
    pub socket: PathBuf,
    pub token: String,
    pub started: chrono::DateTime<chrono::FixedOffset>,
    stopped: Arc<AtomicBool>,
}

fn new_token() -> String {
    (0..16).map(|_| format!("{:02x}", rand::random::<u8>())).collect()
}

impl Server {
    /// Listens on `socket` and answers with `run`.
    pub fn start(socket: &Path, workspace: String, run: Arc<Runner>) -> std::io::Result<Server> {
        if let Some(dir) = socket.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
        }
        let token = new_token();
        let stopped = Arc::new(AtomicBool::new(false));
        let started = chrono::Local::now().fixed_offset();
        let (tok, stop, sock) = (token.clone(), stopped.clone(), socket.to_path_buf());
        std::thread::Builder::new().name("mcp".into()).spawn(move || {
            for conn in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (tok, run, stop, ws, sock) = (tok.clone(), run.clone(), stop.clone(), workspace.clone(), sock.clone());
                std::thread::spawn(move || {
                    let _ = serve(conn, &tok, &*run, &stop, &ws, &sock, started);
                });
            }
        })?;
        Ok(Server { socket: socket.to_path_buf(), token, started, stopped })
    }

    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        // Wake the accept loop so it sees it is stopped.
        let _ = UnixStream::connect(&self.socket);
        let _ = std::fs::remove_file(&self.socket);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn read_request(r: &mut BufReader<UnixStream>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut req = Request { method, path, headers, body: Vec::new() };
    let len: usize = req.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if len > MAX_BODY {
        return Err(std::io::Error::other("too large"));
    }
    req.body = vec![0; len];
    r.read_exact(&mut req.body)?;
    Ok(Some(req))
}

fn respond(w: &mut UnixStream, status: u16, reason: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        w,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    )?;
    w.write_all(body)?;
    w.flush()
}

fn authorized(req: &Request, token: &str) -> bool {
    let got = req
        .header("X-BoundlessDeck-Token")
        .map(str::to_string)
        .or_else(|| req.header("Authorization").and_then(|a| a.strip_prefix("Bearer ")).map(str::to_string))
        .unwrap_or_default();
    // Constant time, as a secret's comparison should be.
    got.len() == token.len() && got.bytes().zip(token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn serve(
    conn: UnixStream,
    token: &str,
    run: &Runner,
    stop: &AtomicBool,
    workspace: &str,
    sock: &Path,
    started: chrono::DateTime<chrono::FixedOffset>,
) -> std::io::Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(120)))?;
    let mut w = conn.try_clone()?;
    let mut r = BufReader::new(conn);
    while let Some(req) = read_request(&mut r)? {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if !authorized(&req, token) {
            respond(&mut w, 401, "Unauthorized", "text/plain", b"unauthorized")?;
            continue;
        }
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/health") => {
                let body = json!({
                    "ok": true, "server": SERVER_NAME, "workspace": workspace,
                    "socket": sock.to_string_lossy(), "tools": super::tools::tool_names().len(),
                    "startedAt": started.to_rfc3339(),
                });
                respond(&mut w, 200, "OK", "application/json", body.to_string().as_bytes())?;
            }
            ("POST", "/mcp") => match rpc::handle_body(&req.body, run) {
                Some(out) => respond(&mut w, 200, "OK", "application/json", &out)?,
                None => respond(&mut w, 202, "Accepted", "text/plain", b"")?,
            },
            (_, "/mcp") => respond(&mut w, 405, "Method Not Allowed", "text/plain", b"POST only")?,
            _ => respond(&mut w, 404, "Not Found", "text/plain", b"not found")?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn serves_rpc_with_the_token_alone() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("t.sock");
        let run: Arc<Runner> = Arc::new(|job| match job {
            rpc::Job::Tool(name, _) => Ok(json!({"ran": name})),
            rpc::Job::Resource(uri) => Ok(json!({"uri": uri})),
        });
        let s = Server::start(&sock, "/ws".into(), run).unwrap();
        let ep = crate::mcp::Endpoint { socket: sock.to_string_lossy().into(), token: s.token.clone(), ..Default::default() };
        let (status, body) = super::super::bridge::post(&ep, br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"deck_info"}}"#).unwrap();
        assert_eq!(status, 200);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["result"]["structuredContent"]["ran"], "deck_info");
        let wrong = crate::mcp::Endpoint { token: "x".into(), ..ep.clone() };
        assert_eq!(super::super::bridge::post(&wrong, b"{}").unwrap().0, 401);
        let health = super::super::bridge::health(&ep).unwrap();
        assert!(String::from_utf8(health).unwrap().contains("\"ok\":true"));
        s.stop();
        assert!(!sock.exists());
    }
}
