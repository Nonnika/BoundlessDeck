package main

import (
	"bytes"
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"hash/fnv"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"time"
)

// The MCP server is the workspace's own: one per open deck, listening on a
// Unix socket inside the workspace, so that an agent working in the folder
// drives the same canvas the window shows. Its stdio bridge (mcpbridge.go)
// is what project configs point at; tools and resources are listed below.
const (
	mcpProtocolVersion = "2025-06-18"
	mcpServerVersion   = "0.1.0"
	mcpMaxBody         = 8 << 20
	mcpMaxResponse     = 32 << 20
	mcpApplyTimeout    = 15 * time.Second
)

var errMCPStopped = errors.New("这个工作区已经关了")

// mcpServer is a deck's MCP endpoint.
type mcpServer struct {
	d         *deck
	http      *http.Server
	ln        net.Listener
	transport string // unix, or tcp where Unix sockets are not there
	sock      string // the socket's path, or a tcp address
	token     string
	started   time.Time
	stopped   atomic.Bool
}

// mcpSocketPath chooses where a workspace's MCP socket lives: in its
// .boundlessdeck folder, or, when that path would not fit a Unix socket,
// under the system's temp folder, hashed from the workspace path.
func mcpSocketPath(root string) (string, error) {
	preferred := filepath.Join(root, deckDir, "mcp.sock")
	// sockaddr_un.sun_path is 104 bytes on macOS, 108 on Linux: stay clear
	// of both, and of the temporary name Go binds while listening.
	if len(preferred) <= 92 {
		return preferred, nil
	}
	h := fnv.New32a()
	_, _ = h.Write([]byte(root))
	return filepath.Join(os.TempDir(), fmt.Sprintf("boundlessdeck-mcp-%08x.sock", h.Sum32())), nil
}

// startMCPForDeck listens on sock and serves the deck's MCP there. A Unix
// socket keeps it off every port; a platform without them gets a loopback
// port instead, still behind the token.
func startMCPForDeck(d *deck, sock string) (*mcpServer, error) {
	transport := "unix"
	if err := os.MkdirAll(filepath.Dir(sock), 0o755); err != nil {
		return nil, err
	}
	// A socket left by a crash is not a listener any more: remove it, or
	// binding fails.
	_ = os.Remove(sock)
	ln, err := net.Listen("unix", sock)
	if err != nil {
		ln, err = net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			return nil, err
		}
		transport, sock = "tcp", ln.Addr().String()
	} else if err := os.Chmod(sock, 0o600); err != nil {
		_ = ln.Close()
		return nil, err
	}
	s := &mcpServer{
		d:         d,
		ln:        ln,
		transport: transport,
		sock:      sock,
		token:     mcpNewToken(),
		started:   time.Now(),
	}
	s.http = &http.Server{
		Handler:           s.handler(),
		ReadHeaderTimeout: 10 * time.Second,
	}
	go func() { _ = s.http.Serve(ln) }()
	return s, nil
}

func mcpNewToken() string {
	buf := make([]byte, 16)
	if _, err := rand.Read(buf); err != nil {
		return fmt.Sprintf("t%d", time.Now().UnixNano())
	}
	return hex.EncodeToString(buf)
}

func (s *mcpServer) close() {
	if s.stopped.Swap(true) {
		return
	}
	_ = s.http.Close()
	_ = s.ln.Close()
	if s.transport == "unix" {
		_ = os.Remove(s.sock)
	}
}

// onUI runs f with the deck on the window's thread, where all of the deck
// changes; without a window, as in tests, it runs it here.
func (s *mcpServer) onUI(f func()) error {
	if s.stopped.Load() {
		return errMCPStopped
	}
	var ferr error
	err := s.d.runOnUI(func() {
		if s.stopped.Load() {
			ferr = errMCPStopped
			return
		}
		f()
	})
	if err != nil {
		return err
	}
	return ferr
}

