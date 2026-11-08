//! Where agents learn of BoundlessDeck's MCP: their user-level configs (the
//! global install: `boundlessdeck mcp-stdio`, which follows the active
//! deck), and, only when the setting asks, a folder's project-level configs
//! (`.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`). Configs are
//! merged, never replaced: a user's other servers and keys stay, in their
//! order, and a user-level config that will not parse is left alone.
//!
//! Each agent writes its own format (`Format`). Where an agent does not hand
//! a server its whole environment (Codex lets through a short list; omp is
//! not documented to), the entry passes on `TERMINAL_ENV` and `DECK_ENV`, so
//! `current_terminal` still knows which terminal the agent runs in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::tools::SERVER_NAME;
use super::{DECK_ENV, TERMINAL_ENV};
use crate::model::write_file_atomic;

/// How an agent's user-level config lists servers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    /// JSON, servers by name under `key`; `typed`: entries carry "type": "stdio".
    Json { key: &'static str, typed: bool },
    /// oh-my-pi: JSON `mcpServers`, typed; our variables passed on by name in `env`.
    Omp,
    /// OpenCode (v2): JSON `mcp.servers`, "type": "local", the command and its
    /// arguments one array; our variables passed on with `{env:NAME}`.
    OpenCode,
    /// Codex: TOML `[mcp_servers.<name>]`; our variables let through with `env_vars`.
    Codex,
}

/// An agent whose user-level config can list our server.
#[derive(Clone, Debug)]
pub struct Agent {
    pub id: &'static str,
    pub label: &'static str,
    /// Its user-level config.
    pub path: PathBuf,
    /// What tells the agent is installed: a file or folder of it.
    pub home: PathBuf,
    pub format: Format,
}

/// Where each agent keeps its config. `xdg` is `$XDG_CONFIG_HOME` or
/// `~/.config` (OpenCode keeps to it on macOS too), `codex` is `$CODEX_HOME`
/// or `~/.codex`, `omp` the agent folder (`$PI_CODING_AGENT_DIR` or `~/.omp/agent`).
pub struct Dirs {
    pub home: PathBuf,
    pub config: PathBuf,
    pub xdg: PathBuf,
    pub codex: PathBuf,
    pub omp: PathBuf,
}

impl Dirs {
    pub fn of(home: &Path, config: &Path) -> Dirs {
        Dirs {
            home: home.to_path_buf(),
            config: config.to_path_buf(),
            xdg: home.join(".config"),
            codex: home.join(".codex"),
            omp: home.join(".omp").join("agent"),
        }
    }
}

/// OpenCode reads opencode.json or opencode.jsonc: the one there, .json when neither is.
fn opencode_file(dir: &Path) -> PathBuf {
    let (json, jsonc) = (dir.join("opencode.json"), dir.join("opencode.jsonc"));
    if !json.exists() && jsonc.exists() { jsonc } else { json }
}

pub fn agents_in(d: &Dirs) -> Vec<Agent> {
    let desktop = d.config.join("Claude");
    let vscode = d.config.join("Code").join("User");
    let opencode = d.xdg.join("opencode");
    let json = |key, typed| Format::Json { key, typed };
    vec![
        Agent { id: "claude-code", label: "Claude Code", path: d.home.join(".claude.json"), home: d.home.join(".claude"), format: json("mcpServers", true) },
        Agent { id: "claude-desktop", label: "Claude 桌面版", path: desktop.join("claude_desktop_config.json"), home: desktop, format: json("mcpServers", false) },
        Agent { id: "cursor", label: "Cursor", path: d.home.join(".cursor").join("mcp.json"), home: d.home.join(".cursor"), format: json("mcpServers", false) },
        Agent { id: "vscode", label: "VS Code", path: vscode.join("mcp.json"), home: vscode, format: json("servers", true) },
        Agent { id: "codex", label: "Codex", path: d.codex.join("config.toml"), home: d.codex.clone(), format: Format::Codex },
        Agent { id: "opencode", label: "OpenCode", path: opencode_file(&opencode), home: opencode, format: Format::OpenCode },
        Agent { id: "omp", label: "oh-my-pi (omp)", path: d.omp.join("mcp.json"), home: d.omp.clone(), format: Format::Omp },
    ]
}

