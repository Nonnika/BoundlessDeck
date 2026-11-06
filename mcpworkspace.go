package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// Every workspace keeps a descriptor: .boundlessdeck/mcp.json says how to
// reach this deck's canvas. When the user asks for it (设置 → 项目级配置,
// off by default: the folder may be someone's repository), the same server
// is merged into the files the common agents read at startup (.mcp.json for
// Claude Code, .cursor/mcp.json, .vscode/mcp.json), so an agent started in
// the folder loads it with no other setup. Otherwise agents reach the deck
// through the global install (mcpglobal.go), which follows the deck open.
const mcpWorkspaceVersion = 1

// mcpWorkspace is .boundlessdeck/mcp.json.
type mcpWorkspace struct {
	Version       int            `json:"version"`
	Generator     string         `json:"generator"`
	Workspace     string         `json:"workspace"`
	Server        mcpEndpoint    `json:"server"`
	SchemaFile    string         `json:"schemaFile"`
	SchemaVersion int            `json:"schemaVersion"`
	Servers       []mcpServerRef `json:"servers"`
	UpdatedAt     time.Time      `json:"updatedAt"`
	Warnings      []string       `json:"warnings,omitempty"`
}

// mcpEndpoint is how to reach a deck's MCP: its Unix socket, the token it
// wants, and the stdio command a project config lists.
type mcpEndpoint struct {
	Name            string       `json:"name"`
	Transport       string       `json:"transport"`
	Socket          string       `json:"socket"`
	Token           string       `json:"token"`
	Bridge          mcpBridgeCmd `json:"bridge"`
	Tools           int          `json:"tools"`
	ToolNames       []string     `json:"toolNames"`
	ProtocolVersion string       `json:"protocolVersion"`
}

type mcpBridgeCmd struct {
	Command string   `json:"command"`
	Args    []string `json:"args"`
}

// mcpServerRef is one project-level MCP server the workspace declares,
// whether it is ours or one the user put in .mcp.json.
type mcpServerRef struct {
	Name      string   `json:"name"`
	Source    string   `json:"source"`
	Transport string   `json:"transport,omitempty"`
	Command   string   `json:"command,omitempty"`
	Args      []string `json:"args,omitempty"`
	URL       string   `json:"url,omitempty"`
	Own       bool     `json:"own,omitempty"`
}

// mcpProjectFile is one agent's project-scoped MCP config: the file, and
// the key under which it keeps its servers.
type mcpProjectFile struct {
	Path  string
	Key   string
	Label string
}

// mcpProjectFiles are the project-scoped configs we keep our server in.
func mcpProjectFiles(root string) []mcpProjectFile {
	return []mcpProjectFile{
		{Path: filepath.Join(root, ".mcp.json"), Key: "mcpServers", Label: ".mcp.json"},
		{Path: filepath.Join(root, ".cursor", "mcp.json"), Key: "mcpServers", Label: ".cursor/mcp.json"},
		{Path: filepath.Join(root, ".vscode", "mcp.json"), Key: "servers", Label: ".vscode/mcp.json"},
	}
}

// startMCP makes the workspace's endpoint, writes its descriptor (and its
// project-level configs, when d.projectMCP asks for them) and makes it the
// active deck the global bridge follows. Opened again, every deck starts it
// over.
func (d *deck) startMCP() {
	if d.root == "" {
		return
	}
	d.stopMCP()
	sock, err := mcpSocketPath(d.root)
	if err != nil {
		d.mcpErr = err
		return
	}
	s, err := startMCPForDeck(d, sock)
	if err != nil {
		d.mcpErr = err
		return
	}
	d.mcp = s
	exe, err := os.Executable()
	if err != nil || exe == "" {
		exe = "boundlessdeck"
	}
	names := make([]string, len(mcpToolList))
	for i, t := range mcpToolList {
		names[i] = t.Name
	}
	ep := mcpEndpoint{
		Name:            mcpServerName,
		Transport:       s.transport,
		Socket:          s.sock,
		Token:           s.token,
		Bridge:          mcpBridgeCmd{Command: exe, Args: mcpBridgeCommand(exe, d.root)},
		Tools:           len(mcpToolList),
		ToolNames:       names,
		ProtocolVersion: mcpProtocolVersion,
	}
	ws, err := writeMCPWorkspace(d.root, ep, d.projectMCP)
	if err != nil {
		d.mcpErr = fmt.Errorf("写 MCP 配置：%w", err)
		return
	}
	if err := writeMCPActive(d.root, ep); err != nil {
		ws.Warnings = append(ws.Warnings, "全局 MCP 找不到这个图版："+err.Error())
	}
	// Load it back, with every server the project configs declare: the
	// workspace's project-level MCPs are ready as soon as it opens.
	if loaded, err := loadMCPWorkspace(d.root); err == nil {
		ws = loaded
	} else {
		ws.Warnings = append(ws.Warnings, err.Error())
	}
	d.mcpProject = ws
}

