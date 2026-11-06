package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	goruntime "runtime"
	"strings"
	"time"
)

// The global MCP: an agent started in any folder reaches the deck open in
// the app, with no config in that folder. The app keeps where its open
// deck's endpoint is in one file of the user's (the active-deck registry);
// a bridge started without --workspace follows it, request by request, so
// switching decks needs no restart. The settings window installs that
// bridge into the agents' user-level configs (mcpAgents).

// mcpActive is the active-deck registry: the deck the app has open.
type mcpActive struct {
	Workspace string      `json:"workspace"`
	Server    mcpEndpoint `json:"server"`
	PID       int         `json:"pid"`
	UpdatedAt time.Time   `json:"updatedAt"`
}

// mcpActiveOverride is where tests keep the registry, so they leave the
// user's alone.
var mcpActiveOverride string

// mcpActivePath is the registry's file. It does not go through MyGo's
// user-data dir: that is named after the app, which a dev build and the
// packaged app may name apart, and both sides must find the same file.
func mcpActivePath() string {
	if mcpActiveOverride != "" {
		return mcpActiveOverride
	}
	dir, err := os.UserConfigDir()
	if err != nil {
		return ""
	}
	return filepath.Join(dir, "BoundlessDeck", "active-deck.json")
}

// writeMCPActive records the deck open as the active one.
func writeMCPActive(root string, ep mcpEndpoint) error {
	path := mcpActivePath()
	if path == "" {
		return errors.New("找不到用户配置目录")
	}
	data, err := json.MarshalIndent(mcpActive{
		Workspace: root, Server: ep, PID: os.Getpid(), UpdatedAt: mcpNow(),
	}, "", "  ")
	if err != nil {
		return err
	}
	// It holds the token: only the user reads it.
	return writeFileAtomicMode(path, data, 0o600)
}

// clearMCPActive forgets the active deck, if it is still root's: another
// instance of the app may have opened a deck since.
func clearMCPActive(root string) {
	path := mcpActivePath()
	if path == "" {
		return
	}
	if a, err := readMCPActive(); err == nil && a.Workspace != root {
		return
	}
	_ = os.Remove(path)
}

// readMCPActive reads the registry.
func readMCPActive() (mcpActive, error) {
	var a mcpActive
	path := mcpActivePath()
	if path == "" {
		return a, errors.New("找不到用户配置目录")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return a, err
	}
	if err := json.Unmarshal(data, &a); err != nil {
		return a, fmt.Errorf("解不开 %s：%w", path, err)
	}
	if a.Server.Socket == "" {
		return a, fmt.Errorf("%s 里没有 server.socket", path)
	}
	if a.Server.Transport == "" {
		a.Server.Transport = "unix"
	}
	return a, nil
}

// errNoActiveDeck is what an agent sees through the global bridge while
// the app has no deck open.
var errNoActiveDeck = errors.New("BoundlessDeck 没有打开的图版：先在应用里打开一个文件夹，再重试")

// activeEndpoint is the endpoint a global bridge talks to now.
func activeEndpoint() (mcpEndpoint, error) {
	a, err := readMCPActive()
	if errors.Is(err, os.ErrNotExist) {
		return mcpEndpoint{}, errNoActiveDeck
	}
	if err != nil {
		return mcpEndpoint{}, err
	}
	return a.Server, nil
}

// mcpAgent is an agent whose user-level config can list our server.
type mcpAgent struct {
	ID    string
	Label string
	// Path is its user-level config; Key the object of servers in it.
	Path string
	Key  string
	// Home is what tells the agent is installed: a file or folder of it.
	Home string
	// typed says its entries carry "type": "stdio".
	typed bool
}

// mcpAgents are the agents the settings window installs into, on this
// platform, under home (the user's home folder) and config (the user's
// config folder, as os.UserConfigDir has it).
func mcpAgents(home, config string) []mcpAgent {
	claudeDesktop := filepath.Join(config, "Claude")
	vscode := filepath.Join(config, "Code", "User")
	return []mcpAgent{
		{ID: "claude-code", Label: "Claude Code", Path: filepath.Join(home, ".claude.json"), Key: "mcpServers",
			Home: filepath.Join(home, ".claude"), typed: true},
		{ID: "claude-desktop", Label: "Claude 桌面版", Path: filepath.Join(claudeDesktop, "claude_desktop_config.json"), Key: "mcpServers",
			Home: claudeDesktop},
		{ID: "cursor", Label: "Cursor", Path: filepath.Join(home, ".cursor", "mcp.json"), Key: "mcpServers",
			Home: filepath.Join(home, ".cursor")},
		{ID: "vscode", Label: "VS Code", Path: filepath.Join(vscode, "mcp.json"), Key: "servers",
			Home: vscode, typed: true},
	}
}