pub fn user_agents() -> Vec<Agent> {
    let (Some(home), Some(config)) = (dirs::home_dir(), dirs::config_dir()) else { return Vec::new() };
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let mut d = Dirs::of(&home, &config);
    if let Some(x) = var("XDG_CONFIG_HOME") {
        d.xdg = x;
    }
    if let Some(c) = var("CODEX_HOME") {
        d.codex = c;
    }
    if let Some(o) = var("PI_CODING_AGENT_DIR") {
        d.omp = o;
    }
    agents_in(&d)
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

/// The servers at a path of keys (`mcpServers`, or `mcp` then `servers`).
fn servers_at(top: &Map<String, Value>, path: &[&str]) -> Result<Map<String, Value>, String> {
    match path {
        [key] => servers_of(top, key),
        [key, rest @ ..] => match top.get(*key) {
            None => Ok(Map::new()),
            Some(Value::Object(m)) => servers_at(m, rest),
            Some(_) => Err(format!("{key} 不是对象")),
        },
        [] => Ok(top.clone()),
    }
}

/// Puts the servers back at their path, keeping the other keys on the way.
fn set_servers_at(top: &mut Map<String, Value>, path: &[&str], servers: Map<String, Value>) {
    match path {
        [key] => {
            top.insert((*key).into(), Value::Object(servers));
        }
        [key, rest @ ..] => {
            let mut inner = match top.get(*key) {
                Some(Value::Object(m)) => m.clone(),
                _ => Map::new(),
            };
            set_servers_at(&mut inner, rest, servers);
            top.insert((*key).into(), Value::Object(inner));
        }
        [] => *top = servers,
    }
}

impl Format {
    /// Where its servers are in the JSON, for the JSON formats.
    fn json_path(self) -> Vec<&'static str> {
        match self {
            Format::Json { key, .. } => vec![key],
            Format::Omp => vec!["mcpServers"],
            Format::OpenCode => vec!["mcp", "servers"],
            Format::Codex => vec![],
        }
    }

    /// The `$schema` a new file of it starts with.
    fn schema(self) -> Option<&'static str> {
        match self {
            Format::OpenCode => Some("https://opencode.ai/config.json"),
            Format::Omp => Some("https://raw.githubusercontent.com/can1357/oh-my-pi/main/packages/coding-agent/src/config/mcp-schema.json"),
            _ => None,
        }
    }

    /// Our entry, for the JSON formats.
    fn entry(self, exe: &str) -> Value {
        match self {
            Format::Json { typed, .. } => {
                let mut e = json!({"command": exe, "args": ["mcp-stdio"]});
                if typed {
                    e["type"] = "stdio".into();
                }
                e
            }
            Format::Omp => json!({
                "type": "stdio", "command": exe, "args": ["mcp-stdio"],
                // omp takes a value that names a set variable as that variable's value.
                "env": {TERMINAL_ENV: TERMINAL_ENV, DECK_ENV: DECK_ENV},
            }),
            Format::OpenCode => json!({
                "type": "local", "command": [exe, "mcp-stdio"],
                "environment": {TERMINAL_ENV: format!("{{env:{TERMINAL_ENV}}}"), DECK_ENV: format!("{{env:{DECK_ENV}}}")},
            }),
            Format::Codex => Value::Null,
        }
    }

    /// The command an entry runs, and whether it is ours run by exe.
    fn entry_command(self, e: &Value, exe: &str) -> (String, bool) {
        let strs = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
        };
        if self == Format::OpenCode {
            let cmd = strs(e.get("command"));
            let first = cmd.first().cloned().unwrap_or_default();
            return (first, cmd == [exe, "mcp-stdio"]);
        }
        let cmd = e.get("command").and_then(Value::as_str).unwrap_or("").to_string();
        let ours = cmd == exe && strs(e.get("args")) == ["mcp-stdio"];
        (cmd, ours)
    }
}

