package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

func TestOnlyBlocksTheWindowShowsAreBuilt(t *testing.T) {
	d := testDeck(t)
	d.blocks = append(d.blocks,
		// 30 DIPs right of the window: nothing of it shows.
		&block{ID: "right", Kind: kindNote, Title: "R", X: 930, Y: 100, W: 200, H: 120},
		// Just below the window, its tape poking up into it.
		&block{ID: "tape", Kind: kindNote, Title: "T", X: 300, Y: 610, W: 200, H: 120},
		// Far off.
		&block{ID: "far", Kind: kindNote, Title: "F", X: 3000, Y: 3000, W: 200, H: 120},
	)
	built := map[string]bool{}
	for _, n := range d.visible(900, 600) {
		built[n.ID] = true
	}
	if built["right"] || built["far"] {
		t.Fatalf("blocks out of view were built: %v", built)
	}
	if !built["tape"] || !built["a"] || !built["b"] {
		t.Fatalf("blocks in view were not built: %v", built)
	}
}

func TestAStackLoadsOnlyTheCardsWhoseFaceShows(t *testing.T) {
	d, st := stackDeck(t)
	var ids []string
	for _, id := range []string{"m1", "m2", "m3", "m4", "m5"} {
		name := id + ".md"
		if err := os.WriteFile(filepath.Join(d.root, name), []byte("# "+id), 0o644); err != nil {
			t.Fatal(err)
		}
		d.addBlock(&block{ID: id, Kind: kindMemo, Title: id, Path: name, X: 100, Y: 1000, W: 200, H: 120})
		ids = append(ids, id)
	}
	st.Items = ids
	d.syncStacked()
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	tt.Frame()

	// Cards m1–m4 show their headers alone, m5 in front shows whole.
	memoLoads := func() []string {
		var out []string
		for _, k := range d.loads.started {
			if strings.HasPrefix(k, "memo:") {
				out = append(out, k)
			}
		}
		return out
	}
	if got := memoLoads(); len(got) != 1 || got[0] != "memo:m5" {
		t.Fatalf("the stack loaded %v, want only the card in front, memo:m5", got)
	}

	// Pushing m2 out shows its face: it loads then.
	tt.ClickAt(700, 106+50)
	for range 12 {
		tt.Advance(50 * time.Millisecond)
		tt.Frame()
	}
	if got := memoLoads(); !containsID(got, "memo:m2") || containsID(got, "memo:m1") {
		t.Fatalf("after pushing m2 out the stack loaded %v, want m2 and not m1", got)
	}
}