// runOnUI runs f on the UI thread and waits for it. A window that does not
// answer within mcpApplyTimeout fails the call instead of hanging the
// agent: the deck may belong to a window that is closing.
func (d *deck) runOnUI(f func()) error {
	if d.win == nil {
		f()
		return nil
	}
	done := make(chan struct{})
	d.win.Update(func() {
		f()
		close(done)
	})
	select {
	case <-done:
		return nil
	case <-time.After(mcpApplyTimeout):
		return fmt.Errorf("窗口 %s 没有响应", mcpApplyTimeout)
	}
}

func (s *mcpServer) handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/mcp", s.serveMCP)
	mux.HandleFunc("/health", s.serveHealth)
	return mux
}

func (s *mcpServer) serveHealth(w http.ResponseWriter, r *http.Request) {
	if !s.authorized(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"ok":        true,
		"server":    mcpServerName,
		"workspace": s.d.root,
		"socket":    s.sock,
		"tools":     len(mcpToolList),
		"startedAt": s.started,
		"uptime":    time.Since(s.started).Round(time.Second).String(),
	})
}

func (s *mcpServer) authorized(r *http.Request) bool {
	if s.token == "" {
		return true
	}
	got := r.Header.Get("X-BoundlessDeck-Token")
	if got == "" {
		if auth := r.Header.Get("Authorization"); strings.HasPrefix(auth, "Bearer ") {
			got = strings.TrimPrefix(auth, "Bearer ")
		}
	}
	return subtle.ConstantTimeCompare([]byte(got), []byte(s.token)) == 1
}

// serveMCP takes one JSON-RPC message, or a batch, and answers it. A
// notification gets 202 and no body, as the streamable HTTP transport says.
func (s *mcpServer) serveMCP(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		w.Header().Set("Allow", http.MethodPost)
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}
	if !s.authorized(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	body, err := io.ReadAll(io.LimitReader(r.Body, mcpMaxBody+1))
	if err != nil {
		http.Error(w, "read: "+err.Error(), http.StatusBadRequest)
		return
	}
	if len(body) > mcpMaxBody {
		http.Error(w, "too large", http.StatusRequestEntityTooLarge)
		return
	}
	out, status := s.handleBody(body)
	if status == http.StatusNoContent || len(out) == 0 {
		w.WriteHeader(http.StatusAccepted)
		return
	}
	if status != http.StatusOK {
		http.Error(w, string(out), status)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_, _ = w.Write(out)
}

// handleBody answers a single message or a batch.
func (s *mcpServer) handleBody(body []byte) ([]byte, int) {
	trimmed := bytes.TrimSpace(body)
	if len(trimmed) == 0 {
		return []byte("empty body"), http.StatusBadRequest
	}
	if trimmed[0] == '[' {
		var batch []json.RawMessage
		if err := json.Unmarshal(trimmed, &batch); err != nil {
			return mcpMarshal(rpcResponse{
				JSONRPC: "2.0",
				Error:   &rpcError{Code: -32700, Message: "JSON 解析失败：" + err.Error()},
			}), http.StatusOK
		}
		var out [][]byte
		for _, one := range batch {
			if resp := s.handleRPC(one); len(resp) > 0 {
				out = append(out, resp)
			}
		}
		if len(out) == 0 {
			return nil, http.StatusNoContent
		}
		return bytes.Join(out, []byte("\n")), http.StatusOK
	}
	if !json.Valid(trimmed) {
		return mcpMarshal(rpcResponse{
			JSONRPC: "2.0",
			Error:   &rpcError{Code: -32700, Message: "JSON 解析失败"},
		}), http.StatusOK
	}
	resp := s.handleRPC(trimmed)
	if len(resp) == 0 {
		return nil, http.StatusNoContent
	}
	return resp, http.StatusOK
}

type rpcRequest struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params"`
}

type rpcError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
	Data    any    `json:"data,omitempty"`
}

type rpcResponse struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Result  any             `json:"result,omitempty"`
	Error   *rpcError       `json:"error,omitempty"`
}

func mcpMarshal(v any) []byte {
	data, err := json.Marshal(v)
	if err != nil {
		return []byte(`{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"结果无法序列化"}}`)
	}
	return data
}