// stopMCP closes the endpoint; the workspace keeps the configs it wrote,
// and the global bridge has no deck to follow until another opens.
func (d *deck) stopMCP() {
	if d.mcp == nil {
		return
	}
	clearMCPActive(d.root)
	d.mcp.close()
	d.mcp = nil
	d.mcpProject = nil
}

// writeMCPWorkspace writes the block schema, this workspace's descriptor
// and, with project, our entry in every project-scoped config, keeping
// what the user put in those files.
func writeMCPWorkspace(root string, ep mcpEndpoint, project bool) (*mcpWorkspace, error) {
	dir := filepath.Join(root, deckDir)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return nil, err
	}
	if err := writeJSONAtomic(filepath.Join(dir, "block-schema.json"), mcpSchemaDigest()); err != nil {
		return nil, err
	}
	servers, warnings := collectProjectServers(root, ep.Name)
	servers = dedupeServers(servers)
	own := mcpServerRef{
		Name:      ep.Name,
		Source:    filepath.Join(deckDir, "mcp.json"),
		Transport: ep.Transport,
		Own:       true,
	}
	ws := &mcpWorkspace{
		Version:       mcpWorkspaceVersion,
		Generator:     "BoundlessDeck",
		Workspace:     root,
		Server:        ep,
		SchemaFile:    filepath.Join(deckDir, "block-schema.json"),
		SchemaVersion: mcpSchemaVersion,
		Servers:       append([]mcpServerRef{own}, servers...),
		UpdatedAt:     mcpNow(),
		Warnings:      warnings,
	}
	descriptor := filepath.Join(dir, "mcp.json")
	if err := writeJSONAtomic(descriptor, ws); err != nil {
		return nil, err
	}
	// The token is a local secret: keep the descriptor out of other users'
	// reach, as the socket already is.
	_ = os.Chmod(descriptor, 0o600)
	if project {
		ws.Warnings = append(ws.Warnings, writeProjectConfigs(root, ep)...)
	}
	return ws, nil
}

// writeProjectConfigs merges our entry into every project-scoped config of
// root, saying what failed.
func writeProjectConfigs(root string, ep mcpEndpoint) []string {
	entry := map[string]any{
		"type":    "stdio",
		"command": ep.Bridge.Command,
		"args":    ep.Bridge.Args,
	}
	var warnings []string
	for _, f := range mcpProjectFiles(root) {
		if err := mergeMCPServer(f.Path, f.Key, ep.Name, entry); err != nil {
			warnings = append(warnings, fmt.Sprintf("%s：%v", f.Label, err))
		}
	}
	return warnings
}

// setProjectMCP turns the project-level configs of the deck open on or
// off: on, they are written now; off, they are taken out of its folder.
func (d *deck) setProjectMCP(on bool) error {
	d.projectMCP = on
	if d.root == "" {
		return nil
	}
	var err error
	if on {
		if d.mcpProject != nil {
			if w := writeProjectConfigs(d.root, d.mcpProject.Server); len(w) > 0 {
				err = errors.New(strings.Join(w, "；"))
			}
		}
	} else {
		err = removeProjectConfigs(d.root)
	}
	if ws, lerr := loadMCPWorkspace(d.root); lerr == nil && d.mcp != nil {
		d.mcpProject = ws
	}
	return err
}

// loadMCPWorkspace reads what a workspace declares: our descriptor, and the
// servers its project configs list. It is how an opened workspace knows its
// project-level MCPs before anything connects.
func loadMCPWorkspace(root string) (*mcpWorkspace, error) {
	ws := &mcpWorkspace{Version: mcpWorkspaceVersion, Workspace: root, Servers: []mcpServerRef{}}
	data, err := os.ReadFile(filepath.Join(root, deckDir, "mcp.json"))
	switch {
	case err == nil:
		if err := json.Unmarshal(data, ws); err != nil {
			return ws, fmt.Errorf("解不开 %s：%w", filepath.Join(deckDir, "mcp.json"), err)
		}
	case errors.Is(err, os.ErrNotExist):
		// No descriptor yet: a fresh workspace still lists what its
		// project configs declare.
	default:
		return ws, err
	}
	ws.Servers = dedupeServers(ws.Servers)
	seen := map[string]bool{}
	for _, s := range ws.Servers {
		seen[s.Name] = true
	}
	extra, warnings := collectProjectServers(root, mcpServerName)
	for _, s := range extra {
		if !seen[s.Name] {
			ws.Servers = append(ws.Servers, s)
			seen[s.Name] = true
		}
	}
	ws.Warnings = append(ws.Warnings, warnings...)
	return ws, nil
}

