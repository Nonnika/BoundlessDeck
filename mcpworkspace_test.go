package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func testEndpoint(root string) mcpEndpoint {
	return mcpEndpoint{
		Name:            mcpServerName,
		Transport:       "unix",
		Socket:          filepath.Join(root, deckDir, "mcp.sock"),
		Token:           "tok",
		Bridge:          mcpBridgeCmd{Command: "/Applications/BoundlessDeck", Args: []string{"mcp-stdio", "--workspace", root}},
		Tools:           3,
		ToolNames:       []string{"deck_info", "update_block", "create_block"},
		ProtocolVersion: mcpProtocolVersion,
	}
}

func readJSONFile(t *testing.T, path string) map[string]any {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("%s: %v", path, err)
	}
	var out map[string]any
	if err := json.Unmarshal(data, &out); err != nil {
		t.Fatalf("%s: %v", path, err)
	}
	return out
}

// TestWriteMCPWorkspaceMergesProjectConfigs: opening a workspace leaves a
// descriptor and our server in every project config, keeping what the user
// had there.
func TestWriteMCPWorkspaceMergesProjectConfigs(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, ".mcp.json"), []byte(`{
	  "mcpServers": {"github": {"command": "github-mcp", "args": ["serve"]}},
	  "theme": "dark"
	}`), 0o644); err != nil {
		t.Fatal(err)
	}
	ep := testEndpoint(root)
	ws, err := writeMCPWorkspace(root, ep, true)
	if err != nil {
		t.Fatal(err)
	}
	if ws.SchemaFile != filepath.Join(deckDir, "block-schema.json") {
		t.Fatalf("schemaFile = %q", ws.SchemaFile)
	}
	if len(ws.Servers) != 2 || !ws.Servers[0].Own || ws.Servers[0].Name != mcpServerName || ws.Servers[1].Name != "github" {
		t.Fatalf("servers = %+v", ws.Servers)
	}

	// The descriptor, and the schema it points at.
	desc := readJSONFile(t, filepath.Join(root, deckDir, "mcp.json"))
	if desc["workspace"] != root {
		t.Fatalf("descriptor workspace = %v", desc["workspace"])
	}
	server := desc["server"].(map[string]any)
	if server["socket"] != ep.Socket || server["transport"] != "unix" {
		t.Fatalf("descriptor server = %v", server)
	}
	bridge := server["bridge"].(map[string]any)
	if bridge["command"] != ep.Bridge.Command {
		t.Fatalf("bridge = %v", bridge)
	}
	schema := readJSONFile(t, filepath.Join(root, deckDir, "block-schema.json"))
	if kinds, _ := schema["kinds"].([]any); len(kinds) != len(blockKinds) {
		t.Fatalf("block-schema.json has %v kinds", schema["kinds"])
	}

	// Claude Code's config keeps the user's server and keys, and gains ours.
	claude := readJSONFile(t, filepath.Join(root, ".mcp.json"))
	if claude["theme"] != "dark" {
		t.Fatalf(".mcp.json lost its other keys: %v", claude)
	}
	servers := claude["mcpServers"].(map[string]any)
	if _, ok := servers["github"]; !ok {
		t.Fatalf(".mcp.json lost github: %v", servers)
	}
	ours := servers[mcpServerName].(map[string]any)
	if ours["command"] != ep.Bridge.Command || ours["type"] != "stdio" {
		t.Fatalf("our entry = %v", ours)
	}
	args := ours["args"].([]any)
	if len(args) != 3 || args[0] != "mcp-stdio" || args[1] != "--workspace" || args[2] != root {
		t.Fatalf("our args = %v", args)
	}

	// Cursor and VS Code get the same server under their own key.
	cursor := readJSONFile(t, filepath.Join(root, ".cursor", "mcp.json"))
	if _, ok := cursor["mcpServers"].(map[string]any)[mcpServerName]; !ok {
		t.Fatalf("cursor config = %v", cursor)
	}
	vscode := readJSONFile(t, filepath.Join(root, ".vscode", "mcp.json"))
	if _, ok := vscode["servers"].(map[string]any)[mcpServerName]; !ok {
		t.Fatalf("vscode config = %v", vscode)
	}

	// Writing again changes nothing but the timestamp.
	again, err := writeMCPWorkspace(root, ep, true)
	if err != nil {
		t.Fatal(err)
	}
	if len(again.Servers) != len(ws.Servers) {
		t.Fatalf("a second write duplicated servers: %+v", again.Servers)
	}
	claude = readJSONFile(t, filepath.Join(root, ".mcp.json"))
	if n := len(claude["mcpServers"].(map[string]any)); n != 2 {
		t.Fatalf(".mcp.json now has %d servers", n)
	}
}