impl Agent {
    pub fn status(&self, exe: &str) -> State {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return if self.home.exists() { State::NotThere } else { State::Absent };
            }
            Err(e) => return State::Broken(e.to_string()),
        };
        if self.format == Format::Codex {
            return codex_status(&data, exe);
        }
        let Some(top) = read_object(&data) else { return State::Broken(self.unreadable()) };
        let servers = match servers_at(&top, &self.format.json_path()) {
            Ok(s) => s,
            Err(e) => return State::Broken(e),
        };
        let Some(e) = servers.get(SERVER_NAME) else { return State::NotThere };
        match self.format.entry_command(e, exe) {
            (_, true) => State::Installed,
            (cmd, false) => State::Elsewhere(cmd),
        }
    }

    fn unreadable(&self) -> String {
        if self.path.extension().is_some_and(|e| e == "jsonc") {
            "含注释的 JSONC，读不了也不改动它；请手动加入".into()
        } else {
            "不是 JSON 对象".into()
        }
    }

    pub fn install(&self, exe: &str) -> Result<(), String> {
        if !self.home.exists() {
            return Err(format!("没有找到 {}", self.label));
        }
        if self.format == Format::Codex {
            return edit_codex(&self.path, |servers| {
                servers.insert(SERVER_NAME, codex_entry(exe));
                true
            });
        }
        let entry = self.format.entry(exe);
        edit_user_servers(&self.path, self.format, |s| {
            s.insert(SERVER_NAME.into(), entry);
            true
        })
    }

    pub fn uninstall(&self) -> Result<(), String> {
        if self.format == Format::Codex {
            return edit_codex(&self.path, |servers| servers.remove(SERVER_NAME).is_some());
        }
        edit_user_servers(&self.path, self.format, |s| s.remove(SERVER_NAME).is_some())
    }
}

