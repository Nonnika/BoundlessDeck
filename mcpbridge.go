package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
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

// mcpBridgeMain runs the stdio side of the workspace's MCP: an agent starts
// this binary as `boundlessdeck mcp-stdio --workspace DIR`, and it relays
// newline-delimited JSON-RPC to the running app's Unix socket. The configs
// a workspace gets (mcpworkspace.go) point here, so no port or token has to
// be kept up to date in them.
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
	if *workspace == "" {
		*workspace, _ = os.Getwd()
	}
	workspacePath, err := filepath.Abs(*workspace)
	if err != nil {
		fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
		return true, 2
	}
	ep := mcpEndpoint{Transport: "unix", Socket: *socket, Token: *token}
	if ep.Socket == "" || ep.Token == "" {
		fromFile, err := readMCPEndpoint(workspacePath)
		if err != nil {
			if *check {
				fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
				return true, 1
			}
			// Keep answering: the app may open the workspace later, and
			// the config would otherwise have to change.
			fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
		}
		if ep.Socket == "" {
			ep.Socket, ep.Transport = fromFile.Socket, fromFile.Transport
		}
		if ep.Token == "" {
			ep.Token = fromFile.Token
		}
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
	if err := bridgeLoop(os.Stdin, os.Stdout, workspacePath, ep); err != nil {
		fmt.Fprintln(os.Stderr, "boundlessdeck mcp-stdio：", err)
		return true, 1
	}
	return true, 0
}

// bridgeLoop relays newline-delimited JSON-RPC from in to the deck and its
// answers back to out. A request the app cannot take gets a JSON-RPC error,
// so the agent sees why instead of waiting forever.
func bridgeLoop(in io.Reader, out io.Writer, workspace string, ep mcpEndpoint) error {
	reader := bufio.NewReaderSize(in, 1<<20)
	writer := bufio.NewWriter(out)
	for {
		line, readErr := reader.ReadBytes('\n')
		line = bytes.TrimSpace(line)
		if len(line) > 0 {
			if ep.Socket == "" {
				// No endpoint yet: try to find one, as the app may have
				// opened the workspace since this bridge started.
				if fromFile, err := readMCPEndpoint(workspace); err == nil {
					ep = fromFile
				}
			}
			status, body, err := postMCP(ep, line)
			switch {
			case err != nil:
				if id, ok := rpcMessageID(line); ok {
					writeRPCError(writer, id, -32000, "BoundlessDeck 没有回应："+err.Error())
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
