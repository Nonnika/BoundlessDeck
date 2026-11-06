package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	goruntime "runtime"
	"strings"
	"testing"
)

// TestMain keeps the active-deck registry of the whole test run in a
// folder of its own: tests open decks, which must not take the user's.
func TestMain(m *testing.M) {
	dir, err := os.MkdirTemp("", "boundlessdeck-test-")
	if err != nil {
		panic(err)
	}
	mcpActiveOverride = filepath.Join(dir, "active-deck.json")
	code := m.Run()
	_ = os.RemoveAll(dir)
	os.Exit(code)
}

// bridgeOnce sends lines through a bridge and decodes its answers.
func bridgeOnce(t *testing.T, ep mcpEndpoint, find func() (mcpEndpoint, error), lines ...string) []map[string]any {
	t.Helper()
	var out bytes.Buffer
	if err := bridgeLoop(strings.NewReader(strings.Join(lines, "\n")+"\n"), &out, ep, find); err != nil {
		t.Fatal(err)
	}
	var answers []map[string]any
	for _, line := range bytes.Split(bytes.TrimSpace(out.Bytes()), []byte("\n")) {
		var m map[string]any
		if err := json.Unmarshal(line, &m); err != nil {
			t.Fatalf("answer %q: %v", line, err)
		}
		answers = append(answers, m)
	}
	return answers
}

const deckInfoCall = `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"deck_info","arguments":{}}}`

func toolText(t *testing.T, answer map[string]any) (string, bool) {
	t.Helper()
	res, _ := answer["result"].(map[string]any)
	if res == nil {
		t.Fatalf("no result: %v", answer)
	}
	content, _ := res["content"].([]any)
	if len(content) == 0 {
		t.Fatalf("no content: %v", res)
	}
	text, _ := content[0].(map[string]any)["text"].(string)
	isErr, _ := res["isError"].(bool)
	return text, isErr
}

// TestGlobalBridgeFollowsTheOpenDeck: a bridge with no workspace talks to
// the deck the app has open, and to the next one once the app switches.
func TestGlobalBridgeFollowsTheOpenDeck(t *testing.T) {
	d1 := testDeck(t)
	d1.startMCP()
	if d1.mcp == nil {
		t.Fatalf("startMCP: %v", d1.mcpErr)
	}
	first, err := activeEndpoint()
	if err != nil {
		t.Fatal(err)
	}
	text, isErr := toolText(t, bridgeOnce(t, mcpEndpoint{}, activeEndpoint, deckInfoCall)[0])
	if isErr || !strings.Contains(text, filepath.Base(d1.root)) {
		t.Fatalf("deck_info through the global bridge = %v %s", isErr, text)
	}

	d1.stopMCP()
	if _, err := readMCPActive(); !os.IsNotExist(err) {
		t.Fatalf("closing the deck left the registry: %v", err)
	}
	d2 := testDeck(t)
	d2.startMCP()
	defer d2.stopMCP()
	// A bridge still holding the first deck's endpoint finds the second.
	text, isErr = toolText(t, bridgeOnce(t, first, activeEndpoint, deckInfoCall)[0])
	if isErr || !strings.Contains(text, filepath.Base(d2.root)) {
		t.Fatalf("after switching decks, deck_info = %v %s", isErr, text)
	}
}

// TestGlobalBridgeAnswersWithNoDeck: an agent started before any deck is
// open still gets its tools; calling one says to open a folder.
func TestGlobalBridgeAnswersWithNoDeck(t *testing.T) {
	clearMCPActive("")
	_ = os.Remove(mcpActivePath())
	answers := bridgeOnce(t, mcpEndpoint{}, activeEndpoint,
		`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}`,
		`{"jsonrpc":"2.0","method":"notifications/initialized"}`,
		`{"jsonrpc":"2.0","id":2,"method":"tools/list"}`,
		strings.Replace(deckInfoCall, `"id":1`, `"id":3`, 1),
	)
	if len(answers) != 3 {
		t.Fatalf("answers = %v", answers)
	}
	if info, _ := answers[0]["result"].(map[string]any)["serverInfo"].(map[string]any); info["name"] != mcpServerName {
		t.Fatalf("initialize = %v", answers[0])
	}
	if tools, _ := answers[1]["result"].(map[string]any)["tools"].([]any); len(tools) != len(mcpToolList) {
		t.Fatalf("tools/list = %v", answers[1])
	}
	text, isErr := toolText(t, answers[2])
	if !isErr || text != errNoActiveDeck.Error() {
		t.Fatalf("tool call with no deck = %v %q", isErr, text)
	}
}