/// Edits the servers of a user-level JSON config, keeping everything else in
/// it, and its mode; a config that will not parse is left as it is.
fn edit_user_servers(path: &Path, format: Format, edit: impl FnOnce(&mut Map<String, Value>) -> bool) -> Result<(), String> {
    let (mut top, mode) = match std::fs::read(path) {
        Ok(data) => {
            let top = read_object(&data).ok_or("配置文件不是 JSON 对象（或含注释），没有改动它")?;
            (top, std::fs::metadata(path).ok().map(|m| m.permissions()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut top = Map::new();
            if let Some(s) = format.schema() {
                top.insert("$schema".into(), s.into());
            }
            (top, None)
        }
        Err(e) => return Err(e.to_string()),
    };
    let at = format.json_path();
    let mut servers = servers_at(&top, &at).map_err(|e| format!("{e}，没有改动它"))?;
    if !edit(&mut servers) {
        return Ok(());
    }
    set_servers_at(&mut top, &at, servers);
    let data = serde_json::to_vec_pretty(&Value::Object(top)).map_err(|e| e.to_string())?;
    write_file_atomic(path, &data).map_err(|e| e.to_string())?;
    if let Some(m) = mode {
        let _ = std::fs::set_permissions(path, m);
    }
    Ok(())
}

// ---- Codex: TOML ----

/// Our `[mcp_servers.boundlessdeck]`: Codex hands a server only a few
/// variables of its own, so ours are named in `env_vars`.
fn codex_entry(exe: &str) -> toml_edit::Item {
    let mut t = toml_edit::Table::new();
    t["command"] = toml_edit::value(exe);
    t["args"] = toml_edit::value(toml_edit::Array::from_iter(["mcp-stdio"]));
    t["env_vars"] = toml_edit::value(toml_edit::Array::from_iter([TERMINAL_ENV, DECK_ENV]));
    toml_edit::Item::Table(t)
}

fn codex_status(data: &[u8], exe: &str) -> State {
    let Ok(text) = std::str::from_utf8(data) else { return State::Broken("不是 UTF-8".into()) };
    let doc = match text.parse::<toml_edit::DocumentMut>() {
        Ok(d) => d,
        Err(e) => return State::Broken(format!("TOML 读不懂：{}", e.message())),
    };
    let Some(e) = doc.get("mcp_servers").and_then(|s| s.get(SERVER_NAME)) else { return State::NotThere };
    let cmd = e.get("command").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let args: Vec<&str> =
        e.get("args").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    if cmd == exe && args == ["mcp-stdio"] { State::Installed } else { State::Elsewhere(cmd) }
}

/// Edits `[mcp_servers]` of Codex's config.toml, keeping the rest as written
/// (comments, order, layout); a file that will not parse is left alone.
fn edit_codex(path: &Path, edit: impl FnOnce(&mut toml_edit::Table) -> bool) -> Result<(), String> {
    let (text, mode) = match std::fs::read_to_string(path) {
        Ok(t) => (t, std::fs::metadata(path).ok().map(|m| m.permissions())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
        Err(e) => return Err(e.to_string()),
    };
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|e| format!("TOML 读不懂，没有改动它：{}", e.message()))?;
    let servers = doc.entry("mcp_servers").or_insert_with(|| {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        toml_edit::Item::Table(t)
    });
    let Some(servers) = servers.as_table_mut() else { return Err("mcp_servers 不是表，没有改动它".into()) };
    if !edit(servers) {
        return Ok(());
    }
    write_file_atomic(path, doc.to_string().as_bytes()).map_err(|e| e.to_string())?;
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
    /// Whether the welcome window has been shown: it opens by itself only once.
    #[serde(default)]
    pub welcomed: bool,
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
        let ags = agents_in(&Dirs::of(home.path(), config.path()));
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
    fn codex_opencode_and_omp_get_their_own_formats() {
        let home = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let ags = agents_in(&Dirs::of(home.path(), config.path()));
        let get = |id: &str| ags.iter().find(|a| a.id == id).unwrap().clone();

        // Codex: TOML, the user's comments and servers kept, our variables let through.
        let codex = get("codex");
        assert_eq!(codex.status("/app"), State::Absent);
        std::fs::create_dir_all(&codex.home).unwrap();
        std::fs::write(&codex.path, "# mine\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"x\"\n").unwrap();
        assert_eq!(codex.status("/app"), State::NotThere);
        codex.install("/app").unwrap();
        assert_eq!(codex.status("/app"), State::Installed);
        let text = std::fs::read_to_string(&codex.path).unwrap();
        assert!(text.starts_with("# mine\nmodel = \"o3\""), "the rest is as written: {text}");
        assert!(text.contains("[mcp_servers.other]") && text.contains("[mcp_servers.boundlessdeck]"));
        assert!(text.contains("BOUNDLESSDECK_TERMINAL") && text.contains("BOUNDLESSDECK_DECK"));
        codex.uninstall().unwrap();
        assert_eq!(codex.status("/app"), State::NotThere);
        assert!(std::fs::read_to_string(&codex.path).unwrap().contains("[mcp_servers.other]"));
        std::fs::write(&codex.path, "[oops").unwrap();
        assert!(matches!(codex.status("/app"), State::Broken(_)));
        assert!(codex.install("/app").is_err());

        // OpenCode: mcp.servers, a local command array, the other settings kept.
        let oc = get("opencode");
        std::fs::create_dir_all(&oc.home).unwrap();
        std::fs::write(&oc.path, r#"{"$schema": "https://opencode.ai/config.json", "provider": {"a": 1}, "mcp": {"timeout": {"startup": 5}}}"#).unwrap();
        oc.install("/app").unwrap();
        assert_eq!(oc.status("/app"), State::Installed);
        let v: Value = serde_json::from_slice(&std::fs::read(&oc.path).unwrap()).unwrap();
        assert_eq!(v["provider"]["a"], 1);
        assert_eq!(v["mcp"]["timeout"]["startup"], 5);
        let e = &v["mcp"]["servers"]["boundlessdeck"];
        assert_eq!(e["type"], "local");
        assert_eq!(e["command"], json!(["/app", "mcp-stdio"]));
        assert_eq!(e["environment"]["BOUNDLESSDECK_TERMINAL"], "{env:BOUNDLESSDECK_TERMINAL}");
        assert_eq!(oc.status("/other"), State::Elsewhere("/app".into()));
        oc.uninstall().unwrap();
        assert_eq!(oc.status("/app"), State::NotThere);
        // A JSONC file with comments is read by nobody here and left as it is.
        std::fs::remove_file(&oc.path).unwrap();
        std::fs::write(oc.home.join("opencode.jsonc"), "{ // mine\n}").unwrap();
        let oc = agents_in(&Dirs::of(home.path(), config.path())).into_iter().find(|a| a.id == "opencode").unwrap();
        assert!(oc.path.ends_with("opencode.jsonc"));
        assert!(matches!(oc.status("/app"), State::Broken(w) if w.contains("JSONC")));
        assert!(oc.install("/app").is_err());

        // omp: a new file gets its $schema; our variables passed on by name.
        let omp = get("omp");
        std::fs::create_dir_all(&omp.home).unwrap();
        omp.install("/app").unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&omp.path).unwrap()).unwrap();
        assert!(v["$schema"].as_str().unwrap().contains("mcp-schema.json"));
        assert_eq!(v["mcpServers"]["boundlessdeck"]["type"], "stdio");
        assert_eq!(v["mcpServers"]["boundlessdeck"]["env"]["BOUNDLESSDECK_DECK"], "BOUNDLESSDECK_DECK");
        assert_eq!(omp.status("/app"), State::Installed);
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
