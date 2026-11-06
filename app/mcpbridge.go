package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// mcpBridgeMain runs the stdio side of the MCP: an agent starts this binary
// as `boundlessdeck mcp-stdio`, and it relays newline-delimited JSON-RPC to
// the running app's Unix socket. With --workspace DIR (what project configs
// list) it talks to that folder's deck; without one (what the global
// configs list, mcpglobal.go) it follows the deck the app has open, so an
// agent in any folder reaches it. No port or token is kept in the configs.
//
// It returns handled=false when the arguments are not the bridge's, and the
// process should start the app.
func mcpBridgeMain(argv []string) (handled bool, code int) {
	if len(argv) == 0 {
		return false, 0
	}
	switch argv[0] {
	case "mcp-stdio", "--mcp-stdio":
	default:
		return false, 0
	}
	fs := flag.NewFlagSet("mcp-stdio", flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	workspace := fs.String("workspace", "", "工作区文件夹")
	fs.StringVar(workspace, "w", "", "工作区文件夹")
	socket := fs.String("socket", "", "直接指定 Unix socket")
	token := fs.String("token", "", "直接指定 token")
	check := fs.Bool("check", false, "只检查连接，不转发")
	if err := fs.Parse(argv[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
		return true, 2
	}
	if *workspace == "" {
		*workspace = os.Getenv("BOUNDLESSDECK_WORKSPACE")
	}
	// find is where the deck's endpoint is now: the workspace's
	// descriptor, or the app's active deck.
	find := activeEndpoint
	if *workspace != "" {
		workspacePath, err := filepath.Abs(*workspace)
		if err != nil {
			fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
			return true, 2
		}
		find = func() (mcpEndpoint, error) { return readMCPEndpoint(workspacePath) }
	}
	ep := mcpEndpoint{Transport: "unix", Socket: *socket, Token: *token}
	if ep.Socket == "" || ep.Token == "" {
		fromFile, err := find()
		if err != nil {
			if *check {
				fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
				return true, 1
			}
			// Keep answering: the app may open the deck later, and the
			// config would otherwise have to change.
			fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
		}
		if ep.Socket == "" {
			ep.Socket, ep.Transport = fromFile.Socket, fromFile.Transport
		}
		if ep.Token == "" {
			ep.Token = fromFile.Token
		}
	}
	if *socket != "" {
		find = nil // pinned by hand
	}
	if *check {
		status, err := mcpHealth(ep)
		if err != nil {
			fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
			return true, 1
		}
		fmt.Println(string(status))
		return true, 0
	}
	if err := bridgeLoop(os.Stdin, os.Stdout, ep, find); err != nil {
		fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
		return true, 1
	}
	return true, 0
}

// bridgeLoop relays newline-delimited JSON-RPC from in to the deck and its
// answers back to out. When the deck does not answer, find (nil for a
// pinned endpoint) says where it is now: the app may have opened it, or
// another deck, since. With no deck to reach, the handshake and the lists
// are answered here, from the same tables the app serves, so an agent
// started before the app still gets its tools; calling one says why it
// cannot run.
func bridgeLoop(in io.Reader, out io.Writer, ep mcpEndpoint, find func() (mcpEndpoint, error)) error {
	reader := bufio.NewReaderSize(in, 1<<20)
	writer := bufio.NewWriter(out)
	for {
		line, readErr := reader.ReadBytes('\n')
		line = bytes.TrimSpace(line)
		if len(line) > 0 {
			status, body, err := postMCP(ep, line)
			if (err != nil || status == http.StatusUnauthorized) && find != nil {
				if now, ferr := find(); ferr == nil && (now.Socket != ep.Socket || now.Token != ep.Token) {
					ep = now
					status, body, err = postMCP(ep, line)
				} else if ferr != nil {
					err = ferr
				}
			}
			switch {
			case err != nil:
				if answer := offlineAnswer(line, err); answer != nil {
					_, _ = writer.Write(answer)
					_ = writer.WriteByte('\n')
				}
			case status == http.StatusAccepted || len(body) == 0:
				// A notification: no answer.
			case status != http.StatusOK:
				writeRPCError(writer, rpcIDOrNull(line), -32000,
					fmt.Sprintf("BoundlessDeck 返回 %d：%s", status, strings.TrimSpace(string(body))))
			default:
				_, _ = writer.Write(body)
				if !bytes.HasSuffix(body, []byte("\n")) {
					_ = writer.WriteByte('\n')
				}
			}
			if err := writer.Flush(); err != nil {
				return err
			}
		}
		if readErr != nil {
			if readErr == io.EOF {
				return nil
			}
			return readErr
		}
	}
}

// offlineAnswer answers a message while no deck answers (why): the
// handshake and the lists as the app would, a tool call with an error
// result the agent reads, anything else with a JSON-RPC error.
// Notifications get nothing.
func offlineAnswer(line []byte, why error) []byte {
	var req rpcRequest
	if err := json.Unmarshal(line, &req); err != nil {
		return nil
	}
	id, ok := requestID(req.ID)
	if !ok {
		return nil
	}
	msg := "BoundlessDeck 没有回应：" + why.Error()
	if errors.Is(why, errNoActiveDeck) {
		msg = why.Error()
	}
	switch req.Method {
	case "initialize", "ping", "tools/list", "resources/list", "resources/templates/list", "prompts/list", "logging/setLevel":
		result, rerr := (&mcpServer{}).dispatch(req)
		if rerr == nil {
			return mcpMarshal(rpcResponse{JSONRPC: "2.0", ID: id, Result: result})
		}
	case "tools/call":
		return mcpMarshal(rpcResponse{JSONRPC: "2.0", ID: id, Result: mcpToolFailure(errors.New(msg))})
	}
	return mcpMarshal(rpcResponse{JSONRPC: "2.0", ID: id, Error: &rpcError{Code: -32000, Message: msg}})
}

func writeRPCError(w *bufio.Writer, id json.RawMessage, code int, message string) {
	msg, err := json.Marshal(map[string]any{
		"jsonrpc": "2.0",
		"id":      id,
		"error":   map[string]any{"code": code, "message": message},
	})
	if err != nil {
		return
	}
	_, _ = w.Write(msg)
	_ = w.WriteByte('\n')
}

// rpcIDOrNull is the id of a message, for an error answer that must carry
// something: null when the message had none.
func rpcIDOrNull(line []byte) json.RawMessage {
	if id, ok := rpcMessageID(line); ok {
		return id
	}
	return json.RawMessage("null")
}

// rpcMessageID reads the id of a JSON-RPC message, if it has one.
func rpcMessageID(line []byte) (json.RawMessage, bool) {
	var probe struct {
		ID json.RawMessage `json:"id"`
	}
	if err := json.Unmarshal(line, &probe); err != nil {
		return nil, false
	}
	return requestID(probe.ID)
}

// mcpHealth asks the running app how it is.
func mcpHealth(ep mcpEndpoint) ([]byte, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "http://boundlessdeck/health", nil)
	if err != nil {
		return nil, err
	}
	if ep.Token != "" {
		req.Header.Set("X-BoundlessDeck-Token", ep.Token)
	}
	resp, err := mcpClient(ep).Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return nil, err
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("返回 %d：%s", resp.StatusCode, strings.TrimSpace(string(body)))
	}
	return body, nil
}

// postMCP sends one JSON-RPC message to the deck.
func postMCP(ep mcpEndpoint, message []byte) (int, []byte, error) {
	if ep.Socket == "" {
		return 0, nil, fmt.Errorf("工作区还没有 MCP 端点（先在 BoundlessDeck 里打开它）")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://boundlessdeck/mcp", bytes.NewReader(message))
	if err != nil {
		return 0, nil, err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	if ep.Token != "" {
		req.Header.Set("X-BoundlessDeck-Token", ep.Token)
	}
	resp, err := mcpClient(ep).Do(req)
	if err != nil {
		return 0, nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, mcpMaxBody))
	if err != nil {
		return 0, nil, err
	}
	return resp.StatusCode, body, nil
}

// mcpClient is an HTTP client whose requests go over the deck's Unix
// socket: no port is opened, and only this user can reach it. An endpoint
// on tcp (a platform without Unix sockets) dials that instead.
func mcpClient(ep mcpEndpoint) *http.Client {
	network := ep.Transport
	if network == "" {
		network = "unix"
	}
	address := ep.Socket
	transport := &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			dialer := &net.Dialer{Timeout: 5 * time.Second}
			return dialer.DialContext(ctx, network, address)
		},
	}
	return &http.Client{Transport: transport}
}
