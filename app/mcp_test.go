package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// mcpTestServer is a deck's MCP with no socket: the tests call the
// JSON-RPC handler directly.
func mcpTestServer(t *testing.T) (*mcpServer, *deck) {
	t.Helper()
	d := testDeck(t)
	d.viewW, d.viewH = 1200, 800
	return &mcpServer{d: d, token: "test-token", started: mcpNow()}, d
}

// rpcRaw sends one JSON-RPC message and returns the decoded response.
func rpcRaw(t *testing.T, s *mcpServer, req map[string]any) map[string]any {
	t.Helper()
	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	resp := s.handleRPC(data)
	if len(resp) == 0 {
		t.Fatalf("no response to %v", req["method"])
	}
	var out map[string]any
	if err := json.Unmarshal(resp, &out); err != nil {
		t.Fatalf("unmarshal %s: %v", resp, err)
	}
	return out
}

// toolCall calls a tool and returns its structured content, failing on a
// tool error.
func toolCall(t *testing.T, s *mcpServer, name string, arguments any) map[string]any {
	t.Helper()
	out, isErr, text := toolCallRaw(t, s, name, arguments)
	if isErr {
		t.Fatalf("%s failed: %s", name, text)
	}
	if out == nil {
		t.Fatalf("%s returned no structured content: %s", name, text)
	}
	return out
}

// toolCallRaw calls a tool, returning its result whether it failed or not.
func toolCallRaw(t *testing.T, s *mcpServer, name string, arguments any) (map[string]any, bool, string) {
	t.Helper()
	resp := rpcRaw(t, s, map[string]any{
		"jsonrpc": "2.0",
		"id":      1,
		"method":  "tools/call",
		"params": map[string]any{
			"name":      name,
			"arguments": arguments,
		},
	})
	if rerr, ok := resp["error"].(map[string]any); ok {
		t.Fatalf("%s: rpc error %v", name, rerr)
	}
	result, ok := resp["result"].(map[string]any)
	if !ok {
		t.Fatalf("%s: no result: %v", name, resp)
	}
	isErr, _ := result["isError"].(bool)
	text := ""
	if content, ok := result["content"].([]any); ok && len(content) > 0 {
		if first, ok := content[0].(map[string]any); ok {
			text, _ = first["text"].(string)
		}
	}
	structured, _ := result["structuredContent"].(map[string]any)
	return structured, isErr, text
}

// TestMCPSchemaCoversEveryBlockField is the guard that keeps the exposed
// schema true: every JSON field of block is described, and the schema names
// no field block does not have.
func TestMCPSchemaCoversEveryBlockField(t *testing.T) {
	known := mcpFieldNames()
	typ := reflect.TypeOf(block{})
	seen := map[string]bool{}
	for i := 0; i < typ.NumField(); i++ {
		name := strings.Split(typ.Field(i).Tag.Get("json"), ",")[0]
		if name == "" || name == "-" {
			continue
		}
		seen[name] = true
		if !known[name] {
			t.Errorf("block 的字段 %q（%s）没有在 mcpFields 里描述", name, typ.Field(i).Name)
		}
	}
	for _, f := range mcpFields {
		if f.Derived {
			if seen[f.Name] {
				t.Errorf("派生字段 %q 不该存在 block 里", f.Name)
			}
			seen[f.Name] = true
		}
	}
	for name := range known {
		if !seen[name] {
			t.Errorf("mcpFields 描述了 block 没有的字段 %q", name)
		}
	}
}