// requestID says whether the message carries an id: without one it is a
// notification, which gets no answer.
func requestID(raw json.RawMessage) (json.RawMessage, bool) {
	trimmed := bytes.TrimSpace(raw)
	if len(trimmed) == 0 || bytes.Equal(trimmed, []byte("null")) {
		return nil, false
	}
	return trimmed, true
}

// handleRPC answers one JSON-RPC message.
func (s *mcpServer) handleRPC(raw []byte) []byte {
	var req rpcRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		return mcpMarshal(rpcResponse{
			JSONRPC: "2.0",
			Error:   &rpcError{Code: -32700, Message: "JSON 解析失败：" + err.Error()},
		})
	}
	if strings.HasPrefix(req.Method, "notifications/") {
		return nil
	}
	id, hasID := requestID(req.ID)
	if !hasID {
		return nil
	}
	result, rerr := s.dispatch(req)
	if rerr != nil {
		return mcpMarshal(rpcResponse{JSONRPC: "2.0", ID: id, Error: rerr})
	}
	return mcpMarshal(rpcResponse{JSONRPC: "2.0", ID: id, Result: result})
}

// dispatch routes a method. Everything that touches the deck runs on the
// UI thread; the protocol itself does not.
func (s *mcpServer) dispatch(req rpcRequest) (any, *rpcError) {
	switch req.Method {
	case "initialize":
		return s.initialize(req.Params)
	case "ping":
		return map[string]any{}, nil
	case "tools/list":
		return map[string]any{"tools": mcpToolDescriptors()}, nil
	case "tools/call":
		return s.callTool(req.Params)
	case "resources/list":
		return map[string]any{"resources": mcpResources()}, nil
	case "resources/templates/list":
		return map[string]any{"resourceTemplates": []map[string]any{{
			"uriTemplate": "boundlessdeck://blocks/{id}",
			"name":        "block",
			"title":       "一个 block",
			"description": "按 block ID 读它的当前值与可写字段。",
			"mimeType":    "application/json",
		}}}, nil
	case "resources/read":
		return s.readResource(req.Params)
	case "prompts/list":
		return map[string]any{"prompts": []any{}}, nil
	case "logging/setLevel":
		return map[string]any{}, nil
	}
	return nil, &rpcError{Code: -32601, Message: "没有方法 " + req.Method}
}

// initialize answers the handshake: the protocol version the client asked
// for when it is one we speak, else the newest.
func (s *mcpServer) initialize(params json.RawMessage) (any, *rpcError) {
	asked := ""
	if len(params) > 0 {
		var p struct {
			ProtocolVersion string `json:"protocolVersion"`
		}
		if err := json.Unmarshal(params, &p); err == nil {
			asked = p.ProtocolVersion
		}
	}
	version := mcpProtocolVersion
	switch asked {
	case "2024-11-05", "2025-03-26", "2025-06-18":
		version = asked
	}
	return map[string]any{
		"protocolVersion": version,
		"capabilities": map[string]any{
			"tools":     map[string]any{"listChanged": false},
			"resources": map[string]any{"subscribe": false, "listChanged": false},
			"prompts":   map[string]any{"listChanged": false},
		},
		"serverInfo": map[string]any{
			"name":    mcpServerName,
			"title":   "BoundlessDeck 画布",
			"version": mcpServerVersion,
		},
		"instructions": "这是 BoundlessDeck 的无限画布。「block」是画布上的卡片，" +
			"kind 有 note（QuickLook 便签）、memo（笔记：Markdown 文件）、file（本地文件预览）、terminal（终端）、browser（浏览器）、stack（块堆：用 items 收纳其他 block）。" +
			"先用 deck_info 和 block_schema 了解现场，用 list_blocks / read_selection 读，" +
			"用 create_block / update_block / move_blocks / delete_blocks 写；" +
			"排版用 align_blocks（对齐边或中线）、distribute_blocks（等间距）、arrange_blocks（行/列/网格）。" +
			"加、删、移动、堆的收放与排版都进撤销栈，并保存到工作区。",
	}, nil
}