// userMCPAgents are the clients of the user running the app.
func userMCPAgents() []mcpAgent {
	home, _ := os.UserHomeDir()
	config, _ := os.UserConfigDir()
	if home == "" || config == "" {
		return nil
	}
	return mcpAgents(home, config)
}

// globalEntry is our server as the client's config lists it: the bridge,
// with no workspace, so it follows the deck open.
func (ag mcpAgent) globalEntry(exe string) map[string]any {
	e := map[string]any{"command": exe, "args": []string{"mcp-stdio"}}
	if ag.typed {
		e["type"] = "stdio"
	}
	return e
}

// mcpInstallState is where a client stands with our server.
type mcpInstallState int

const (
	mcpAbsent    mcpInstallState = iota // the agent is not on this computer
	mcpNotThere                         // the agent is, our server is not in its config
	mcpInstalled                        // our server is, pointing at this app
	mcpElsewhere                        // our server is, pointing at another copy of the app
	mcpBroken                           // its config will not read
)

// mcpAgentStatus is one client's row in the settings window.
type mcpAgentStatus struct {
	State   mcpInstallState
	Command string // the command its entry runs, when there is one
	Err     error
}

// status reads where the client stands, for the app at exe.
func (ag mcpAgent) status(exe string) mcpAgentStatus {
	data, err := os.ReadFile(ag.Path)
	if errors.Is(err, os.ErrNotExist) {
		if _, err := os.Stat(ag.Home); err != nil {
			return mcpAgentStatus{State: mcpAbsent}
		}
		return mcpAgentStatus{State: mcpNotThere}
	}
	if err != nil {
		return mcpAgentStatus{State: mcpBroken, Err: err}
	}
	servers, err := readServersObject(data, ag.Key)
	if err != nil {
		return mcpAgentStatus{State: mcpBroken, Err: err}
	}
	raw, ok := servers[mcpServerName]
	if !ok {
		return mcpAgentStatus{State: mcpNotThere}
	}
	var e struct {
		Command string   `json:"command"`
		Args    []string `json:"args"`
	}
	_ = json.Unmarshal(raw, &e)
	st := mcpAgentStatus{State: mcpInstalled, Command: e.Command}
	if e.Command != exe || len(e.Args) != 1 || e.Args[0] != "mcp-stdio" {
		st.State = mcpElsewhere
	}
	return st
}

// readServersObject is the object of servers under key, empty when the
// file has none. A file that is not a JSON object is an error: a user's
// config is never replaced.
func readServersObject(data []byte, key string) (map[string]json.RawMessage, error) {
	var top map[string]json.RawMessage
	if err := json.Unmarshal(data, &top); err != nil || top == nil {
		return nil, errors.New("不是 JSON 对象")
	}
	servers := map[string]json.RawMessage{}
	if raw, ok := top[key]; ok {
		if err := json.Unmarshal(raw, &servers); err != nil || servers == nil {
			return nil, fmt.Errorf("%s 不是对象", key)
		}
	}
	return servers, nil
}

// install puts our server in the client's config, or points it at exe.
func (ag mcpAgent) install(exe string) error {
	if _, err := os.Stat(ag.Home); err != nil {
		return fmt.Errorf("没有找到 %s", ag.Label)
	}
	encoded, err := json.Marshal(ag.globalEntry(exe))
	if err != nil {
		return err
	}
	return editUserServers(ag.Path, ag.Key, func(servers map[string]json.RawMessage) bool {
		servers[mcpServerName] = encoded
		return true
	})
}

// uninstall takes our server out of the client's config.
func (ag mcpAgent) uninstall() error {
	return editUserServers(ag.Path, ag.Key, func(servers map[string]json.RawMessage) bool {
		if _, ok := servers[mcpServerName]; !ok {
			return false
		}
		delete(servers, mcpServerName)
		return true
	})
}