func TestLoadMCPWorkspaceAddsExternalServers(t *testing.T) {
	root := t.TempDir()
	ep := testEndpoint(root)
	if _, err := writeMCPWorkspace(root, ep, true); err != nil {
		t.Fatal(err)
	}
	// A server the user adds to .mcp.json after we wrote it is loaded too.
	data := readJSONFile(t, filepath.Join(root, ".mcp.json"))
	servers := data["mcpServers"].(map[string]any)
	servers["notion"] = map[string]any{"type": "http", "url": "https://mcp.notion.example/mcp"}
	encoded, _ := json.Marshal(data)
	if err := os.WriteFile(filepath.Join(root, ".mcp.json"), encoded, 0o644); err != nil {
		t.Fatal(err)
	}

	ws, err := loadMCPWorkspace(root)
	if err != nil {
		t.Fatal(err)
	}
	names := map[string]mcpServerRef{}
	for _, s := range ws.Servers {
		names[s.Name] = s
	}
	if _, ok := names[mcpServerName]; !ok {
		t.Fatalf("own server missing from %+v", ws.Servers)
	}
	notion, ok := names["notion"]
	if !ok || notion.Transport != "http" || notion.URL == "" {
		t.Fatalf("notion = %+v", notion)
	}
}

func TestMCPConfigKeepsABrokenFileAside(t *testing.T) {
	root := t.TempDir()
	broken := []byte(`{"mcpServers": {oops`)
	path := filepath.Join(root, ".mcp.json")
	if err := os.WriteFile(path, broken, 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := writeMCPWorkspace(root, testEndpoint(root), true); err != nil {
		t.Fatal(err)
	}
	if data, err := os.ReadFile(path + ".bak"); err != nil || string(data) != string(broken) {
		t.Fatalf(".bak = %q, %v", data, err)
	}
	servers := readJSONFile(t, path)["mcpServers"].(map[string]any)
	if _, ok := servers[mcpServerName]; !ok {
		t.Fatalf("broken config was not replaced with ours: %v", servers)
	}
}

func TestMCPSocketPathStaysShort(t *testing.T) {
	long := filepath.Join(t.TempDir(), strings.Repeat("a", 80), strings.Repeat("b", 40))
	sock, err := mcpSocketPath(long)
	if err != nil {
		t.Fatal(err)
	}
	if len(sock) > 96 {
		t.Fatalf("socket path %d bytes: %s", len(sock), sock)
	}
	if !strings.HasSuffix(sock, ".sock") {
		t.Fatalf("socket %s", sock)
	}
	if !strings.HasPrefix(sock, filepath.Join(long, deckDir)) {
		if !strings.HasPrefix(sock, os.TempDir()) {
			t.Fatalf("an over-long socket path did not fall back to the temp folder: %s", sock)
		}
	}
}

// TestOpeningAFolderStartsItsMCP is the wiring: a workspace created or
// opened serves its canvas, becomes the deck the global bridge follows,
// and, with 项目级配置 on, carries the configs that load it.
func TestOpeningAFolderStartsItsMCP(t *testing.T) {
	a := testApp(t)
	a.settings.ProjectMCP = true
	dir := t.TempDir()
	a.openFolder(dir)
	if a.deck == nil || a.deck.mcp == nil {
		t.Fatalf("opening the folder did not start its MCP: %v", a.deck.mcpErr)
	}
	sock := a.deck.mcp.sock
	if _, err := os.Stat(filepath.Join(dir, ".mcp.json")); err != nil {
		t.Fatalf(".mcp.json: %v", err)
	}
	if act, err := readMCPActive(); err != nil || act.Workspace != dir || act.Server.Socket != sock {
		t.Fatalf("active deck = %+v, %v", act, err)
	}
	if _, err := os.Stat(filepath.Join(dir, deckDir, "mcp.json")); err != nil {
		t.Fatalf("descriptor: %v", err)
	}
	ep, err := readMCPEndpoint(dir)
	if err != nil {
		t.Fatal(err)
	}
	if ep.Socket != sock || ep.Token == "" {
		t.Fatalf("endpoint = %+v", ep)
	}

	a.goHome()
	if a.deck != nil {
		t.Fatal("going home left the deck open")
	}
	if _, err := os.Stat(sock); !os.IsNotExist(err) {
		t.Fatalf("the socket outlived the workspace: %v", err)
	}
	if _, err := readMCPActive(); !os.IsNotExist(err) {
		t.Fatalf("the active deck outlived it: %v", err)
	}
	// The configs stay: the workspace still says how to reach a deck.
	if _, err := os.Stat(filepath.Join(dir, ".mcp.json")); err != nil {
		t.Fatalf(".mcp.json after going home: %v", err)
	}
}