// callTool runs a tool with its arguments, on the UI thread.
func (s *mcpServer) callTool(params json.RawMessage) (any, *rpcError) {
	var p struct {
		Name      string          `json:"name"`
		Arguments json.RawMessage `json:"arguments"`
	}
	if len(params) == 0 {
		return nil, &rpcError{Code: -32602, Message: "tools/call 需要 name"}
	}
	if err := json.Unmarshal(params, &p); err != nil {
		return nil, &rpcError{Code: -32602, Message: "tools/call 参数不对：" + err.Error()}
	}
	tool := mcpToolByName(p.Name)
	if tool == nil {
		return nil, &rpcError{Code: -32602, Message: "没有工具 " + p.Name}
	}
	args := mcpArgs{}
	if len(p.Arguments) > 0 && !bytes.Equal(bytes.TrimSpace(p.Arguments), []byte("null")) {
		if err := json.Unmarshal(p.Arguments, &args); err != nil {
			return mcpToolFailure(fmt.Errorf("参数要是对象：%w", err)), nil
		}
	}
	var out any
	var callErr error
	if err := s.onUI(func() { out, callErr = tool.Call(s.d, args) }); err != nil {
		return mcpToolFailure(err), nil
	}
	if callErr != nil {
		return mcpToolFailure(callErr), nil
	}
	return mcpToolResult(out), nil
}

// mcpToolResult is a CallToolResult: the JSON as text for clients that
// only read text, and structuredContent for those that read objects.
func mcpToolResult(out any) map[string]any {
	text, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		text = []byte(fmt.Sprintf("%v", out))
	}
	res := map[string]any{
		"content": []any{map[string]any{"type": "text", "text": string(text)}},
		"isError": false,
	}
	if obj, ok := out.(map[string]any); ok {
		res["structuredContent"] = obj
	}
	if list, ok := out.([]map[string]any); ok {
		res["structuredContent"] = map[string]any{"blocks": list}
	}
	return res
}

func mcpToolFailure(err error) map[string]any {
	return map[string]any{
		"content": []any{map[string]any{"type": "text", "text": err.Error()}},
		"isError": true,
	}
}

// mcpResources lists what a client may read whole.
func mcpResources() []map[string]any {
	return []map[string]any{
		{
			"uri":         "boundlessdeck://deck",
			"name":        "deck",
			"title":       "工作区与画布概况",
			"description": "工作区路径、block 数量、选择集、相机、MCP 状态。",
			"mimeType":    "application/json",
		},
		{
			"uri":         "boundlessdeck://blocks",
			"name":        "blocks",
			"title":       "全部 block",
			"description": "画布上每个 block 的当前值与可写字段。",
			"mimeType":    "application/json",
		},
		{
			"uri":         "boundlessdeck://schema",
			"name":        "schema",
			"title":       "block 字段 schema",
			"description": "每种 block 的可写字段、只读字段、默认值与约束。",
			"mimeType":    "application/json",
		},
	}
}

// readResource returns one resource as JSON text.
func (s *mcpServer) readResource(params json.RawMessage) (any, *rpcError) {
	var p struct {
		URI string `json:"uri"`
	}
	if err := json.Unmarshal(params, &p); err != nil || p.URI == "" {
		return nil, &rpcError{Code: -32602, Message: "resources/read 需要 uri"}
	}
	var out any
	var readErr error
	if err := s.onUI(func() { out, readErr = s.d.mcpReadResource(p.URI) }); err != nil {
		return nil, &rpcError{Code: -32000, Message: err.Error()}
	}
	if readErr != nil {
		return nil, &rpcError{Code: -32602, Message: readErr.Error()}
	}
	text, _ := json.MarshalIndent(out, "", "  ")
	return map[string]any{
		"contents": []any{map[string]any{
			"uri":      p.URI,
			"mimeType": "application/json",
			"text":     string(text),
		}},
	}, nil
}

