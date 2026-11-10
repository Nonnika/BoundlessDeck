//! `boundlessdeck mcp-stdio [--workspace DIR]`: what agents start. It reads
//! JSON-RPC lines on stdin and relays them to the deck's socket. With a
//! workspace it talks to that folder's deck; without, to the active deck,
//! read again whenever the deck does not answer — so switching decks needs
//! no restart. While no deck is open it still answers the handshake and
//! lists its tools; calls then say why they cannot run.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use super::{Caller, Endpoint, rpc};

fn request(ep: &Endpoint, method: &str, path: &str, body: &[u8], extra: &str) -> Result<(u16, Vec<u8>), String> {
    if ep.socket.is_empty() {
        return Err("工作区还没有 MCP 端点（先在 BoundlessDeck 里打开它）".into());
    }
    if !ep.transport.is_empty() && ep.transport != "unix" {
        return Err(format!("不支持的传输 {}", ep.transport));
    }
    let mut s = UnixStream::connect(&ep.socket).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(60))).ok();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: boundlessdeck\r\nContent-Type: application/json\r\nAccept: application/json\r\nX-BoundlessDeck-Token: {}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        ep.token,
        body.len()
    );
    s.write_all(head.as_bytes()).and_then(|_| s.write_all(body)).map_err(|e| e.to_string())?;
    let mut r = BufReader::new(s);
    let mut status = String::new();
    r.read_line(&mut status).map_err(|e| e.to_string())?;
    let code: u16 = status.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("回应读不懂")?;
    let mut len: Option<usize> = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().ok();
        }
    }
    let mut out = Vec::new();
    match len {
        Some(n) => {
            out.resize(n, 0);
            r.read_exact(&mut out).map_err(|e| e.to_string())?;
        }
        None => {
            r.read_to_end(&mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok((code, out))
}

#[cfg(test)]
pub fn post(ep: &Endpoint, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    post_as(ep, body, &Caller::default())
}

/// Posts as the caller: the deck learns which terminal, if any, it runs in.
pub fn post_as(ep: &Endpoint, body: &[u8], caller: &Caller) -> Result<(u16, Vec<u8>), String> {
    request(ep, "POST", "/mcp", body, &caller.headers())
}

pub fn health(ep: &Endpoint) -> Result<Vec<u8>, String> {
    match request(ep, "GET", "/health", b"", "")? {
        (200, body) => Ok(body),
        (code, body) => Err(format!("返回 {code}：{}", String::from_utf8_lossy(&body).trim())),
    }
}

/// What to say when the deck does not answer: the handshake and lists from
/// here, a failed call for a tool, an error otherwise.
fn offline(line: &[u8], why: &str) -> Option<Value> {
    let msg: Value = serde_json::from_slice(line).ok()?;
    let id = msg.get("id").cloned().filter(|v| !v.is_null())?;
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    if let Some(result) = rpc::answer_offline(method, &params) {
        return Some(serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }
    let text = if why == super::NO_ACTIVE_DECK { why.to_string() } else { format!("BoundlessDeck 没有回应：{why}") };
    if method == "tools/call" {
        return Some(serde_json::json!({"jsonrpc": "2.0", "id": id, "result": rpc::tool_failure(&text)}));
    }
    Some(rpc::error(id, -32000, &text))
}

/// The bridge's main, when the arguments ask for it: Some(exit code).
pub fn main(args: &[String]) -> Option<i32> {
    if !matches!(args.first().map(String::as_str), Some("mcp-stdio" | "--mcp-stdio")) {
        return None;
    }
    let mut workspace = std::env::var("BOUNDLESSDECK_WORKSPACE").ok().filter(|s| !s.is_empty());
    let (mut socket, mut token, mut check) = (None, None, false);
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        let (key, inline) = match a.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (a.as_str(), None),
        };
        let mut val = || inline.clone().or_else(|| it.next().cloned());
        match key.trim_start_matches('-') {
            "workspace" | "w" => workspace = val(),
            "socket" => socket = val(),
            "token" => token = val(),
            "check" => check = true,
            other => {
                eprintln!("boundlessdeck mcp-stdio：不认识的参数 {other}");
                return Some(2);
            }
        }
    }
    let find = move || -> Result<Endpoint, String> {
        match &workspace {
            Some(w) => super::read_endpoint(&std::path::absolute(PathBuf::from(w)).map_err(|e| e.to_string())?),
            None => super::read_active().map(|a| a.server),
        }
    };
    let pinned = socket.is_some();
    let mut ep = Endpoint { transport: "unix".into(), ..Default::default() };
    if socket.is_none() || token.is_none() {
        match find() {
            Ok(found) => ep = found,
            Err(e) => {
                eprintln!("boundlessdeck mcp-stdio：{e}");
                if check {
                    return Some(1);
                }
            }
        }
    }
    if let Some(s) = socket {
        ep.socket = s;
    }
    if let Some(t) = token {
        ep.token = t;
    }
    if check {
        return Some(match health(&ep) {
            Ok(body) => {
                println!("{}", String::from_utf8_lossy(&body));
                0
            }
            Err(e) => {
                eprintln!("boundlessdeck mcp-stdio：{e}");
                1
            }
        });
    }
    let caller = Caller::from_env();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut res = post_as(&ep, line.as_bytes(), &caller);
        if (res.is_err() || res.as_ref().is_ok_and(|r| r.0 == 401)) && !pinned {
            match find() {
                Ok(now) if now.socket != ep.socket || now.token != ep.token => {
                    ep = now;
                    res = post_as(&ep, line.as_bytes(), &caller);
                }
                Ok(_) => {}
                Err(e) => res = Err(e),
            }
        }
        let answer: Option<Vec<u8>> = match res {
            Err(e) => offline(line.as_bytes(), &e).map(|v| serde_json::to_vec(&v).unwrap()),
            Ok((202, _)) => None,
            Ok((_, body)) if body.is_empty() => None,
            Ok((200, body)) => Some(body),
            Ok((code, body)) => {
                let id = serde_json::from_str::<Value>(line).ok().and_then(|v| v.get("id").cloned()).unwrap_or(Value::Null);
                let msg = format!("BoundlessDeck 返回 {code}：{}", String::from_utf8_lossy(&body).trim());
                Some(serde_json::to_vec(&rpc::error(id, -32000, &msg)).unwrap())
            }
        };
        if let Some(a) = answer
            && out.write_all(&a).and_then(|_| out.write_all(b"\n")).and_then(|_| out.flush()).is_err()
        {
            break;
        }
    }
    Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_offline_as_far_as_it_can() {
        let init = offline(br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#, "down").unwrap();
        assert!(init["result"]["serverInfo"].is_object());
        let call = offline(br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"x"}}"#, super::super::NO_ACTIVE_DECK).unwrap();
        assert_eq!(call["result"]["isError"], true);
        assert!(call["result"]["content"][0]["text"].as_str().unwrap().contains("没有打开的图版"));
        assert!(offline(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "down").is_none());
    }

    #[test]
    fn not_ours_unless_asked() {
        assert_eq!(main(&["/some/folder".into()]), None);
    }
}