func TestMCPSchemaByKind(t *testing.T) {
	cases := []struct {
		kind    blockKind
		mutable []string
		not     []string
	}{
		{kindNote, []string{"title", "named", "x", "y", "w", "h", "text"}, []string{"path", "url", "page"}},
		{kindFile, []string{"title", "named", "x", "y", "w", "h", "path", "page"}, []string{"text", "url"}},
		{kindTerminal, []string{"title", "named", "x", "y", "w", "h"}, []string{"text", "path", "url", "page"}},
		{kindBrowser, []string{"title", "named", "x", "y", "w", "h", "url"}, []string{"text", "path", "page"}},
		{kindStack, []string{"title", "named", "x", "y", "w", "h", "items"}, []string{"text", "path", "url", "page"}},
	}
	for _, c := range cases {
		got := mcpMutableNames(c.kind)
		for _, want := range c.mutable {
			if !contains(got, want) {
				t.Errorf("%s 的可写字段没有 %q：%v", c.kind, want, got)
			}
		}
		for _, no := range c.not {
			if contains(got, no) {
				t.Errorf("%s 不该可写 %q：%v", c.kind, no, got)
			}
		}
	}
	digest := mcpSchemaDigest()
	kinds, _ := digest["kinds"].([]map[string]any)
	if len(kinds) != len(blockKinds) {
		t.Fatalf("schema 有 %d 种 kind，want %d", len(kinds), len(blockKinds))
	}
}

func contains(list []string, want string) bool {
	for _, v := range list {
		if v == want {
			return true
		}
	}
	return false
}