// TestProjectConfigsOnlyWhenAsked: a folder opened gets no project config
// unless the setting asks; turned on, they are written; removed, only our
// entry goes and files that were ours alone go with it.
func TestProjectConfigsOnlyWhenAsked(t *testing.T) {
	d := testDeck(t)
	d.startMCP()
	defer d.stopMCP()
	if d.mcp == nil {
		t.Fatalf("startMCP: %v", d.mcpErr)
	}
	for _, f := range mcpProjectFiles(d.root) {
		if _, err := os.Stat(f.Path); !os.IsNotExist(err) {
			t.Fatalf("%s written with the setting off", f.Label)
		}
	}
	if _, err := readMCPEndpoint(d.root); err != nil {
		t.Fatalf("the descriptor is always written: %v", err)
	}

	userMCP := filepath.Join(d.root, ".mcp.json")
	if err := os.WriteFile(userMCP, []byte(`{"mcpServers":{"github":{"command":"gh-mcp"}}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := d.setProjectMCP(true); err != nil {
		t.Fatal(err)
	}
	if !projectFilesHaveUs(d.root) {
		t.Fatal("turning it on wrote no project config")
	}
	if err := removeProjectConfigs(d.root); err != nil {
		t.Fatal(err)
	}
	if projectFilesHaveUs(d.root) {
		t.Fatal("our entry is still in a project config")
	}
	servers := readJSONFile(t, userMCP)["mcpServers"].(map[string]any)
	if _, ok := servers["github"]; !ok || len(servers) != 1 {
		t.Fatalf(".mcp.json servers = %v", servers)
	}
	for _, dir := range []string{".cursor", ".vscode"} {
		if _, err := os.Stat(filepath.Join(d.root, dir)); !os.IsNotExist(err) {
			t.Fatalf("%s, ours alone, is still there", dir)
		}
	}
}

// TestGlobalInstallKeepsTheUsersConfig: installing and removing touch
// only our entry, keep the file's permissions, and leave a config that
// will not read alone.
func TestGlobalInstallKeepsTheUsersConfig(t *testing.T) {
	home, config := t.TempDir(), t.TempDir()
	agents := map[string]mcpAgent{}
	for _, ag := range mcpAgents(home, config) {
		agents[ag.ID] = ag
	}
	cc := agents["claude-code"]
	if st := cc.status("/app"); st.State != mcpAbsent {
		t.Fatalf("no Claude Code yet: %v", st.State)
	}
	if err := cc.install("/app"); err == nil {
		t.Fatal("installed into an agent that is not there")
	}
	if err := os.MkdirAll(cc.Home, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(cc.Path, []byte(`{"numStartups":7,"mcpServers":{"github":{"command":"gh"}},"projects":{"/x":{}}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if st := cc.status("/app"); st.State != mcpNotThere {
		t.Fatalf("before install: %v", st.State)
	}
	if err := cc.install("/app"); err != nil {
		t.Fatal(err)
	}
	top := readJSONFile(t, cc.Path)
	if top["numStartups"].(float64) != 7 || top["projects"] == nil {
		t.Fatalf("install lost the user's keys: %v", top)
	}
	ours := top["mcpServers"].(map[string]any)[mcpServerName].(map[string]any)
	if ours["command"] != "/app" || ours["type"] != "stdio" {
		t.Fatalf("our entry = %v", ours)
	}
	if args := ours["args"].([]any); len(args) != 1 || args[0] != "mcp-stdio" {
		t.Fatalf("a global entry pins no workspace: %v", args)
	}
	if goruntime.GOOS != "windows" {
		if info, _ := os.Stat(cc.Path); info.Mode().Perm() != 0o600 {
			t.Fatalf("mode = %v, want 0600", info.Mode().Perm())
		}
	}
	if st := cc.status("/app"); st.State != mcpInstalled {
		t.Fatalf("after install: %v", st.State)
	}
	if st := cc.status("/other"); st.State != mcpElsewhere {
		t.Fatalf("another app's view: %v", st.State)
	}
	if err := cc.uninstall(); err != nil {
		t.Fatal(err)
	}
	servers := readJSONFile(t, cc.Path)["mcpServers"].(map[string]any)
	if _, ok := servers[mcpServerName]; ok || servers["github"] == nil {
		t.Fatalf("after uninstall: %v", servers)
	}

	// VS Code keeps its servers under "servers".
	vs := agents["vscode"]
	if err := os.MkdirAll(vs.Home, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := vs.install("/app"); err != nil {
		t.Fatal(err)
	}
	if _, ok := readJSONFile(t, vs.Path)["servers"].(map[string]any)[mcpServerName]; !ok {
		t.Fatal("VS Code got no entry under servers")
	}

	// A config that is not JSON is never replaced.
	cur := agents["cursor"]
	if err := os.MkdirAll(cur.Home, 0o755); err != nil {
		t.Fatal(err)
	}
	broken := []byte("{ not json")
	if err := os.WriteFile(cur.Path, broken, 0o644); err != nil {
		t.Fatal(err)
	}
	if st := cur.status("/app"); st.State != mcpBroken {
		t.Fatalf("broken config: %v", st.State)
	}
	if err := cur.install("/app"); err == nil {
		t.Fatal("installed over a config that does not read")
	}
	if data, _ := os.ReadFile(cur.Path); !bytes.Equal(data, broken) {
		t.Fatalf("broken config changed: %q", data)
	}
}

func TestStableExe(t *testing.T) {
	if stableExe("/tmp/go-build123/b001/exe/boundlessdeck") == nil {
		t.Fatal("a go run binary was taken as stable")
	}
	if err := stableExe("/Applications/BoundlessDeck.app/Contents/MacOS/BoundlessDeck"); err != nil {
		t.Fatal(err)
	}
}

func TestSettingsItemIsWhereThePlatformKeepsIt(t *testing.T) {
	a := &app{}
	m := a.buildMenu()
	it := m.ItemByID("settings")
	if it == nil || it.Accelerator != "CmdOrCtrl+," {
		t.Fatal("no 设置… item on ⌘,")
	}
	var under string
	for _, top := range a.menuItems {
		for _, sub := range top.Submenu {
			if sub == it {
				under = top.Label
			}
		}
	}
	want := "文件"
	if goruntime.GOOS == "darwin" {
		want = "BoundlessDeck"
	}
	if under != want {
		t.Fatalf("设置… is under %q, want %q", under, want)
	}
}