// mcpReadResource resolves a boundlessdeck:// URI.
func (d *deck) mcpReadResource(uri string) (any, error) {
	const prefix = "boundlessdeck://"
	if !strings.HasPrefix(uri, prefix) {
		return nil, fmt.Errorf("只认 %s 开头的资源：%s", prefix, uri)
	}
	path := strings.TrimPrefix(uri, prefix)
	switch {
	case path == "deck" || path == "deck/":
		return d.mcpDeckInfo(), nil
	case path == "blocks" || path == "blocks/":
		return map[string]any{"blocks": d.mcpListBlocks("")}, nil
	case path == "schema" || path == "schema/":
		return mcpSchemaDigest(), nil
	case strings.HasPrefix(path, "blocks/"):
		id := strings.TrimPrefix(path, "blocks/")
		n := d.blockByID(id)
		if n == nil {
			return nil, fmt.Errorf("没有 id 为 %q 的 block", id)
		}
		return d.mcpFieldsJSON(n), nil
	}
	return nil, fmt.Errorf("没有资源 %s", uri)
}

// mcpTool is one tool: its descriptor and what it does.
type mcpTool struct {
	Name        string
	Title       string
	Description string
	Schema      map[string]any
	Call        func(*deck, mcpArgs) (any, error)
}

// mcpToolByName finds a tool, or nil.
func mcpToolByName(name string) *mcpTool {
	for i := range mcpToolList {
		if mcpToolList[i].Name == name {
			return &mcpToolList[i]
		}
	}
	return nil
}

// mcpToolDescriptors is tools/list: the tools with their input schemas.
func mcpToolDescriptors() []map[string]any {
	out := make([]map[string]any, 0, len(mcpToolList))
	for _, t := range mcpToolList {
		out = append(out, map[string]any{
			"name":        t.Name,
			"title":       t.Title,
			"description": t.Description,
			"inputSchema": t.Schema,
		})
	}
	return out
}

const mcpServerName = "boundlessdeck"

func mcpObjectSchema(props map[string]any, required ...string) map[string]any {
	s := map[string]any{
		"type":                 "object",
		"properties":           props,
		"additionalProperties": false,
	}
	if len(required) > 0 {
		s["required"] = required
	}
	return s
}

// mcpStringArrayProp is the ids array every batch tool takes.
func mcpStringArrayProp(what string) map[string]any {
	return map[string]any{
		"type":        "array",
		"items":       map[string]any{"type": "string"},
		"description": what,
	}
}

// mcpToolList is the tool surface. Every write goes through the deck's own
// addBlock/removeBlock, so agent edits undo and save like the user's.
//
// It is filled in init: some tools read the deck's MCP status, which counts
// the tools, and a variable initializer would close that cycle.
var mcpToolList []mcpTool

