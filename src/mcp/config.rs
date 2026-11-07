//! Where agents learn of BoundlessDeck's MCP: their user-level configs (the
//! global install: `boundlessdeck mcp-stdio`, which follows the active
//! deck), and, only when the setting asks, a folder's project-level configs
//! (`.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`). Configs are
//! merged, never replaced: a user's other servers and keys stay, in their
//! order, and a user-level config that will not parse is left alone.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::tools::SERVER_NAME;
use crate::model::write_file_atomic;

/// An agent whose user-level config can list our server.
#[derive(Clone, Debug)]
pub struct Agent {
    pub id: &'static str,
    pub label: &'static str,
    /// Its user-level config, and the object of servers in it.
    pub path: PathBuf,
    pub key: &'static str,
    /// What tells the agent is installed: a file or folder of it.
    pub home: PathBuf,
    /// Its entries carry "type": "stdio".
    pub typed: bool,
}

pub fn agents_in(home: &Path, config: &Path) -> Vec<Agent> {
    let desktop = config.join("Claude");
    let vscode = config.join("Code").join("User");
    vec![
        Agent { id: "claude-code", label: "Claude Code", path: home.join(".claude.json"), key: "mcpServers", home: home.join(".claude"), typed: true },
        Agent { id: "claude-desktop", label: "Claude 桌面版", path: desktop.join("claude_desktop_config.json"), key: "mcpServers", home: desktop, typed: false },
        Agent { id: "cursor", label: "Cursor", path: home.join(".cursor").join("mcp.json"), key: "mcpServers", home: home.join(".cursor"), typed: false },
        Agent { id: "vscode", label: "VS Code", path: vscode.join("mcp.json"), key: "servers", home: vscode, typed: true },
    ]
}

pub fn user_agents() -> Vec<Agent> {
    match (dirs::home_dir(), dirs::config_dir()) {
        (Some(h), Some(c)) => agents_in(&h, &c),
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// The agent is not on this computer.
    Absent,
    /// It is; our server is not in its config.
    NotThere,
    /// Our server is, pointing at this app.
    Installed,
    /// Our server is, pointing at another copy of the app (its command).
    Elsewhere(String),
    /// Its config will not read.
    Broken(String),
}

fn read_object(data: &[u8]) -> Option<Map<String, Value>> {
    match serde_json::from_slice::<Value>(data).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

fn servers_of(top: &Map<String, Value>, key: &str) -> Result<Map<String, Value>, String> {
    match top.get(key) {
        None => Ok(Map::new()),
        Some(Value::Object(m)) => Ok(m.clone()),
        Some(_) => Err(format!("{key} 不是对象")),
    }
}

impl Agent {
    fn entry(&self, exe: &str) -> Value {
        let mut e = json!({"command": exe, "args": ["mcp-stdio"]});
        if self.typed {
            e["type"] = "stdio".into();
        }
        e
    }

    pub fn status(&self, exe: &str) -> State {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return if self.home.exists() { State::NotThere } else { State::Absent };
            }
            Err(e) => return State::Broken(e.to_string()),
        };
        let Some(top) = read_object(&data) else { return State::Broken("不是 JSON 对象".into()) };
        let servers = match servers_of(&top, self.key) {
            Ok(s) => s,
            Err(e) => return State::Broken(e),
        };
        let Some(e) = servers.get(SERVER_NAME) else { return State::NotThere };
        let cmd = e.get("command").and_then(Value::as_str).unwrap_or("").to_string();
        let args: Vec<&str> =
            e.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        if cmd == exe && args == ["mcp-stdio"] { State::Installed } else { State::Elsewhere(cmd) }
    }

    pub fn install(&self, exe: &str) -> Result<(), String> {
        if !self.home.exists() {
            return Err(format!("没有找到 {}", self.label));
        }
        let entry = self.entry(exe);
        edit_user_servers(&self.path, self.key, |s| {
            s.insert(SERVER_NAME.into(), entry);
            true
        })
    }

    pub fn uninstall(&self) -> Result<(), String> {
        edit_user_servers(&self.path, self.key, |s| s.remove(SERVER_NAME).is_some())
    }
}

