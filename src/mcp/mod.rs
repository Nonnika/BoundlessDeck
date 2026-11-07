//! The workspace's MCP endpoint. Opening a deck serves its canvas over
//! JSON-RPC (HTTP on a Unix socket in `.boundlessdeck/`), writes
//! `.boundlessdeck/mcp.json` saying how to reach it, and makes it the
//! active deck in the user's registry, which a global stdio bridge
//! (`boundlessdeck mcp-stdio`) follows. The files are the Go build's: a
//! bridge of either build reaches a deck of either.

pub mod bridge;
pub mod rpc;
pub mod server;
pub mod tools;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{DECK_DIR, write_file_atomic};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_VERSION: &str = "0.1.0";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BridgeCmd {
    pub command: String,
    pub args: Vec<String>,
}

/// How to reach a deck's MCP.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Endpoint {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub transport: String,
    #[serde(default)]
    pub socket: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub bridge: BridgeCmd,
    #[serde(default)]
    pub tools: usize,
    #[serde(default, rename = "toolNames")]
    pub tool_names: Vec<String>,
    #[serde(default, rename = "protocolVersion")]
    pub protocol_version: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServerRef {
    pub name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transport: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own: bool,
}

/// `.boundlessdeck/mcp.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Workspace {
    pub version: i32,
    pub generator: String,
    pub workspace: String,
    pub server: Endpoint,
    #[serde(rename = "schemaFile")]
    pub schema_file: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    pub servers: Vec<ServerRef>,
    #[serde(rename = "updatedAt")]
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

/// The active-deck registry: the deck the app has open.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Active {
    pub workspace: String,
    pub server: Endpoint,
    pub pid: u32,
    #[serde(rename = "updatedAt")]
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

/// Where the socket goes: in the deck's folder, or, when that path is too
/// long for a Unix socket, in the temp folder by the root's hash.
pub fn socket_path(root: &Path) -> PathBuf {
    let preferred = root.join(DECK_DIR).join("mcp.sock");
    if preferred.as_os_str().len() <= 92 {
        return preferred;
    }
    let mut h: u32 = 0x811c9dc5;
    for b in root.as_os_str().as_encoded_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    std::env::temp_dir().join(format!("boundlessdeck-mcp-{h:08x}.sock"))
}

/// The registry's file, unless a test put it elsewhere.
pub fn active_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("BOUNDLESSDECK_ACTIVE_FILE") {
        return Some(PathBuf::from(p));
    }
    dirs::config_dir().map(|d| d.join("BoundlessDeck").join("active-deck.json"))
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    write_file_atomic(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Writes the deck's descriptor and schema, and makes it the active deck.
pub fn publish(root: &Path, ep: &Endpoint) -> Result<(), String> {
    let dir = root.join(DECK_DIR);
    let schema = serde_json::to_vec_pretty(&tools::schema_digest()).map_err(|e| e.to_string())?;
    write_file_atomic(&dir.join("block-schema.json"), &schema).map_err(|e| e.to_string())?;
    let ws = Workspace {
        version: 1,
        generator: "BoundlessDeck".into(),
        workspace: root.to_string_lossy().into_owned(),
        server: ep.clone(),
        schema_file: format!("{DECK_DIR}/block-schema.json"),
        schema_version: tools::SCHEMA_VERSION,
        servers: vec![ServerRef {
            name: ep.name.clone(),
            source: format!("{DECK_DIR}/mcp.json"),
            transport: ep.transport.clone(),
            own: true,
        }],
        updated_at: chrono::Local::now().fixed_offset(),
    };
    let data = serde_json::to_vec_pretty(&ws).map_err(|e| e.to_string())?;
    write_private(&dir.join("mcp.json"), &data).map_err(|e| e.to_string())?;
    let Some(path) = active_path() else { return Err("找不到用户配置目录".into()) };
    let active = Active {
        workspace: root.to_string_lossy().into_owned(),
        server: ep.clone(),
        pid: std::process::id(),
        updated_at: chrono::Local::now().fixed_offset(),
    };
    let data = serde_json::to_vec_pretty(&active).map_err(|e| e.to_string())?;
    write_private(&path, &data).map_err(|e| format!("全局 MCP 找不到这个图版：{e}"))
}

/// Forgets the active deck, if it is still root's.
pub fn unpublish(root: &Path) {
    let Some(path) = active_path() else { return };
    if read_active().is_ok_and(|a| a.workspace != root.to_string_lossy()) {
        return;
    }
    let _ = std::fs::remove_file(path);
}

pub fn read_active() -> Result<Active, String> {
    let path = active_path().ok_or("找不到用户配置目录")?;
    let data = std::fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            NO_ACTIVE_DECK.to_string()
        } else {
            e.to_string()
        }
    })?;
    let mut a: Active = serde_json::from_slice(&data).map_err(|e| format!("解不开 {}：{e}", path.display()))?;
    if a.server.socket.is_empty() {
        return Err(format!("{} 里没有 server.socket", path.display()));
    }
    if a.server.transport.is_empty() {
        a.server.transport = "unix".into();
    }
    Ok(a)
}

pub const NO_ACTIVE_DECK: &str = "BoundlessDeck 没有打开的图版：先在应用里打开一个文件夹，再重试";

/// What a bridge started for a workspace reads to find its deck.
pub fn read_endpoint(workspace: &Path) -> Result<Endpoint, String> {
    let path = workspace.join(DECK_DIR).join("mcp.json");
    let data = std::fs::read(&path).map_err(|e| format!("读 {}：{e}", path.display()))?;
    let ws: Workspace = serde_json::from_slice(&data).map_err(|e| format!("解不开 {}：{e}", path.display()))?;
    if ws.server.socket.is_empty() {
        return Err(format!("{} 里没有 server.socket", path.display()));
    }
    let mut ep = ws.server;
    if ep.transport.is_empty() {
        ep.transport = "unix".into();
    }
    Ok(ep)
}
