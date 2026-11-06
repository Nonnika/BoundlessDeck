package main

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// stackDeck is a deck with a 块堆 holding the two notes testDeck makes.
func stackDeck(t *testing.T) (*deck, *block) {
	t.Helper()
	d := testDeck(t)
	st := &block{ID: "s", Kind: kindStack, Title: "堆", X: 600, Y: 80, W: 280, H: 260, Items: []string{"a", "b"}}
	d.addBlock(st)
	d.undo = nil
	return d, st
}

func TestDroppingABlockOnAStackHidesItAndUndoes(t *testing.T) {
	d := testDeck(t)
	st := &block{ID: "s", Kind: kindStack, Title: "堆", X: 600, Y: 80, W: 280, H: 260}
	d.addBlock(st)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	// A's title bar is at (100,80)–(300,106): drag it onto the pile.
	tt.Press(120, 90)
	tt.Move(700, 200)
	tt.Release(700, 200)

	if len(st.Items) != 1 || st.Items[0] != "a" {
		t.Fatalf("stack items = %v, want [a]", st.Items)
	}
	if !d.stackedIDs()["a"] {
		t.Fatal("the dragged block is not hidden")
	}
	for _, n := range d.visible(900, 600) {
		if n.ID == "a" {
			t.Fatal("a hidden block is still built on the canvas")
		}
	}
	if !d.sel["s"] {
		t.Fatalf("after the drop the selection is %v, want the stack", d.sel)
	}

	d.undoLast()
	if len(st.Items) != 0 {
		t.Fatalf("undo left the stack holding %v", st.Items)
	}
	found := false
	for _, n := range d.visible(900, 600) {
		found = found || n.ID == "a"
	}
	if !found {
		t.Fatal("undo did not put the block back on the canvas")
	}
}

func TestClickingAStackPushesTheHoveredCardOut(t *testing.T) {
	d, st := stackDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	rt := d.ensureRuntime(st)

	// Hovering a pile not opened does nothing.
	tt.Move(700, 200)
	tt.Frame()
	if rt.stackOut {
		t.Fatal("hovering a stack not clicked pushed a card out")
	}

	// The body starts at y 106: card a's header at 118, card b's at 150.
	tt.ClickAt(700, 200)
	if d.stackOpen != st || !d.sel["s"] {
		t.Fatalf("clicking the stack opened %v, selected %v", d.stackOpen, d.sel)
	}
	if !rt.stackOut || rt.stackPick != 1 {
		t.Fatalf("after a click card %d is out (%v), want card 1", rt.stackPick, rt.stackOut)
	}

	// Hovering a's header pushes a out, and b goes back.
	tt.Move(700, 128)
	tt.Frame()
	if !rt.stackOut || rt.stackPick != 0 {
		t.Fatalf("hovering a's header has card %d out (%v), want card 0", rt.stackPick, rt.stackOut)
	}

	// Space previews the card out.
	tt.Key(0, ui.KeySpace)
	tt.Frame()
	if d.previewStack != st || !tt.HasText("第 1 / 2 张") {
		t.Fatalf("space opened %v: %v", d.previewStack, tt.Texts())
	}
	tt.Key(0, ui.KeySpace)
	tt.Frame()

	// A click elsewhere takes the focus and the selection: the card goes back.
	tt.ClickAt(100, 520)
	tt.Frame()
	if d.stackOpen != nil || rt.stackOut {
		t.Fatalf("after a click elsewhere the stack is open %v, card out %v", d.stackOpen, rt.stackOut)
	}
}

func TestHoveringAStackEdgeScrollsItsCards(t *testing.T) {
	d, st := stackDeck(t)
	for _, id := range []string{"c", "e", "f", "g"} {
		d.addBlock(&block{ID: id, Kind: kindNote, Title: id, X: 100, Y: 1000, W: 200, H: 120})
	}
	st.Items = []string{"a", "b", "c", "e", "f", "g"}
	d.syncStacked()
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	rt := d.ensureRuntime(st)

	tt.ClickAt(700, 200)
	// The bottom edge of the body (106–340) scrolls on while hovered.
	tt.Move(700, 330)
	for range 20 {
		tt.Advance(50 * time.Millisecond)
		tt.Frame()
	}
	if want := d.stackGeomOf(st, 6).max; rt.stackScroll < want-0.01 {
		t.Fatalf("hovering the bottom edge scrolled to %v, want %v", rt.stackScroll, want)
	}
	if rt.stackOut {
		t.Fatal("a card stayed out while the cards scrolled")
	}

	// The top edge scrolls back.
	tt.Move(700, 110)
	for range 20 {
		tt.Advance(50 * time.Millisecond)
		tt.Frame()
	}
	if rt.stackScroll > 0.01 {
		t.Fatalf("hovering the top edge left the cards at %v, want 0", rt.stackScroll)
	}

	// The wheel scrolls them too, rather than the canvas.
	cam := d.cam
	tt.Scroll(700, 200, 0, 64)
	tt.Frame()
	if rt.stackScroll < 1.9 || d.cam != cam {
		t.Fatalf("the wheel scrolled the cards to %v and the camera %v -> %v", rt.stackScroll, cam, d.cam)
	}
}