// dedupeServers keeps the first entry of each name: one server listed in
// two project configs is still one server.
func dedupeServers(servers []mcpServerRef) []mcpServerRef {
	out := make([]mcpServerRef, 0, len(servers))
	seen := map[string]bool{}
	for _, s := range servers {
		if seen[s.Name] {
			continue
		}
		seen[s.Name] = true
		out = append(out, s)
	}
	return out
}

// collectProjectServers reads the project-scoped configs and describes the
// servers in them, but ours.
func collectProjectServers(root, own string) ([]mcpServerRef, []string) {
	var out []mcpServerRef
	var warnings []string
	for _, f := range mcpProjectFiles(root) {
		refs, err := readMCPServersFile(f.Path, f.Key, f.Label, own)
		if err != nil {
			warnings = append(warnings, fmt.Sprintf("%s：%v", f.Label, err))
			continue
		}
		out = append(out, refs...)
	}
	return out, warnings
}

// readMCPServersFile describes the servers one config file declares.
func readMCPServersFile(path, key, label, own string) ([]mcpServerRef, error) {
	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var top map[string]json.RawMessage
	if err := json.Unmarshal(data, &top); err != nil {
		return nil, fmt.Errorf("不是 JSON 对象")
	}
	raw, ok := top[key]
	if !ok {
		return nil, nil
	}
	var servers map[string]json.RawMessage
	if err := json.Unmarshal(raw, &servers); err != nil {
		return nil, fmt.Errorf("%s 不是对象", key)
	}
	names := make([]string, 0, len(servers))
	for name := range servers {
		names = append(names, name)
	}
	sort.Strings(names)
	out := make([]mcpServerRef, 0, len(names))
	for _, name := range names {
		if name == own {
			continue
		}
		var e struct {
			Type    string   `json:"type"`
			Command string   `json:"command"`
			Args    []string `json:"args"`
			URL     string   `json:"url"`
		}
		_ = json.Unmarshal(servers[name], &e)
		ref := mcpServerRef{
			Name: name, Source: label, Transport: e.Type,
			Command: e.Command, Args: e.Args, URL: e.URL,
		}
		if ref.Transport == "" {
			if e.URL != "" {
				ref.Transport = "http"
			} else {
				ref.Transport = "stdio"
			}
		}
		out = append(out, ref)
	}
	return out, nil
}

// mergeMCPServer puts our entry under name in a project config, keeping
// every other key and server in it. A config that will not parse is kept
// aside as .bak and replaced, so a broken file never blocks the workspace.
func mergeMCPServer(path, key, name string, entry map[string]any) error {
	top := map[string]json.RawMessage{}
	if data, err := os.ReadFile(path); err == nil {
		if err := json.Unmarshal(data, &top); err != nil || top == nil {
			_ = writeFileAtomic(path+".bak", data)
			top = map[string]json.RawMessage{}
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	servers := map[string]json.RawMessage{}
	if raw, ok := top[key]; ok {
		if err := json.Unmarshal(raw, &servers); err != nil || servers == nil {
			servers = map[string]json.RawMessage{}
		}
	}
	encoded, err := json.Marshal(entry)
	if err != nil {
		return err
	}
	servers[name] = encoded
	encoded, err = json.Marshal(servers)
	if err != nil {
		return err
	}
	top[key] = encoded
	return writeJSONAtomic(path, top)
}

// writeJSONAtomic writes v as indented JSON, a temporary file then a
// rename.
func writeJSONAtomic(path string, v any) error {
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	return writeFileAtomic(path, data)
}

// readMCPEndpoint is what the stdio bridge reads to find its deck.
func readMCPEndpoint(workspace string) (mcpEndpoint, error) {
	path := filepath.Join(workspace, deckDir, "mcp.json")
	data, err := os.ReadFile(path)
	if err != nil {
		return mcpEndpoint{}, fmt.Errorf("读 %s：%w", path, err)
	}
	var ws mcpWorkspace
	if err := json.Unmarshal(data, &ws); err != nil {
		return mcpEndpoint{}, fmt.Errorf("解不开 %s：%w", path, err)
	}
	if ws.Server.Socket == "" {
		return mcpEndpoint{}, fmt.Errorf("%s 里没有 server.socket", path)
	}
	if ws.Server.Transport == "" {
		ws.Server.Transport = "unix"
	}
	return ws.Server, nil
}

// mcpProjectSummary is the one-line state the status bar shows.
func mcpProjectSummary(ws *mcpWorkspace, project bool) string {
	if ws == nil {
		return ""
	}
	if !project {
		return "通过全局 MCP 连接（设置 → MCP）"
	}
	var extra []string
	for _, s := range ws.Servers {
		if !s.Own {
			extra = append(extra, s.Name)
		}
	}
	if len(extra) == 0 {
		return "项目级 MCP 已加载"
	}
	return "项目级 MCP 已加载：" + strings.Join(extra, "、")
}