/// Edits the servers of a user-level config, keeping everything else in it,
/// and its mode; a config that will not parse is left as it is.
fn edit_user_servers(path: &Path, key: &str, edit: impl FnOnce(&mut Map<String, Value>) -> bool) -> Result<(), String> {
    let (mut top, mode) = match std::fs::read(path) {
        Ok(data) => {
            let top = read_object(&data).ok_or("配置文件不是 JSON 对象，没有改动它")?;
            (top, std::fs::metadata(path).ok().map(|m| m.permissions()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Map::new(), None),
        Err(e) => return Err(e.to_string()),
    };
    let mut servers = servers_of(&top, key).map_err(|e| format!("{e}，没有改动它"))?;
    if !edit(&mut servers) {
        return Ok(());
    }
    top.insert(key.into(), Value::Object(servers));
    let data = serde_json::to_vec_pretty(&Value::Object(top)).map_err(|e| e.to_string())?;
    write_file_atomic(path, &data).map_err(|e| e.to_string())?;
    if let Some(m) = mode {
        let _ = std::fs::set_permissions(path, m);
    }
    Ok(())
}

/// The command to install: this app, where it stays.
pub fn install_exe() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let s = exe.to_string_lossy().into_owned();
    if s.contains("/AppTranslocation/") {
        return Err("macOS 正从临时位置运行这个应用：先把 BoundlessDeck 拖进「应用程序」文件夹再打开".into());
    }
    Ok(s)
}

// ---- project-level configs ----

pub struct ProjectFile {
    pub path: PathBuf,
    pub key: &'static str,
    pub label: &'static str,
}

pub fn project_files(root: &Path) -> Vec<ProjectFile> {
    vec![
        ProjectFile { path: root.join(".mcp.json"), key: "mcpServers", label: ".mcp.json" },
        ProjectFile { path: root.join(".cursor").join("mcp.json"), key: "mcpServers", label: ".cursor/mcp.json" },
        ProjectFile { path: root.join(".vscode").join("mcp.json"), key: "servers", label: ".vscode/mcp.json" },
    ]
}

/// Puts our entry into each project config, keeping the rest; a config that
/// will not parse is kept aside as .bak and replaced. Returns warnings.
pub fn write_project_configs(root: &Path, command: &str, args: &[String]) -> Vec<String> {
    let entry = json!({"type": "stdio", "command": command, "args": args});
    let mut warnings = Vec::new();
    for f in project_files(root) {
        let mut top = Map::new();
        match std::fs::read(&f.path) {
            Ok(data) => match read_object(&data) {
                Some(t) => top = t,
                None => {
                    let _ = write_file_atomic(&f.path.with_extension("json.bak"), &data);
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warnings.push(format!("{}：{e}", f.label));
                continue;
            }
        }
        let mut servers = servers_of(&top, f.key).unwrap_or_default();
        servers.insert(SERVER_NAME.into(), entry.clone());
        top.insert(f.key.into(), Value::Object(servers));
        let data = serde_json::to_vec_pretty(&Value::Object(top)).unwrap();
        if let Err(e) = write_file_atomic(&f.path, &data) {
            warnings.push(format!("{}：{e}", f.label));
        }
    }
    warnings
}

pub fn project_has_us(root: &Path) -> bool {
    project_files(root).iter().any(|f| {
        std::fs::read(&f.path)
            .ok()
            .and_then(|d| read_object(&d))
            .and_then(|t| servers_of(&t, f.key).ok())
            .is_some_and(|s| s.contains_key(SERVER_NAME))
    })
}

/// Takes our entry out of the project configs; a file left with nothing
/// but an empty list of servers goes, and its folder if that empties.
pub fn remove_project_configs(root: &Path) -> Result<(), String> {
    let mut errs = Vec::new();
    for f in project_files(root) {
        let Ok(data) = std::fs::read(&f.path) else { continue };
        let Some(mut top) = read_object(&data) else { continue };
        let Ok(mut servers) = servers_of(&top, f.key) else { continue };
        if servers.remove(SERVER_NAME).is_none() {
            continue;
        }
        if servers.is_empty() && top.len() == 1 {
            if let Err(e) = std::fs::remove_file(&f.path) {
                errs.push(format!("{}：{e}", f.label));
            }
            let _ = std::fs::remove_file(f.path.with_extension("json.bak"));
            if let Some(dir) = f.path.parent().filter(|d| *d != root) {
                let _ = std::fs::remove_dir(dir); // only when empty
            }
            continue;
        }
        top.insert(f.key.into(), Value::Object(servers));
        if let Err(e) = write_file_atomic(&f.path, &serde_json::to_vec_pretty(&Value::Object(top)).unwrap()) {
            errs.push(format!("{}：{e}", f.label));
        }
    }
    if errs.is_empty() { Ok(()) } else { Err(errs.join("；")) }
}

// ---- the app's settings ----

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, rename = "projectMCPConfigs")]
    pub project_mcp: bool,
}

fn settings_file() -> Option<PathBuf> {
    crate::model::data_dir().map(|d| d.join("settings.json"))
}

impl Settings {
    pub fn load() -> Settings {
        settings_file().and_then(|f| std::fs::read(f).ok()).and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(f) = settings_file() else { return Ok(()) };
        write_file_atomic(&f, &serde_json::to_vec_pretty(self).unwrap()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_keeps_what_else_the_config_holds() {
        let home = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let ags = agents_in(home.path(), config.path());
        let code = &ags[0];
        assert_eq!(code.status("/app"), State::Absent);
        assert!(code.install("/app").is_err(), "no agent, no install");
        std::fs::create_dir(home.path().join(".claude")).unwrap();
        std::fs::write(&code.path, r#"{"zeta": 1, "mcpServers": {"other": {"command": "x"}}, "alpha": 2}"#).unwrap();
        assert_eq!(code.status("/app"), State::NotThere);
        code.install("/app").unwrap();
        assert_eq!(code.status("/app"), State::Installed);
        assert_eq!(code.status("/other"), State::Elsewhere("/app".into()));
        let text = std::fs::read_to_string(&code.path).unwrap();
        assert!(text.find("zeta").unwrap() < text.find("alpha").unwrap(), "keys keep their order");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["boundlessdeck"]["type"], "stdio");
        code.uninstall().unwrap();
        assert_eq!(code.status("/app"), State::NotThere);
        // A config that will not parse is not touched.
        std::fs::write(&code.path, "{oops").unwrap();
        assert!(matches!(code.status("/app"), State::Broken(_)));
        assert!(code.install("/app").is_err());
        assert_eq!(std::fs::read_to_string(&code.path).unwrap(), "{oops");
    }

    #[test]
    fn project_configs_merge_and_go_cleanly() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".mcp.json"), r#"{"mcpServers": {"mine": {"command": "m"}}}"#).unwrap();
        assert!(write_project_configs(root.path(), "/app", &["mcp-stdio".into()]).is_empty());
        assert!(project_has_us(root.path()));
        let v: Value = serde_json::from_slice(&std::fs::read(root.path().join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["mine"]["command"], "m");
        remove_project_configs(root.path()).unwrap();
        assert!(!project_has_us(root.path()));
        assert!(root.path().join(".mcp.json").exists(), "the user's server stays");
        assert!(!root.path().join(".cursor").exists(), "a folder only we used goes");
    }
}