func TestMCPInitializeAndToolsList(t *testing.T) {
	s, _ := mcpTestServer(t)
	out := rpcRaw(t, s, map[string]any{
		"jsonrpc": "2.0", "id": 1, "method": "initialize",
		"params": map[string]any{"protocolVersion": "2025-03-26", "capabilities": map[string]any{}},
	})
	result := out["result"].(map[string]any)
	if result["protocolVersion"] != "2025-03-26" {
		t.Fatalf("protocolVersion = %v, want the client's", result["protocolVersion"])
	}
	info := result["serverInfo"].(map[string]any)
	if info["name"] != mcpServerName {
		t.Fatalf("server name %v", info["name"])
	}

	list := rpcRaw(t, s, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
	tools, _ := list["result"].(map[string]any)["tools"].([]any)
	if len(tools) != len(mcpToolList) || len(tools) == 0 {
		t.Fatalf("tools/list has %d tools, want %d", len(tools), len(mcpToolList))
	}
	seen := map[string]bool{}
	for _, raw := range tools {
		tool := raw.(map[string]any)
		if tool["name"] == "" || tool["description"] == "" {
			t.Errorf("tool without name or description: %v", tool)
		}
		if name, _ := tool["name"].(string); seen[name] {
			t.Errorf("tool %q listed twice", name)
		}
		seen[tool["name"].(string)] = true
		if schema, ok := tool["inputSchema"].(map[string]any); !ok || schema["type"] != "object" {
			t.Errorf("%v has no object inputSchema", tool["name"])
		}
	}
	// The tool that writes a block exposes every mutable field of one.
	props := mcpToolByName("update_block").Schema["properties"].(map[string]any)
	for _, f := range mcpFields {
		if f.Mutable {
			if _, ok := props[f.Name]; !ok {
				t.Errorf("update_block 没有暴露可写字段 %q", f.Name)
			}
		}
	}
}

func TestMCPCreateAndReadBlocks(t *testing.T) {
	s, d := mcpTestServer(t)
	out := toolCall(t, s, "create_block", map[string]any{
		"kind": "note", "title": "脚本", "text": "# 开场", "x": 10, "y": 20, "w": 200, "h": 50,
	})
	id, _ := out["id"].(string)
	if id == "" {
		t.Fatalf("no id in %v", out)
	}
	n := d.blockByID(id)
	if n == nil || n.Kind != kindNote || n.Title != "脚本" || n.Text != "# 开场" {
		t.Fatalf("block = %+v", n)
	}
	if n.W < minBlockW || n.H < minBlockH {
		t.Fatalf("size %vx%v under the minimum", n.W, n.H)
	}
	if !d.sel[id] {
		t.Fatal("a new block is not selected")
	}

	list := toolCall(t, s, "list_blocks", map[string]any{})
	blocks, _ := list["blocks"].([]any)
	if len(blocks) != 3 {
		t.Fatalf("list has %d blocks, want 3", len(blocks))
	}

	one := toolCall(t, s, "get_block", map[string]any{"id": id})
	if one["title"] != "脚本" || one["text"] != "# 开场" {
		t.Fatalf("get_block = %v", one)
	}
	if editable, _ := one["editable"].([]any); !containsAny(editable, "text") {
		t.Fatalf("note does not list text as editable: %v", one["editable"])
	}

	// A block without x/y still lands somewhere, and on disk.
	loose := toolCall(t, s, "create_block", map[string]any{"kind": "terminal"})
	if x, _ := loose["x"].(float64); x == 0 {
		if y, _ := loose["y"].(float64); y == 0 {
			t.Fatalf("a block without x/y landed at the origin: %v", loose)
		}
	}
	if _, err := os.Stat(d.deckFile()); err != nil {
		t.Fatalf("the deck was not saved: %v", err)
	}
}

func containsAny(list []any, want string) bool {
	for _, v := range list {
		if v == want {
			return true
		}
	}
	return false
}

func TestMCPCreateFileNeedsAPath(t *testing.T) {
	s, _ := mcpTestServer(t)
	if _, isErr, text := toolCallRaw(t, s, "create_block", map[string]any{"kind": "file"}); !isErr || !strings.Contains(text, "path") {
		t.Fatalf("create file without path: isErr=%v text=%q", isErr, text)
	}
}

func TestMCPUpdateBlockFields(t *testing.T) {
	s, d := mcpTestServer(t)
	out := toolCall(t, s, "update_block", map[string]any{
		"id": "a", "title": "  新标题 ", "text": "正文", "w": 20, "x": 42.5,
	})
	if out["title"] != "新标题" {
		t.Fatalf("title = %v", out["title"])
	}
	n := d.blockByID("a")
	if n.Title != "新标题" || !n.Named || n.Text != "正文" {
		t.Fatalf("block = %+v", n)
	}
	if n.W != minBlockW {
		t.Fatalf("w = %v, want clamped to minBlockW", n.W)
	}
	if n.X != 42.5 {
		t.Fatalf("x = %v", n.X)
	}
	if applied, _ := out["applied"].([]any); len(applied) != 4 {
		t.Fatalf("applied = %v, want 4 fields", applied)
	}

	// Emptying the title gives the kind's default back, unnamed.
	toolCall(t, s, "update_block", map[string]any{"id": "a", "title": " "})
	if n.Title != "QuickLook" || n.Named {
		t.Fatalf("title after clearing = %q named=%v", n.Title, n.Named)
	}
}

func TestMCPUpdateRejectsFieldsOfOtherKinds(t *testing.T) {
	s, d := mcpTestServer(t)
	d.blocks = append(d.blocks, &block{ID: "t", Kind: kindTerminal, Title: "终端", W: 400, H: 300})

	cases := []struct {
		name string
		args map[string]any
		want string
	}{
		{"text on terminal", map[string]any{"id": "t", "text": "hi"}, "text"},
		{"url on note", map[string]any{"id": "a", "url": "https://x"}, "url"},
		{"unknown field", map[string]any{"id": "a", "colour": "red"}, "colour"},
		{"page on note", map[string]any{"id": "a", "page": 1}, "page"},
	}
	for _, c := range cases {
		_, isErr, text := toolCallRaw(t, s, "update_block", c.args)
		if !isErr {
			t.Errorf("%s: no error", c.name)
			continue
		}
		if !strings.Contains(text, c.want) {
			t.Errorf("%s: error %q does not name %q", c.name, text, c.want)
		}
	}
	// The failed patches changed nothing.
	if d.blockByID("t").Text != "" || d.blockByID("a").Title != "A" {
		t.Fatalf("a rejected patch wrote something: %+v %+v", d.blockByID("a"), d.blockByID("t"))
	}
}

func TestMCPUpdatePatchIsAllOrNothing(t *testing.T) {
	s, d := mcpTestServer(t)
	_, isErr, _ := toolCallRaw(t, s, "update_block", map[string]any{
		"id": "a", "title": "改了", "url": "https://example.com",
	})
	if !isErr {
		t.Fatal("a patch with one bad field succeeded")
	}
	if d.blockByID("a").Title != "A" {
		t.Fatalf("a good field was written before a bad one failed: %q", d.blockByID("a").Title)
	}
}

func TestMCPUpdateFileAndBrowser(t *testing.T) {
	s, d := mcpTestServer(t)
	file := filepath.Join(d.root, "notes.md")
	if err := os.WriteFile(file, []byte("# hi"), 0o644); err != nil {
		t.Fatal(err)
	}
	fileOut := toolCall(t, s, "create_block", map[string]any{"kind": "file", "path": file, "page": 3})
	if fileOut["path"] != "notes.md" {
		t.Fatalf("an absolute path inside the root was not made relative: %v", fileOut["path"])
	}
	if fileOut["preview"] != "markdown" {
		t.Fatalf("preview = %v", fileOut["preview"])
	}
	if fileOut["page"].(float64) != 3 {
		t.Fatalf("page = %v", fileOut["page"])
	}

	browserOut := toolCall(t, s, "create_block", map[string]any{"kind": "browser", "url": "example.com"})
	if browserOut["url"] != "https://example.com" {
		t.Fatalf("a bare host was not completed: %v", browserOut["url"])
	}
	if _, isErr, text := toolCallRaw(t, s, "update_block", map[string]any{
		"id": browserOut["id"], "url": "javascript:alert(1)",
	}); !isErr || !strings.Contains(text, "javascript") {
		t.Fatalf("javascript: URL accepted: isErr=%v text=%q", isErr, text)
	}
	if d.blockByID(browserOut["id"].(string)).URL != "https://example.com" {
		t.Fatal("the rejected URL was written")
	}
}

func TestMCPDeleteMoveSelect(t *testing.T) {
	s, d := mcpTestServer(t)
	if _, isErr, _ := toolCallRaw(t, s, "delete_blocks", map[string]any{"ids": []string{"a", "nope"}}); !isErr {
		t.Fatal("a delete with an unknown id succeeded")
	}
	if len(d.blocks) != 2 {
		t.Fatalf("a failed delete removed blocks: %d left", len(d.blocks))
	}
	out := toolCall(t, s, "delete_blocks", map[string]any{"ids": []string{"a"}})
	if deleted, _ := out["deleted"].([]any); len(deleted) != 1 || len(d.blocks) != 1 {
		t.Fatalf("delete = %v, blocks %d", out, len(d.blocks))
	}

	toolCall(t, s, "move_blocks", map[string]any{"ids": []string{"b"}, "dx": 10, "dy": -5})
	if b := d.blockByID("b"); b.X != 410 || b.Y != 295 {
		t.Fatalf("b = (%v,%v), want (410,295)", b.X, b.Y)
	}

	toolCall(t, s, "select_blocks", map[string]any{"ids": []string{"b"}})
	sel := toolCall(t, s, "read_selection", map[string]any{})
	if ids, _ := sel["ids"].([]any); len(ids) != 1 || ids[0] != "b" {
		t.Fatalf("selection = %v", sel["ids"])
	}
}

func TestMCPUndoAndRedo(t *testing.T) {
	s, d := mcpTestServer(t)
	created := toolCall(t, s, "create_block", map[string]any{"kind": "note", "title": "临时"})
	id := created["id"].(string)
	toolCall(t, s, "undo", map[string]any{})
	if d.blockByID(id) != nil {
		t.Fatal("undo did not remove the block")
	}
	toolCall(t, s, "redo", map[string]any{})
	if d.blockByID(id) == nil {
		t.Fatal("redo did not bring the block back")
	}
}

func TestMCPResources(t *testing.T) {
	s, _ := mcpTestServer(t)
	list := rpcRaw(t, s, map[string]any{"jsonrpc": "2.0", "id": 1, "method": "resources/list"})
	resources := list["result"].(map[string]any)["resources"].([]any)
	if len(resources) != 3 {
		t.Fatalf("resources = %v", resources)
	}
	for _, uri := range []string{"boundlessdeck://deck", "boundlessdeck://blocks", "boundlessdeck://schema", "boundlessdeck://blocks/a"} {
		resp := rpcRaw(t, s, map[string]any{
			"jsonrpc": "2.0", "id": 2, "method": "resources/read",
			"params": map[string]any{"uri": uri},
		})
		if _, ok := resp["error"]; ok {
			t.Fatalf("%s: %v", uri, resp["error"])
		}
		contents := resp["result"].(map[string]any)["contents"].([]any)
		first := contents[0].(map[string]any)
		if text, _ := first["text"].(string); !json.Valid([]byte(text)) {
			t.Fatalf("%s: not JSON: %q", uri, text)
		}
	}
}

func TestMCPHTTPNeedsTheToken(t *testing.T) {
	s, _ := mcpTestServer(t)
	ts := httptest.NewServer(s.handler())
	defer ts.Close()
	body := `{"jsonrpc":"2.0","id":1,"method":"ping"}`

	resp, err := http.Post(ts.URL+"/mcp", "application/json", strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("no token: %d", resp.StatusCode)
	}

	req, _ := http.NewRequest(http.MethodPost, ts.URL+"/mcp", strings.NewReader(body))
	req.Header.Set("X-BoundlessDeck-Token", "test-token")
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("with token: %d", resp.StatusCode)
	}
	var out map[string]any
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		t.Fatal(err)
	}
	if _, ok := out["result"]; !ok {
		t.Fatalf("ping = %v", out)
	}

	// A notification gets 202 and no body.
	notify := `{"jsonrpc":"2.0","method":"notifications/initialized"}`
	req, _ = http.NewRequest(http.MethodPost, ts.URL+"/mcp", strings.NewReader(notify))
	req.Header.Set("X-BoundlessDeck-Token", "test-token")
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusAccepted {
		t.Fatalf("notification: %d", resp.StatusCode)
	}
}

