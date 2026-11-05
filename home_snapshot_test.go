package main

import (
	"image/png"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// TestHomeSnapshot renders the home screen, light and dark, to
// docs/home-snapshot.png and docs/home-snapshot-dark.png:
// DECK_SNAPSHOT=1 go test -run TestHomeSnapshot .
func TestHomeSnapshot(t *testing.T) {
	if os.Getenv("DECK_SNAPSHOT") == "" {
		t.Skip("set DECK_SNAPSHOT=1 to write docs/home-snapshot.png")
	}
	a := testApp(t)
	now := time.Now()
	a.recents = []recent{
		{Path: "/Users/demo/Code/boundless", Blocks: 14, OpenedAt: now.Add(-3 * time.Minute)},
		{Path: "/Users/demo/Notes/research", Blocks: 6, OpenedAt: now.Add(-5 * time.Hour)},
		{Path: "/Users/demo/Old/archive", Blocks: 2, OpenedAt: now.Add(-72 * time.Hour), missing: true},
	}
	if err := os.MkdirAll("docs", 0o755); err != nil {
		t.Fatal(err)
	}
	for _, dark := range []bool{false, true} {
		tt := ui.NewTester(a.view, 1100, 760)
		tt.SetDark(dark)
		tt.Frame()
		name := "home-snapshot.png"
		if dark {
			name = "home-snapshot-dark.png"
		}
		f, err := os.Create(filepath.Join("docs", name))
		if err != nil {
			t.Fatal(err)
		}
		err = png.Encode(f, tt.Image())
		f.Close()
		if err != nil {
			t.Fatal(err)
		}
	}
}