func init() {
	mcpToolList = []mcpTool{
		{
			Name:  "deck_info",
			Title: "工作区概况",
			Description: "看当前工作区：路径、block 数、选择集、相机、MCP 端点与项目级 server 列表。" +
				"任何写操作前先调它。",
			Schema: mcpObjectSchema(map[string]any{}),
			Call: func(d *deck, _ mcpArgs) (any, error) {
				return d.mcpDeckInfo(), nil
			},
		},
		{
			Name:  "block_schema",
			Title: "block 字段 schema",
			Description: "列出每种 block 的可写字段、只读字段、默认值与约束。改字段前用它确认字段名和类型；" +
				"省略 kind 则返回全部种类。",
			Schema: mcpObjectSchema(map[string]any{
				"kind": map[string]any{
					"type":        "string",
					"enum":        kindNames(),
					"description": "只看这一种 block",
				},
			}),
			Call: func(d *deck, args mcpArgs) (any, error) {
				kindText, ok, err := args.str("kind")
				if err != nil {
					return nil, err
				}
				out := mcpBlockSchema()
				if !ok || kindText == "" {
					return map[string]any{"schemaVersion": mcpSchemaVersion, "kinds": out}, nil
				}
				for _, k := range out {
					if k["kind"] == kindText {
						return k, nil
					}
				}
				return nil, fmt.Errorf("没有 kind %q", kindText)
			},
		},
		{
			Name:        "list_blocks",
			Title:       "列出 block",
			Description: "列出画布上的 block，含每个的当前值与可写字段。可按 kind 过滤。",
			Schema: mcpObjectSchema(map[string]any{
				"kind": map[string]any{
					"type":        "string",
					"enum":        kindNames(),
					"description": "只看这一种 block",
				},
			}),
			Call: func(d *deck, args mcpArgs) (any, error) {
				kindText, _, err := args.str("kind")
				if err != nil {
					return nil, err
				}
				return d.mcpListBlocks(blockKind(kindText)), nil
			},
		},
		{
			Name:        "get_block",
			Title:       "读一个 block",
			Description: "按 id 读一个 block 的全部字段。",
			Schema: mcpObjectSchema(map[string]any{
				"id": map[string]any{"type": "string", "description": "block ID"},
			}, "id"),
			Call: func(d *deck, args mcpArgs) (any, error) {
				id, _, err := args.str("id")
				if err != nil {
					return nil, err
				}
				n := d.blockByID(id)
				if n == nil {
					return nil, fmt.Errorf("没有 id 为 %q 的 block", id)
				}
				return d.mcpFieldsJSON(n), nil
			},
		},
		{
			Name:        "read_selection",
			Title:       "读选择集",
			Description: "读人当前选中的 block——「选择即上下文」。",
			Schema:      mcpObjectSchema(map[string]any{}),
			Call: func(d *deck, _ mcpArgs) (any, error) {
				return map[string]any{
					"ids":    d.mcpSelectedIDs(),
					"blocks": d.mcpSelectedBlocks(),
				}, nil
			},
		},
		{
			Name:        "create_block",
			Title:       "新建 block",
			Description: "在画布上建一个 block 并写初始字段；kind 必填。file 需要 path。省略 x/y 时落在视口中央附近的空位。",
			Schema:      mcpObjectSchema(mcpCreateProps(), "kind"),
			Call: func(d *deck, args mcpArgs) (any, error) {
				return d.mcpCreateBlock(args)
			},
		},
		{
			Name:  "update_block",
			Title: "改一个 block",
			Description: "按 id 写一个或多个可写字段；字段按 block 的 kind 校验，写入是原子的（有一个不合法就一个都不写）。" +
				"也可以用 fields 对象一次传多个字段。",
			Schema: mcpObjectSchema(func() map[string]any {
				props := mcpMutableProps()
				props["id"] = map[string]any{"type": "string", "description": "block ID"}
				props["fields"] = map[string]any{
					"type":                 "object",
					"description":          "要写的字段，等价于把它们平铺在参数里",
					"additionalProperties": true,
				}
				return props
			}(), "id"),
			Call: func(d *deck, args mcpArgs) (any, error) {
				id, _, err := args.str("id")
				if err != nil {
					return nil, err
				}
				return d.mcpUpdateBlock(id, args)
			},
		},
		{
			Name:        "update_blocks",
			Title:       "批量改 block",
			Description: "把同一组字段写到多个 block；任何一个不合法就整批失败。",
			Schema: mcpObjectSchema(map[string]any{
				"ids":    mcpStringArrayProp("要改的 block ID"),
				"fields": map[string]any{"type": "object", "additionalProperties": true, "description": "要写的字段"},
			}, "ids", "fields"),
			Call: func(d *deck, args mcpArgs) (any, error) {
				ids, err := args.strings("ids")
				if err != nil {
					return nil, err
				}
				var fields mcpArgs
				raw, ok := args["fields"]
				if !ok {
					return nil, fmt.Errorf("update_blocks 需要 fields")
				}
				if err := json.Unmarshal(raw, &fields); err != nil {
					return nil, fmt.Errorf("fields 要是对象：%w", err)
				}
				if len(ids) == 0 {
					return nil, fmt.Errorf("update_blocks 需要 ids")
				}
				// Dry-run on every block first, then write: a bad field on the
				// third block leaves the first two untouched.
				var checked []*block
				for _, id := range ids {
					n := d.blockByID(id)
					if n == nil {
						return nil, fmt.Errorf("没有 id 为 %q 的 block", id)
					}
					if err := d.mcpValidatePatch(n.Kind, fields); err != nil {
						return nil, fmt.Errorf("%s：%w", id, err)
					}
					checked = append(checked, n)
				}
				updated := []map[string]any{}
				for _, n := range checked {
					if _, err := d.mcpPatchBlock(n, fields); err != nil {
						return nil, err
					}
					updated = append(updated, d.mcpFieldsJSON(n))
				}
				if len(updated) > 0 {
					d.requestSave()
				}
				return map[string]any{"updated": updated}, nil
			},
		},
		{
			Name:        "delete_blocks",
			Title:       "删除 block",
			Description: "删除一个或多个 block；有任何一个 id 不存在就一个都不删。可撤销。",
			Schema: mcpObjectSchema(map[string]any{
				"ids": mcpStringArrayProp("要删的 block ID"),
			}, "ids"),
			Call: func(d *deck, args mcpArgs) (any, error) {
				ids, err := args.strings("ids")
				if err != nil {
					return nil, err
				}
				return d.mcpDeleteBlocks(ids)
			},
		},
		{
			Name:        "move_blocks",
			Title:       "移动 block",
			Description: "把一组 block 在世界坐标里平移 dx、dy。要落到位就用 update_block 的 x/y。",
			Schema: mcpObjectSchema(map[string]any{
				"ids": mcpStringArrayProp("要移动的 block ID"),
				"dx":  map[string]any{"type": "number", "description": "水平位移"},
				"dy":  map[string]any{"type": "number", "description": "垂直位移"},
			}, "ids"),
			Call: func(d *deck, args mcpArgs) (any, error) {
				ids, err := args.strings("ids")
				if err != nil {
					return nil, err
				}
				dx, _, err := args.num("dx")
				if err != nil {
					return nil, err
				}
				dy, _, err := args.num("dy")
				if err != nil {
					return nil, err
				}
				return d.mcpMoveBlocks(ids, dx, dy)
			},
		},
		{
			Name:  "align_blocks",
			Title: "对齐 block",
			Description: "把一组 block 对齐到同一条边或中线（左、右、上、下、水平居中、垂直居中）。" +
				"默认以这组 block 的外框为参考，也可以对齐到第一个、最后一个或指定的 block；" +
				"再给 spacing 就会沿另一轴按顺序排成一列/一行。省略 ids 则用当前选择集。",
			Schema: mcpObjectSchema(map[string]any{
				"ids": mcpStringArrayProp("要对齐的 block；省略则用当前选择集"),
				"align": map[string]any{
					"type":        "string",
					"enum":        []any{"left", "right", "top", "bottom", "centerX", "centerY"},
					"description": "对齐到哪条边或中线",
				},
				"to": map[string]any{
					"type":        "string",
					"description": "参考：bounds（默认，这组 block 的外框）、first、last，或一个 block ID",
				},
				"spacing": map[string]any{
					"type":        "number",
					"minimum":     0,
					"description": "给了就沿另一轴按当前顺序首尾排开，间距为这个值（world 单位）：对齐 left + spacing 得到整齐的一列",
				},
			}, "align"),
			Call: func(d *deck, args mcpArgs) (any, error) { return d.mcpAlignBlocks(args) },
		},
		{
			Name:  "distribute_blocks",
			Title: "均匀分布 block",
			Description: "沿 x 或 y 轴把三个以上 block 均匀排开，最外侧两个不动。" +
				"mode=gaps 让相邻间隙相等，mode=centers 让中心间距相等。空间不够时默认报错，允许重叠传 allowOverlap=true。",
			Schema: mcpObjectSchema(map[string]any{
				"ids": mcpStringArrayProp("要分布的 block；省略则用当前选择集"),
				"axis": map[string]any{
					"type": "string", "enum": []any{"x", "y"},
					"description": "沿哪条轴分布",
				},
				"mode": map[string]any{
					"type": "string", "enum": []any{"gaps", "centers"},
					"description": "gaps（默认）等间隙，centers 等中心距",
				},
				"allowOverlap": map[string]any{
					"type": "boolean", "description": "间隙算出来是负数时也照做（block 会重叠）",
				},
			}),
			Call: func(d *deck, args mcpArgs) (any, error) { return d.mcpDistributeBlocks(args) },
		},
		{
			Name:  "arrange_blocks",
			Title: "排列 block",
			Description: "把 block 排成一行、一列或网格，保持各自大小，默认外框左上角为起点、间距 24。" +
				"网格按 ids 顺序填充，columns 省略时取 ceil(sqrt(n))；order=position 按画布上的位置（从上到下、从左到右）排。",
			Schema: mcpObjectSchema(map[string]any{
				"ids": mcpStringArrayProp("要排列的 block；省略则用当前选择集"),
				"layout": map[string]any{
					"type": "string", "enum": []any{"row", "column", "grid"},
					"description": "排成一行、一列或网格",
				},
				"columns": map[string]any{
					"type": "integer", "minimum": 1,
					"description": "网格的列数；省略取 ceil(sqrt(n))",
				},
				"gapX":    map[string]any{"type": "number", "minimum": 0, "description": "列间距，默认 24"},
				"gapY":    map[string]any{"type": "number", "minimum": 0, "description": "行间距，默认 24"},
				"originX": map[string]any{"type": "number", "description": "左上角 X；省略用这组 block 外框的 X"},
				"originY": map[string]any{"type": "number", "description": "左上角 Y；省略用这组 block 外框的 Y"},
				"order": map[string]any{
					"type": "string", "enum": []any{"ids", "position"},
					"description": "ids（默认）按 ids 顺序，position 按画布位置",
				},
			}),
			Call: func(d *deck, args mcpArgs) (any, error) { return d.mcpArrangeBlocks(args) },
		},
		{
			Name:        "select_blocks",
			Title:       "设置选择集",
			Description: "改选择集，让窗口里选中 agent 正在处理的 block；mode 为 replace（默认）、add 或 toggle。",
			Schema: mcpObjectSchema(map[string]any{
				"ids": mcpStringArrayProp("要选中的 block ID"),
				"mode": map[string]any{
					"type": "string", "enum": []any{"replace", "add", "toggle"},
					"description": "replace 换成这些，add 加上，toggle 取反",
				},
			}, "ids"),
			Call: func(d *deck, args mcpArgs) (any, error) {
				ids, err := args.strings("ids")
				if err != nil {
					return nil, err
				}
				mode, _, err := args.str("mode")
				if err != nil {
					return nil, err
				}
				return d.mcpSelectBlocks(ids, mode)
			},
		},
		{
			Name:        "undo",
			Title:       "撤销",
			Description: "撤销上一步画布操作（加、删 block），与窗口里的 ⌘Z 同一条撤销栈。",
			Schema:      mcpObjectSchema(map[string]any{}),
			Call: func(d *deck, _ mcpArgs) (any, error) {
				d.undoLast()
				d.requestSave()
				return map[string]any{"ok": true, "blocks": len(d.blocks)}, nil
			},
		},
		{
			Name:        "redo",
			Title:       "重做",
			Description: "重做上一步撤销。",
			Schema:      mcpObjectSchema(map[string]any{}),
			Call: func(d *deck, _ mcpArgs) (any, error) {
				d.redoLast()
				d.requestSave()
				return map[string]any{"ok": true, "blocks": len(d.blocks)}, nil
			},
		},
		{
			Name:        "save_deck",
			Title:       "保存画布",
			Description: "立刻把画布写进工作区的 .boundlessdeck/deck.json；写操作本来就会自动保存，这个用于确认。",
			Schema:      mcpObjectSchema(map[string]any{}),
			Call: func(d *deck, _ mcpArgs) (any, error) {
				d.save()
				return map[string]any{"saved": d.deckFile(), "blocks": len(d.blocks)}, nil
			},
		},
	}
}

// mcpBridgeCommand is the stdio command a project config lists: the same
// binary, in its bridge mode, pointed at the workspace.
func mcpBridgeCommand(exe, root string) []string {
	return []string{"mcp-stdio", "--workspace", root}
}
