package main

import (
	"path/filepath"
	"strings"
	"testing"
)

// layoutDeck is a deck of notes at given places, for the layout tests.
func layoutDeck(t *testing.T, blocks ...*block) (*mcpServer, *deck) {
	t.Helper()
	d := newDeck()
	d.root = t.TempDir()
	d.file = filepath.Join(d.root, "deck.json")
	d.blocks = blocks
	return &mcpServer{d: d, token: "t"}, d
}

func note(id string, x, y, w, h float32) *block {
	return &block{ID: id, Kind: kindNote, Title: id, X: x, Y: y, W: w, H: h}
}

func TestMCPAlignLeftRightAndCenters(t *testing.T) {
	s, d := layoutDeck(t,
		note("a", 100, 100, 200, 100),
		note("b", 400, 300, 300, 150),
		note("c", 700, 50, 100, 200),
	)

	toolCall(t, s, "align_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "align": "left",
	})
	for _, id := range []string{"a", "b", "c"} {
		if x := d.blockByID(id).X; x != 100 {
			t.Fatalf("%s.X = %v, want 100", id, x)
		}
	}
	// Y is untouched by a horizontal alignment.
	if d.blockByID("c").Y != 50 {
		t.Fatalf("align left moved Y: %v", d.blockByID("c").Y)
	}

	toolCall(t, s, "align_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "align": "centerX",
	})
	// After aligning left, the bounds are 100..400: every center is 250.
	for _, id := range []string{"a", "b", "c"} {
		n := d.blockByID(id)
		if c := n.X + n.W/2; abs(c-250) > 0.01 {
			t.Fatalf("%s center = %v, want 250", id, c)
		}
	}

	// Align to one block, which stays where it is.
	toolCall(t, s, "align_blocks", map[string]any{
		"ids": []string{"a", "c"}, "align": "right", "to": "b",
	})
	if got := d.blockByID("a"); got.X != 200 {
		t.Fatalf("a.X = %v, want 200 (b's right edge minus a's width)", got.X)
	}
	if got := d.blockByID("c"); got.X != 300 {
		t.Fatalf("c.X = %v, want 300", got.X)
	}
	if x := d.blockByID("b").X; x != 100 {
		t.Fatalf("the reference block moved to %v", x)
	}
}

func TestMCPAlignWithSpacingMakesAColumn(t *testing.T) {
	s, d := layoutDeck(t,
		note("a", 100, 100, 200, 100),
		note("b", 400, 300, 300, 150),
		note("c", 700, 50, 100, 200),
	)
	toolCall(t, s, "align_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "align": "left", "spacing": 20,
	})
	// Order by the Y they had: c (50), a (100), b (300).
	want := map[string]float32{"c": 50, "a": 270, "b": 390}
	for id, y := range want {
		n := d.blockByID(id)
		if n.X != 100 || n.Y != y {
			t.Fatalf("%s = (%v,%v), want (100,%v)", id, n.X, n.Y, y)
		}
	}
}

func TestMCPDistributeGaps(t *testing.T) {
	s, d := layoutDeck(t,
		note("a", 0, 0, 100, 50),
		note("b", 200, 0, 100, 50),
		note("c", 600, 0, 200, 50),
	)
	out := toolCall(t, s, "distribute_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "axis": "x",
	})
	if gap, _ := out["gap"].(float64); gap != 200 {
		t.Fatalf("gap = %v, want 200", out["gap"])
	}
	if d.blockByID("a").X != 0 || d.blockByID("b").X != 300 || d.blockByID("c").X != 600 {
		t.Fatalf("x = %v %v %v", d.blockByID("a").X, d.blockByID("b").X, d.blockByID("c").X)
	}

	// Vertical, by center: the middle block's center lands halfway.
	v, d2 := layoutDeck(t,
		note("a", 0, 0, 100, 100),
		note("b", 0, 250, 100, 100),
		note("c", 0, 1000, 100, 100),
	)
	toolCall(t, v, "distribute_blocks", map[string]any{
		"ids": []string{"b", "a", "c"}, "axis": "y", "mode": "centers",
	})
	if y := d2.blockByID("b").Y; y != 500 {
		t.Fatalf("b.Y = %v, want 500", y)
	}
}

func TestMCPDistributeNeedsRoom(t *testing.T) {
	s, d := layoutDeck(t,
		note("a", 0, 0, 500, 50),
		note("b", 100, 0, 500, 50),
		note("c", 200, 0, 500, 50),
	)
	if _, isErr, text := toolCallRaw(t, s, "distribute_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "axis": "x",
	}); !isErr || !strings.Contains(text, "排不下") {
		t.Fatalf("too-tight distribute: isErr=%v text=%q", isErr, text)
	}
	if _, isErr, _ := toolCallRaw(t, s, "distribute_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "axis": "x", "allowOverlap": true,
	}); isErr {
		t.Fatal("allowOverlap did not let the blocks overlap")
	}
	if d.blockByID("b").X != 100 {
		t.Fatalf("b.X = %v", d.blockByID("b").X)
	}
}

