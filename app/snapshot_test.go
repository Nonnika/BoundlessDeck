package main

import (
	"image/png"
	"os"
	"path/filepath"
	"testing"

	"github.com/egoist/mygo/ui"
)

// TestSnapshot renders the canvas to docs/canvas-snapshot.png, for reviewing
// a change to how the canvas paints: DECK_SNAPSHOT=1 go test -run TestSnapshot .
func TestSnapshot(t *testing.T) {
	if os.Getenv("DECK_SNAPSHOT") == "" {
		t.Skip("set DECK_SNAPSHOT=1 to write docs/canvas-snapshot.png")
	}
	d := testDeck(t)
	if err := os.WriteFile(filepath.Join(d.root, "notes.md"), []byte("# Hello\n\nA linked file previews here.\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	d.blocks = append(d.blocks,
		&block{ID: "c", Kind: kindFile, Title: "notes.md", Path: "notes.md", X: 360, Y: 60, W: 460, H: 320},
	)
	d.blocks[0].W, d.blocks[0].H = 240, 200
	d.blocks[0].Text = "## 待办\n\n- [x] 画布\n- [ ] **QuickLook** 支持 `Markdown`\n\n> 点击编辑"
	d.sel = map[string]bool{"a": true}
	tt := ui.NewTester(d.view, 1100, 700)
	tt.Frame()
	img := tt.Image()
	if err := os.MkdirAll(filepath.Join("..", "docs"), 0o755); err != nil {
		t.Fatal(err)
	}
	f, err := os.Create(filepath.Join("..", "docs", "canvas-snapshot.png"))
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	if err := png.Encode(f, img); err != nil {
		t.Fatal(err)
	}
}