func TestMCPUnixSocketAndBridge(t *testing.T) {
	d := testDeck(t)
	sock, err := mcpSocketPath(d.root)
	if err != nil {
		t.Fatal(err)
	}
	s, err := startMCPForDeck(d, sock)
	if err != nil {
		t.Fatal(err)
	}
	defer s.close()
	ep := mcpEndpoint{Socket: s.sock, Token: s.token}

	status, body, err := postMCP(ep, []byte(`{"jsonrpc":"2.0","id":7,"method":"initialize","params":{}}`))
	if err != nil {
		t.Fatal(err)
	}
	if status != http.StatusOK {
		t.Fatalf("status %d: %s", status, body)
	}
	if !bytes.Contains(body, []byte(`"serverInfo"`)) {
		t.Fatalf("initialize answer = %s", body)
	}

	// The stdio bridge relays a line and its answer.
	var out bytes.Buffer
	in := strings.NewReader(`{"jsonrpc":"2.0","id":9,"method":"tools/list"}` + "\n")
	if err := bridgeLoop(in, &out, ep, nil); err != nil {
		t.Fatal(err)
	}
	var resp map[string]any
	if err := json.Unmarshal(bytes.TrimSpace(out.Bytes()), &resp); err != nil {
		t.Fatalf("bridge answer %q: %v", out.String(), err)
	}
	if resp["id"].(float64) != 9 {
		t.Fatalf("bridge answer id = %v", resp["id"])
	}
	if _, ok := resp["result"].(map[string]any)["tools"]; !ok {
		t.Fatalf("bridge tools/list = %v", resp["result"])
	}

	// A bridge that cannot reach the app answers with an error, not a hang.
	var dead bytes.Buffer
	deadEP := mcpEndpoint{Socket: filepath.Join(t.TempDir(), "nothing.sock")}
	if err := bridgeLoop(strings.NewReader(`{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"boundlessdeck://deck"}}`+"\n"), &dead, deadEP, nil); err != nil {
		t.Fatal(err)
	}
	var errResp map[string]any
	if err := json.Unmarshal(bytes.TrimSpace(dead.Bytes()), &errResp); err != nil {
		t.Fatalf("dead bridge answer %q: %v", dead.String(), err)
	}
	if _, ok := errResp["error"].(map[string]any); !ok {
		t.Fatalf("dead bridge = %v", errResp)
	}
}

