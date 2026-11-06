package main

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

func readFile(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// A new 笔记 is a Markdown file in the deck's folder, named after it, and
// is there to be written in; what is typed goes to the file.
func TestANewMemoIsAMarkdownFileOfTheDeck(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 1100, 700)
	tt.Frame()
	if err := tt.Click("笔记"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	n := d.blocks[len(d.blocks)-1]
	if n.Kind != kindMemo || n.Path != "笔记.md" || n.Title != "笔记" {
		t.Fatalf("new memo = %+v", n)
	}
	if d.editing != n || d.editPart != editBody {
		t.Fatal("a new memo is not being written in")
	}
	if got := readFile(t, filepath.Join(d.root, "笔记.md")); got != "" {
		t.Fatalf("a new memo's file holds %q", got)
	}

	n.rt.memo = "# 计划\n\n- 第一步"
	tt.Frame()
	d.stopEditing()
	if got := readFile(t, filepath.Join(d.root, "笔记.md")); got != n.rt.memo {
		t.Fatalf("the file holds %q after the edit", got)
	}
	tt.Frame()
	if !tt.HasText("计划") {
		t.Fatalf("the memo does not show its Markdown: %v", tt.Texts())
	}

	// Another one does not take the first one's file.
	second := d.newMemo(500, 300)
	if second.Path != "笔记 2.md" {
		t.Fatalf("second memo at %q", second.Path)
	}
}

// Typing is written once it rests, while the edit goes on.
func TestAMemoSavesWhileTyping(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 1100, 700)
	n := d.newMemo(500, 300)
	tt.Frame()
	n.rt.memo = "草稿"
	tt.Frame()
	n.rt.memoEdited = time.Now().Add(-memoSaveIdle) // typing rested
	tt.Frame()
	if d.editing != n {
		t.Fatal("the edit ended")
	}
	if got := readFile(t, d.path(n)); got != "草稿" {
		t.Fatalf("mid-edit the file holds %q", got)
	}
}

// Renaming a memo renames its file, never over another one; taking it off
// the canvas writes what was typed and leaves the file.
func TestAMemoIsItsFile(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 1100, 700)
	n := d.newMemo(500, 300)
	tt.Frame()
	d.stopEditing()

	d.startEditing(n, editTitle)
	n.Title = "会议: 周五"
	d.stopEditing()
	if n.Path != "会议- 周五.md" || n.Title != "会议- 周五" {
		t.Fatalf("renamed memo: path %q title %q", n.Path, n.Title)
	}
	if _, err := os.Stat(filepath.Join(d.root, "笔记.md")); !os.IsNotExist(err) {
		t.Fatal("the old file is still there")
	}

	if err := os.WriteFile(filepath.Join(d.root, "taken.md"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	d.startEditing(n, editTitle)
	n.Title = "taken"
	d.stopEditing()
	if n.Path != "会议- 周五.md" || readFile(t, filepath.Join(d.root, "taken.md")) != "x" {
		t.Fatalf("renamed over another file: %q", n.Path)
	}

	tt.Frame()
	n.rt.memo = "最后一句"
	d.removeBlock(n)
	if got := readFile(t, filepath.Join(d.root, "会议- 周五.md")); got != "最后一句" {
		t.Fatalf("removed, the file holds %q", got)
	}
}

// What else writes to the file shows in the block.
func TestAMemoShowsWhatElseWroteItsFile(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 1100, 700)
	n := d.newMemo(500, 300)
	tt.Frame()
	d.stopEditing()
	tt.Frame()

	later := time.Now().Add(time.Second)
	path := d.path(n)
	if err := os.WriteFile(path, []byte("## 来自 agent"), 0o644); err != nil {
		t.Fatal(err)
	}
	os.Chtimes(path, later, later)
	n.rt.memoCheck = time.Time{} // due
	tt.Frame()
	tt.Frame()
	if n.rt.memo != "## 来自 agent" || !tt.HasText("来自 agent") {
		t.Fatalf("the memo shows %q", n.rt.memo)
	}
}

// Through the MCP: a memo is made with its file, its text written to it.
func TestMCPCreatesAMemoFile(t *testing.T) {
	s, d := mcpTestServer(t)
	out := toolCall(t, s, "create_block", map[string]any{
		"kind": "memo", "title": "大纲", "text": "# 第一章",
	})
	if out["path"] != "大纲.md" || out["absPath"] != filepath.Join(d.root, "大纲.md") {
		t.Fatalf("create_block memo = %v", out)
	}
	if got := readFile(t, filepath.Join(d.root, "大纲.md")); got != "# 第一章" {
		t.Fatalf("the memo's file holds %q", got)
	}

	id := out["id"].(string)
	toolCall(t, s, "update_block", map[string]any{"id": id, "text": "# 第二章", "title": "终稿"})
	if got := readFile(t, filepath.Join(d.root, "终稿.md")); got != "# 第二章" {
		t.Fatalf("after update the file holds %q", got)
	}

	if _, isErr, _ := toolCallRaw(t, s, "create_block", map[string]any{"kind": "memo", "path": "notes.txt"}); !isErr {
		t.Fatal("a memo at a .txt path was made")
	}
}