// editUserServers changes the servers of a user's config, keeping every
// other key in it and its permissions. Unlike a project config, one that
// will not read is left alone: it may be an agent's whole state.
func editUserServers(path, key string, edit func(map[string]json.RawMessage) bool) error {
	top := map[string]json.RawMessage{}
	mode := os.FileMode(0o644)
	data, err := os.ReadFile(path)
	switch {
	case err == nil:
		if info, err := os.Stat(path); err == nil {
			mode = info.Mode().Perm()
		}
		if err := json.Unmarshal(data, &top); err != nil || top == nil {
			return errors.New("配置文件不是 JSON 对象，没有改动它")
		}
	case errors.Is(err, os.ErrNotExist):
	default:
		return err
	}
	servers := map[string]json.RawMessage{}
	if raw, ok := top[key]; ok {
		if err := json.Unmarshal(raw, &servers); err != nil || servers == nil {
			return fmt.Errorf("%s 不是对象，没有改动它", key)
		}
	}
	if !edit(servers) {
		return nil
	}
	encoded, err := json.Marshal(servers)
	if err != nil {
		return err
	}
	top[key] = encoded
	out, err := json.MarshalIndent(top, "", "  ")
	if err != nil {
		return err
	}
	return writeFileAtomicMode(path, out, mode)
}

func mustJSON(v any) []byte {
	data, _ := json.Marshal(v)
	return data
}

// writeFileAtomicMode writes data to path with mode, a temporary file then
// a rename.
func writeFileAtomicMode(path string, data []byte, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, mode); err != nil {
		return err
	}
	_ = os.Chmod(tmp, mode)
	return os.Rename(tmp, path)
}

// mcpInstallExe is the command a global config runs: this app, when it
// lives somewhere that stays. A `go run`/`go test` build is gone once it
// exits, and a quarantined app runs from a random path macOS made up.
func mcpInstallExe() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	if real, err := filepath.EvalSymlinks(exe); err == nil {
		exe = real
	}
	return exe, stableExe(exe)
}

// stableExe says why exe cannot go in a global config, nil when it can.
func stableExe(exe string) error {
	slash := filepath.ToSlash(exe)
	switch {
	case strings.Contains(slash, "/go-build"):
		return errors.New("这是 go run / go test 的临时程序，退出就没了：请用 mygo dev 或打包后的应用安装")
	case goruntime.GOOS == "darwin" && strings.Contains(slash, "/AppTranslocation/"):
		return errors.New("macOS 正从临时位置运行这个应用：先把 BoundlessDeck 拖进「应用程序」文件夹再打开")
	}
	return nil
}

// projectFilesHaveUs says whether any project config of root lists our
// server.
func projectFilesHaveUs(root string) bool {
	for _, f := range mcpProjectFiles(root) {
		data, err := os.ReadFile(f.Path)
		if err != nil {
			continue
		}
		if servers, err := readServersObject(data, f.Key); err == nil {
			if _, ok := servers[mcpServerName]; ok {
				return true
			}
		}
	}
	return false
}

// removeProjectConfigs takes our server out of root's project configs. A
// file left with nothing but an empty list of servers was ours alone and
// goes, and so does a .cursor/.vscode folder it leaves empty.
func removeProjectConfigs(root string) error {
	var errs []error
	for _, f := range mcpProjectFiles(root) {
		data, err := os.ReadFile(f.Path)
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			errs = append(errs, fmt.Errorf("%s：%w", f.Label, err))
			continue
		}
		var top map[string]json.RawMessage
		if err := json.Unmarshal(data, &top); err != nil || top == nil {
			continue // not ours to touch
		}
		servers, err := readServersObject(data, f.Key)
		if err != nil {
			continue
		}
		if _, ok := servers[mcpServerName]; !ok {
			continue
		}
		delete(servers, mcpServerName)
		if len(servers) == 0 && len(top) == 1 {
			if err := os.Remove(f.Path); err != nil {
				errs = append(errs, fmt.Errorf("%s：%w", f.Label, err))
			}
			_ = os.Remove(f.Path + ".bak")
			if dir := filepath.Dir(f.Path); dir != root {
				_ = os.Remove(dir) // only when empty
			}
			continue
		}
		top[f.Key] = mustJSON(servers)
		if err := writeJSONAtomic(f.Path, top); err != nil {
			errs = append(errs, fmt.Errorf("%s：%w", f.Label, err))
		}
	}
	return errors.Join(errs...)
}