func TestMCPStopsWithItsDeck(t *testing.T) {
	d := testDeck(t)
	d.startMCP()
	if d.mcp == nil {
		t.Fatalf("startMCP failed: %v", d.mcpErr)
	}
	sock := d.mcp.sock
	if _, err := os.Stat(sock); err != nil {
		t.Fatalf("socket %s: %v", sock, err)
	}
	if _, err := readMCPEndpoint(d.root); err != nil {
		t.Fatalf("descriptor: %v", err)
	}
	d.stopMCP()
	if d.mcp != nil || d.mcpProject != nil {
		t.Fatal("stopMCP left state behind")
	}
	if _, err := os.Stat(sock); !os.IsNotExist(err) {
		t.Fatalf("socket still there: %v", err)
	}
}

func TestMCPUpdateBlocksIsAllOrNothing(t *testing.T) {
	s, d := mcpTestServer(t)
	// url is not a note field, so the whole batch is refused.
	_, isErr, text := toolCallRaw(t, s, "update_blocks", map[string]any{
		"ids":    []string{"a", "b"},
		"fields": map[string]any{"title": "批量", "url": "https://example.com"},
	})
	if !isErr || !strings.Contains(text, "url") {
		t.Fatalf("bad batch: isErr=%v text=%q", isErr, text)
	}
	if d.blockByID("a").Title != "A" || d.blockByID("b").Title != "B" {
		t.Fatal("a refused batch changed a block")
	}

	out := toolCall(t, s, "update_blocks", map[string]any{
		"ids":    []string{"a", "b"},
		"fields": map[string]any{"title": "批量"},
	})
	if updated, _ := out["updated"].([]any); len(updated) != 2 {
		t.Fatalf("updated = %v", out["updated"])
	}
	if d.blockByID("a").Title != "批量" || d.blockByID("b").Title != "批量" {
		t.Fatal("a good batch did not write both blocks")
	}
}

func TestMCPSelectLeavesSelectionOnUnknownID(t *testing.T) {
	s, d := mcpTestServer(t)
	toolCall(t, s, "select_blocks", map[string]any{"ids": []string{"a"}})
	if _, isErr, _ := toolCallRaw(t, s, "select_blocks", map[string]any{"ids": []string{"nope"}}); !isErr {
		t.Fatal("selecting an unknown block succeeded")
	}
	if !d.sel["a"] {
		t.Fatal("a refused selection changed the selection")
	}
}

func TestMCPResourceTemplates(t *testing.T) {
	s, _ := mcpTestServer(t)
	out := rpcRaw(t, s, map[string]any{"jsonrpc": "2.0", "id": 1, "method": "resources/templates/list"})
	templates, _ := out["result"].(map[string]any)["resourceTemplates"].([]any)
	if len(templates) != 1 {
		t.Fatalf("templates = %v", out["result"])
	}
	if templates[0].(map[string]any)["uriTemplate"] != "boundlessdeck://blocks/{id}" {
		t.Fatalf("template = %v", templates[0])
	}
}