func TestSpaceOpensALastingStackPreview(t *testing.T) {
	d, st := stackDeck(t)
	d.sel = map[string]bool{"s": true}
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	tt.Key(0, ui.KeySpace)
	if d.previewStack != st {
		t.Fatalf("space opened %v, want the selected stack", d.previewStack)
	}
	if !tt.HasText("第 1 / 2 张") {
		t.Fatalf("the preview does not say which card it shows: %v", tt.Texts())
	}

	// The preview is long: it stays frame after frame, until space again.
	tt.Frame()
	if d.previewStack != st {
		t.Fatal("the preview closed by itself")
	}

	// Its buttons work: turn a card, take one out, close.
	if err := tt.Click("›"); err != nil {
		t.Fatalf("next button: %v", err)
	}
	tt.Frame()
	if got := d.ensureRuntime(st).stackPick; got != 1 {
		t.Fatalf("the next button shows card %d, want 1", got)
	}
	if err := tt.Click("取出"); err != nil {
		t.Fatalf("take button: %v", err)
	}
	tt.Frame()
	if len(st.Items) != 1 || !d.stackedIDs()["a"] {
		t.Fatalf("taking one out left %v, want [a] on the canvas", st.Items)
	}
	if err := tt.Click("关闭"); err != nil {
		t.Fatalf("close button: %v", err)
	}
	if d.previewStack != nil {
		t.Fatal("the close button left the preview open")
	}

	// Space opens it again once the stack is selected.
	d.sel = map[string]bool{"s": true}
	tt.Key(0, ui.KeySpace)
	if d.previewStack != st {
		t.Fatal("space did not reopen the preview")
	}
	tt.Key(0, ui.KeySpace)
	if d.previewStack != nil {
		t.Fatal("space did not close the preview")
	}

	tt.Key(0, ui.KeySpace)
	tt.Frame()
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if d.previewStack != nil {
		t.Fatal("escape did not close the preview")
	}
}

func TestStackItemsPersistAndLoad(t *testing.T) {
	d, st := stackDeck(t)
	d.save()

	next := newDeck()
	next.root, next.file = d.root, filepath.Join(d.root, "deck.json")
	next.load()

	var got *block
	for _, n := range next.blocks {
		if n.Kind == kindStack {
			got = n
		}
	}
	if got == nil {
		t.Fatal("the stack did not load")
	}
	if len(got.Items) != 2 || got.Items[0] != "a" || got.Items[1] != "b" {
		t.Fatalf("loaded stack items = %v, want [a b]", got.Items)
	}
	if !next.stackedIDs()["a"] {
		t.Fatal("a loaded block of the stack is not hidden")
	}
	_ = st
}

func TestMCPCreatesAndUpdatesStackItems(t *testing.T) {
	s, d := mcpTestServer(t)
	out := toolCall(t, s, "create_block", map[string]any{
		"kind": "stack", "title": "堆", "items": []string{"a", "b"}, "x": 600, "y": 80,
	})
	id, _ := out["id"].(string)
	if id == "" {
		t.Fatalf("create_block made no stack: %v", out)
	}
	if items, _ := out["items"].([]any); len(items) != 2 {
		t.Fatalf("created stack items = %v, want a and b", out["items"])
	}
	if !d.stackedIDs()["a"] || !d.stackedIDs()["b"] {
		t.Fatal("the created stack does not hide its items")
	}

	out = toolCall(t, s, "update_block", map[string]any{"id": id, "items": []string{"b"}})
	if items, _ := out["items"].([]any); len(items) != 1 || items[0] != "b" {
		t.Fatalf("updated stack items = %v, want [b]", out["items"])
	}

	if _, isErr, _ := toolCallRaw(t, s, "update_block", map[string]any{
		"id": id, "items": []string{"nope"},
	}); !isErr {
		t.Fatal("an unknown block id in items was accepted")
	}

	second := toolCall(t, s, "create_block", map[string]any{"kind": "stack", "x": 1000, "y": 80})
	secondID, _ := second["id"].(string)
	if _, isErr, _ := toolCallRaw(t, s, "update_block", map[string]any{
		"id": id, "items": []string{secondID},
	}); !isErr {
		t.Fatal("a stack was allowed in a stack")
	}
}

func TestSpaceAfterTheToolbarStillOpensThePreview(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	// Make a stack from the toolbar: the button keeps the focus.
	if err := tt.Click("块堆"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	var st *block
	for _, n := range d.blocks {
		if n.Kind == kindStack {
			st = n
		}
	}
	if st == nil {
		t.Fatal("the toolbar made no stack")
	}
	st.Items = []string{"a"} // a block to preview
	tt.Frame()

	// Clicking the pile moves the focus to it; space then opens the
	// preview rather than pressing the toolbar button again.
	tt.ClickAt(st.X+40, st.Y+80)
	tt.Frame()
	tt.Key(0, ui.KeySpace)
	tt.Frame()
	if d.previewStack != st {
		t.Fatalf("space opened %v, want the stack clicked after the toolbar", d.previewStack)
	}
}

func TestDroppingABlockByItsTapeCollectsIt(t *testing.T) {
	d := testDeck(t)
	st := &block{ID: "s", Kind: kindStack, Title: "堆", X: 600, Y: 80, W: 280, H: 260}
	d.addBlock(st)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	// The washi tape is the handle most people grab: it sticks out above
	// the card's title bar.
	tp := d.tapeOf(d.blocks[0]).box()
	tt.Press(tp.X+tp.W/2, tp.Y+tp.H/2)
	tt.Move(700, 200)
	tt.Release(700, 200)

	if len(st.Items) != 1 || st.Items[0] != "a" {
		t.Fatalf("dragging by the tape left the stack holding %v, want [a]", st.Items)
	}
	if !d.stackedIDs()["a"] {
		t.Fatal("the tape drop did not hide the block")
	}
}