func TestMCPArrangeGrid(t *testing.T) {
	s, d := layoutDeck(t,
		note("a", 500, 500, 100, 50),
		note("b", 0, 0, 300, 100),
		note("c", 300, 900, 100, 200),
	)
	out := toolCall(t, s, "arrange_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "layout": "grid", "columns": 2, "gapX": 10, "gapY": 10,
	})
	if cols := out["columns"].(float64); cols != 2 {
		t.Fatalf("columns = %v", out["columns"])
	}
	// Origin is the old bounds' top-left (0,0); column 0 is 100 wide
	// (a, c), column 1 300 (b); row 0 100 tall (a, b), row 1 200 (c).
	want := map[string][2]float32{"a": {0, 0}, "b": {110, 0}, "c": {0, 110}}
	for id, p := range want {
		n := d.blockByID(id)
		if n.X != p[0] || n.Y != p[1] {
			t.Fatalf("%s = (%v,%v), want (%v,%v)", id, n.X, n.Y, p[0], p[1])
		}
	}
	// Sizes are kept.
	if n := d.blockByID("b"); n.W != 300 || n.H != 100 {
		t.Fatalf("b resized to %vx%v", n.W, n.H)
	}
}

func TestMCPArrangeRowByPosition(t *testing.T) {
	s, d := layoutDeck(t,
		note("a", 500, 500, 100, 50),
		note("b", 0, 0, 300, 100),
		note("c", 300, 900, 100, 200),
	)
	toolCall(t, s, "arrange_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "layout": "row", "order": "position", "gapX": 10,
	})
	// Left to right: b (0), c (300), a (500).
	want := map[string]float32{"b": 0, "c": 310, "a": 420}
	for id, x := range want {
		if n := d.blockByID(id); n.X != x || n.Y != 0 {
			t.Fatalf("%s = (%v,%v), want (%v,0)", id, n.X, n.Y, x)
		}
	}
}

func TestMCPLayoutUsesTheSelection(t *testing.T) {
	s, d := layoutDeck(t,
		note("a", 100, 100, 200, 100),
		note("b", 400, 300, 300, 150),
		note("c", 700, 50, 100, 200),
	)
	d.sel = map[string]bool{"b": true, "c": true}
	toolCall(t, s, "align_blocks", map[string]any{"align": "top"})
	if d.blockByID("b").Y != 50 || d.blockByID("c").Y != 50 {
		t.Fatalf("selection not aligned: b.Y=%v c.Y=%v", d.blockByID("b").Y, d.blockByID("c").Y)
	}
	if d.blockByID("a").Y != 100 {
		t.Fatalf("a block outside the selection moved: %v", d.blockByID("a").Y)
	}

	// With no ids and no selection, the call says what to do.
	d.sel = map[string]bool{}
	if _, isErr, text := toolCallRaw(t, s, "align_blocks", map[string]any{"align": "left"}); !isErr || !strings.Contains(text, "ids") {
		t.Fatalf("empty set: isErr=%v text=%q", isErr, text)
	}
}

func TestMCPLayoutIsUndoable(t *testing.T) {
	s, d := layoutDeck(t,
		note("a", 100, 100, 200, 100),
		note("b", 400, 300, 300, 150),
		note("c", 700, 50, 100, 200),
	)
	toolCall(t, s, "arrange_blocks", map[string]any{
		"ids": []string{"a", "b", "c"}, "layout": "row",
	})
	after := map[string][2]float32{}
	for _, id := range []string{"a", "b", "c"} {
		n := d.blockByID(id)
		after[id] = [2]float32{n.X, n.Y}
	}
	toolCall(t, s, "undo", map[string]any{})
	if n := d.blockByID("a"); n.X != 100 || n.Y != 100 {
		t.Fatalf("undo left a at (%v,%v)", n.X, n.Y)
	}
	if n := d.blockByID("b"); n.X != 400 || n.Y != 300 {
		t.Fatalf("undo left b at (%v,%v)", n.X, n.Y)
	}
	toolCall(t, s, "redo", map[string]any{})
	for id, p := range after {
		n := d.blockByID(id)
		if n.X != p[0] || n.Y != p[1] {
			t.Fatalf("redo left %s at (%v,%v), want (%v,%v)", id, n.X, n.Y, p[0], p[1])
		}
	}
}

func TestMCPMoveIsUndoable(t *testing.T) {
	s, d := layoutDeck(t, note("a", 100, 100, 200, 100), note("b", 400, 300, 300, 150))
	toolCall(t, s, "move_blocks", map[string]any{"ids": []string{"a", "b"}, "dx": 10, "dy": -5})
	if d.blockByID("a").X != 110 || d.blockByID("b").Y != 295 {
		t.Fatal("move_blocks did not move")
	}
	toolCall(t, s, "undo", map[string]any{})
	if d.blockByID("a").X != 100 || d.blockByID("b").Y != 300 {
		t.Fatalf("undo: a.X=%v b.Y=%v", d.blockByID("a").X, d.blockByID("b").Y)
	}
}

func TestMCPLayoutRejectsUnknownBlocks(t *testing.T) {
	s, d := layoutDeck(t, note("a", 0, 0, 100, 100), note("b", 200, 0, 100, 100))
	if _, isErr, _ := toolCallRaw(t, s, "align_blocks", map[string]any{
		"ids": []string{"a", "nope"}, "align": "top",
	}); !isErr {
		t.Fatal("an unknown id was accepted")
	}
	if d.blockByID("a").Y != 0 || d.blockByID("b").X != 200 {
		t.Fatal("a refused layout moved blocks")
	}
	if _, isErr, _ := toolCallRaw(t, s, "distribute_blocks", map[string]any{
		"ids": []string{"a", "b"}, "axis": "x",
	}); !isErr {
		t.Fatal("distributing two blocks was accepted")
	}
}
