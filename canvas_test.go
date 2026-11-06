package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/egoist/mygo/plugins/terminal"
	"github.com/egoist/mygo/ui"
)

// testDeck is a deck with a fixed workspace root and no window.
func testDeck(t *testing.T) *deck {
	t.Helper()
	d := newDeck()
	d.root = t.TempDir()
	d.file = filepath.Join(d.root, "deck.json")
	d.blocks = []*block{
		{ID: "a", Kind: kindNote, Title: "A", X: 100, Y: 80, W: 200, H: 120},
		{ID: "b", Kind: kindNote, Title: "B", X: 400, Y: 300, W: 200, H: 120},
	}
	return d
}

func TestZoomKeepsThePointUnderThePointer(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	const px, py = 300, 200
	wx0, wy0 := d.cam.toWorld(px, py)
	tt.Pinch(px, py, 2)
	tt.Frame()
	wx1, wy1 := d.cam.toWorld(px, py)

	if abs(wx0-wx1) > 0.01 || abs(wy0-wy1) > 0.01 {
		t.Fatalf("the world point under the pointer moved: (%v,%v) -> (%v,%v)", wx0, wy0, wx1, wy1)
	}
	if d.cam.S < 1.9 || d.cam.S > 2.1 {
		t.Fatalf("scale = %v, want about 2", d.cam.S)
	}
}

func TestZoomStaysWithinBounds(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	for range 40 {
		tt.Pinch(800, 500, 2)
	}
	if d.cam.S != maxScale {
		t.Fatalf("scale = %v, want %v", d.cam.S, maxScale)
	}
	for range 80 {
		tt.Pinch(800, 500, 0.5)
	}
	if d.cam.S != minScale {
		t.Fatalf("scale = %v, want %v", d.cam.S, minScale)
	}
}

func TestWheelPansTheCamera(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	before := d.cam
	tt.Wheel(400, 300, 0, 100)
	if d.cam.Y <= before.Y {
		t.Fatalf("scrolling down moved the camera to %v, want past %v", d.cam.Y, before.Y)
	}
	if d.cam.Y-before.Y != 100 {
		t.Fatalf("the camera moved by %v, want 100", d.cam.Y-before.Y)
	}
}

// A Tester's wheel carries no modifiers, so the handler sees the event the
// window would give it with Cmd held.
func TestWheelWithCmdZooms(t *testing.T) {
	d := testDeck(t)
	before := d.cam.S
	ev := ui.InputEvent{Kind: ui.InputScroll, X: 400, Y: 300, DY: -100, Mods: ui.Cmd}
	if !d.onCanvasInput(ev) {
		t.Fatal("the canvas did not take the wheel")
	}
	if d.cam.S <= before {
		t.Fatalf("scale = %v, want more than %v", d.cam.S, before)
	}
}

func TestDraggingATitleBarMovesWithoutSelecting(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	tt.Press(120, 90) // in A's title bar: world (100, 80) is its corner
	tt.Move(170, 90)
	tt.Release(170, 90)

	if len(d.sel) != 0 {
		t.Fatalf("a drag selected %v", d.sel)
	}
	if d.editing != nil {
		t.Fatal("a drag of the title bar started an edit")
	}
	if d.blocks[0].X != 150 || d.blocks[0].Y != 80 {
		t.Fatalf("block moved to (%v,%v), want (150,80)", d.blocks[0].X, d.blocks[0].Y)
	}
}

func TestShiftClickSelectsWithoutMoving(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	tt.ClickAtWith(ui.Shift, 150, 140) // A's body, below its title bar
	tt.Frame()

	if !d.sel["a"] {
		t.Fatal("Shift+click did not select the block")
	}
	if d.editing != nil {
		t.Fatal("Shift+click started an edit")
	}
	if d.blocks[0].X != 100 || d.blocks[0].Y != 80 {
		t.Fatalf("the body dragged the block to (%v,%v)", d.blocks[0].X, d.blocks[0].Y)
	}
}

func TestMarqueeSelectsTheBlocksInside(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	tt.Press(700, 200) // empty canvas
	tt.Release(700, 200)
	if len(d.sel) != 0 {
		t.Fatalf("a press on empty canvas selected %v", d.sel)
	}

	tt.Press(20, 20)
	tt.Move(380, 280)
	tt.Release(380, 280)
	if !d.sel["a"] || d.sel["b"] {
		t.Fatalf("the marquee selected %v, want just a", d.sel)
	}
}

func TestLowZoomBuildsNoBodies(t *testing.T) {
	d := testDeck(t)
	path := filepath.Join(d.root, "hello.txt")
	if err := os.WriteFile(path, []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}
	d.blocks = append(d.blocks, &block{ID: "f", Kind: kindFile, Title: "hello.txt", Path: "hello.txt", X: 700, Y: 80, W: 300, H: 200})
	d.cam.S = 0.2

	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	if d.blocks[2].rt != nil {
		t.Fatal("a file block below the LOD threshold loaded its body")
	}

	d.cam.S = 1
	tt.Frame()
	if d.blocks[2].rt == nil || d.blocks[2].rt.text == "" {
		t.Fatal("the file block did not load its text above the LOD threshold")
	}
}

func TestWorkspaceRoundTrip(t *testing.T) {
	d := testDeck(t)
	d.cam = camera{X: -120, Y: 40, S: 1.5}
	d.save()

	other := newDeck()
	other.file = d.file
	other.load()

	if other.cam != d.cam {
		t.Fatalf("camera came back as %+v, want %+v", other.cam, d.cam)
	}
	if len(other.blocks) != len(d.blocks) || other.blocks[0].X != 100 || other.blocks[0].Title != "A" {
		t.Fatalf("blocks came back as %+v", other.blocks)
	}
}

func TestAddBlockSelectsItAndUndoes(t *testing.T) {
	d := testDeck(t)

	b := d.addBlock(&block{Kind: kindNote, Title: "C", X: 900, Y: 700, W: 100, H: 100})
	if len(d.blocks) != 3 {
		t.Fatalf("%d blocks after adding one, want 3", len(d.blocks))
	}
	if !d.sel[b.ID] || len(d.sel) != 1 {
		t.Fatalf("selection is %v, want just the new block", d.sel)
	}

	d.undoLast()
	if len(d.blocks) != 2 {
		t.Fatalf("%d blocks after undo, want 2", len(d.blocks))
	}
	if len(d.sel) != 0 {
		t.Fatalf("selection after undo is %v, want none", d.sel)
	}

	d.redoLast()
	if len(d.blocks) != 3 || !d.sel[b.ID] {
		t.Fatalf("redo put back %d blocks and selection %v", len(d.blocks), d.sel)
	}
}

func TestDeleteSelectedUndoesInOrder(t *testing.T) {
	d := testDeck(t)
	d.sel = map[string]bool{"a": true, "b": true}

	d.deleteSelected()
	if len(d.blocks) != 0 {
		t.Fatalf("%d blocks left after deleting all", len(d.blocks))
	}

	d.undoLast()
	d.undoLast()
	if len(d.blocks) != 2 || d.blocks[0].ID != "a" || d.blocks[1].ID != "b" {
		t.Fatalf("undo put back %v", d.blocks)
	}
}

func TestNewChangesForgetTheRedo(t *testing.T) {
	d := testDeck(t)
	d.addBlock(&block{Kind: kindNote, Title: "C"})
	d.undoLast()
	d.addBlock(&block{Kind: kindNote, Title: "D"})

	if len(d.redo) != 0 {
		t.Fatalf("a new block kept %d redoable changes", len(d.redo))
	}
	if len(d.blocks) != 3 || d.blocks[2].Title != "D" {
		t.Fatalf("the deck holds %v", d.blocks)
	}
}

func TestDroppedFilesBecomeBlocksWhereTheyLand(t *testing.T) {
	d := testDeck(t)
	for _, name := range []string{"photo.txt", "notes.md"} {
		if err := os.WriteFile(filepath.Join(d.root, name), []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	added := d.dropFiles([]string{
		filepath.Join(d.root, "photo.txt"),
		filepath.Join(d.root, "notes.md"),
	}, 900, 600)
	if added != 2 {
		t.Fatalf("dropping two files added %d blocks", added)
	}
	first, second := d.blocks[2], d.blocks[3]
	for _, b := range []*block{first, second} {
		if b.Kind != kindFile || b.Path == "" {
			t.Fatalf("dropped block is %+v", b)
		}
	}
	// The first block lands under the pointer; the second steps aside into
	// the nearest free place rather than covering it.
	if 900 < first.X || 900 >= first.X+first.W || 600 < first.Y || 600 >= first.Y+first.H {
		t.Fatalf("the dropped block at (%v,%v) does not cover the pointer", first.X, first.Y)
	}
	if first.X < second.X+second.W && first.X+first.W > second.X &&
		first.Y < second.Y+second.H && first.Y+first.H > second.Y {
		t.Fatalf("two dropped blocks overlap")
	}
}

func TestDroppedFoldersAreIgnored(t *testing.T) {
	d := testDeck(t)
	if added := d.dropFiles([]string{d.root}, 200, 200); added != 0 {
		t.Fatalf("a dropped folder added %d blocks", added)
	}
}

func TestNewBlocksNeverLandOnEachOther(t *testing.T) {
	d := testDeck(t)
	first := d.addAt(kindNote, 300, 200)
	second := d.addAt(kindNote, 300, 200)

	if first.X < second.X+second.W && first.X+first.W > second.X &&
		first.Y < second.Y+second.H && first.Y+first.H > second.Y {
		t.Fatalf("blocks at (%v,%v) and (%v,%v) overlap", first.X, first.Y, second.X, second.Y)
	}
}

func TestCmdDDeletesTheSelection(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	tt.Key(ui.Cmd, ui.KeyD)
	tt.Frame()
	if len(d.blocks) != 2 {
		t.Fatalf("with nothing selected, %d blocks left, want 2", len(d.blocks))
	}

	d.sel = map[string]bool{"a": true}
	tt.Key(ui.Cmd, ui.KeyD)
	tt.Frame()
	if len(d.blocks) != 1 || d.blocks[0].ID != "b" {
		t.Fatalf("blocks after deleting a: %v", d.blocks)
	}
	d.undoLast()
	if len(d.blocks) != 2 {
		t.Fatalf("undo left %d blocks, want 2", len(d.blocks))
	}
}

func TestClickEditsAQuickLookBody(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	// Block a is 200×120 at (100, 80): its body is below the title bar.
	// The first click focuses its preview, the next edits it.
	tt.ClickAt(200, 160)
	tt.Frame()
	tt.Frame()
	if d.editing != nil || d.focusBlock != "a" {
		t.Fatalf("first click: editing %v, focus %q; want the preview focused", d.editing, d.focusBlock)
	}
	tt.ClickAt(200, 160)
	tt.Frame()
	tt.Frame()
	if d.editing != d.blocks[0] || d.editPart != editBody {
		t.Fatalf("editing %v part %v, want block a's body", d.editing, d.editPart)
	}
	tt.Type("# Hi **there**")
	tt.Frame()
	if d.blocks[0].Text != "# Hi **there**" {
		t.Fatalf("text = %q", d.blocks[0].Text)
	}
	// ⌘D types into the body rather than deleting the block.
	tt.Key(ui.Cmd, ui.KeyD)
	tt.Frame()
	if len(d.blocks) != 2 {
		t.Fatalf("⌘D while editing deleted a block")
	}
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if d.editing != nil {
		t.Fatal("Escape did not end the edit")
	}
	if !tt.HasText("Hi") || tt.HasText("# Hi **there**") {
		t.Fatalf("the body does not show as Markdown: %q", tt.Texts())
	}
}

func TestClickEditsAQuickLookTitle(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	tt.ClickAt(130, 92) // the title
	tt.Frame()
	tt.Frame()
	if d.editing != d.blocks[0] || d.editPart != editTitle {
		t.Fatalf("editing %v part %v, want block a's title", d.editing, d.editPart)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Plan")
	tt.Key(0, ui.KeyEnter)
	tt.Frame()
	if d.editing != nil || d.blocks[0].Title != "Plan" {
		t.Fatalf("editing %v, title %q; want done, \"Plan\"", d.editing, d.blocks[0].Title)
	}
}

func TestClickingElsewhereEndsTheEdit(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	d.startEditing(d.blocks[0], editBody)
	tt.Frame()
	tt.Frame()
	tt.ClickAt(700, 550)
	tt.Frame()
	tt.Frame()
	if d.editing != nil {
		t.Fatal("a click on the canvas left the edit on")
	}
}

func TestDraggingTheTapeMovesTheBlock(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	a := d.blocks[0] // 200×120 at (100, 80): its tape sticks out above y 80
	tp := d.tapeOf(a)
	px, py := tp.at(0, -tp.t*0.4) // on the tape, above the block
	tt.Press(px, py)
	tt.Frame()
	tt.Move(px+60, py+40)
	tt.Frame()
	tt.Release(px+60, py+40)
	tt.Frame()

	if abs(a.X-160) > 0.5 || abs(a.Y-120) > 0.5 {
		t.Fatalf("block at (%v, %v), want (160, 120)", a.X, a.Y)
	}
	if !d.sel["a"] || len(d.sel) != 1 || d.dragging != nil {
		t.Fatalf("selected %v, dragging %v; want a selected, the drag over", d.sel, d.dragging)
	}
	if d.editing != nil {
		t.Fatal("pressing the tape started an edit")
	}
}

func TestTitlesGrowWithTheZoom(t *testing.T) {
	d := testDeck(t)
	n := d.blocks[0]
	d.cam.S = 1
	h1 := d.titleHeight(n)
	d.cam.S = maxScale
	if h := d.titleHeight(n); abs(h-h1*maxScale) > 0.01 {
		t.Fatalf("title bar at %vx is %v DIPs, want %v: it should scale as the body does", maxScale, h, h1*maxScale)
	}
}

func TestResizingKeepsTheProportions(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	a := d.blocks[0] // 200×120 at (100, 80)
	rb := d.resizeBox(a)
	px, py := rb.X+rb.W/2, rb.Y+rb.H/2
	tt.Press(px, py)
	tt.Frame()
	tt.Move(px+100, py+20) // mostly sideways: it still grows in proportion
	tt.Frame()
	tt.Release(px+100, py+20)
	tt.Frame()

	if a.W <= 200 || abs(a.W/a.H-200.0/120) > 0.001 {
		t.Fatalf("block is %v×%v, want larger than 200×120 in the same proportions", a.W, a.H)
	}
	if a.X != 100 || a.Y != 80 {
		t.Fatalf("resizing moved the block to (%v, %v)", a.X, a.Y)
	}
	if d.resizing != nil || d.editing != nil || len(d.sel) != 0 {
		t.Fatalf("after the resize: resizing %v, editing %v, selection %v", d.resizing, d.editing, d.sel)
	}
}

func TestResizingStopsAtTheLeastSize(t *testing.T) {
	w, h := scaleKeepingRatio(200, 120, -500, -500)
	if w < minBlockW || h < minBlockH || abs(w/h-200.0/120) > 0.001 {
		t.Fatalf("shrunk to %v×%v", w, h)
	}
}

func TestFilePreviewTextScalesWithTheCanvas(t *testing.T) {
	d := testDeck(t)
	if err := os.WriteFile(filepath.Join(d.root, "a.go"), []byte("package a\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	f := &block{ID: "f", Kind: kindFile, Title: "a.go", Path: "a.go", X: 0, Y: 0, W: 300, H: 200}
	d.blocks = append(d.blocks, f)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	if f.rt == nil || f.rt.editor == nil {
		t.Fatal("the file did not load")
	}
	if got := f.rt.editor.Font.Size; got != editorFontSize {
		t.Fatalf("font at 100%% is %v, want %v", got, editorFontSize)
	}
	d.cam.S = 2
	tt.Frame()
	if got := f.rt.editor.Font.Size; got != 2*editorFontSize {
		t.Fatalf("font at 200%% is %v, want %v", got, 2*editorFontSize)
	}
}

func TestTerminalZoomZoomsTheCanvas(t *testing.T) {
	d := testDeck(t)
	n := d.blocks[1] // 200×120 at (400, 300)
	cx, cy := d.cam.toScreen(n.X+n.W/2, n.Y+n.H/2)
	wx0, wy0 := d.cam.toWorld(cx, cy)

	d.zoomTerminal(n, terminal.Zoom{Steps: 2})
	if want := wheelZoom(-80); abs(d.cam.S-want) > 0.001 {
		t.Fatalf("scale = %v, want %v, as 80 DIPs of the wheel zoom the canvas", d.cam.S, want)
	}
	cx, cy = d.cam.toScreen(n.X+n.W/2, n.Y+n.H/2)
	if wx, wy := d.cam.toWorld(cx, cy); abs(wx-wx0) > 0.01 || abs(wy-wy0) > 0.01 {
		t.Fatal("the block's middle moved")
	}
}

func TestZoomedOutBlocksDragFromAnywhere(t *testing.T) {
	d := testDeck(t)
	d.cam.S = 0.2
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	a := d.blocks[0] // 200×120 at (100, 80): on screen 40×24 at (20, 16)
	tt.Press(40, 32) // its middle, well below the thin title bar
	tt.Move(60, 32)
	tt.Release(60, 32)
	tt.Frame()
	if abs(a.X-200) > 0.5 || a.Y != 80 {
		t.Fatalf("block at (%v, %v), want (200, 80)", a.X, a.Y)
	}
	if d.editing != nil {
		t.Fatal("a press on a zoomed-out block started an edit")
	}
}

func TestCmdScrollOverATerminalZoomsSmoothly(t *testing.T) {
	d := testDeck(t)
	n := &block{ID: "t", Kind: kindTerminal, Title: "term", X: 20, Y: 20, W: 400, H: 240}
	d.blocks = []*block{n}
	term, err := terminal.New(terminal.Options{Dir: d.root, Command: []string{"/bin/sh", "-c", "sleep 30"}})
	if err != nil {
		t.Fatal(err)
	}
	defer term.Close()
	d.ensureRuntime(n).term = term
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	// The terminal's first step started a zoom over it.
	d.zoomTerminal(n, terminal.Zoom{Steps: 1})
	s0 := d.cam.S
	tt.HoldModifiers(ui.Cmd)
	tt.Frame()

	// Small touchpad moves, each under the terminal's step of 40 DIPs:
	// the canvas follows every one.
	x, y := d.cam.toScreen(n.X+n.W/2, n.Y+n.H/2)
	for i := 0; i < 5; i++ {
		// The Tester's scroll comes with no modifiers, and lets go of
		// them: a real one comes with ⌘ while it is held.
		tt.HoldModifiers(ui.Cmd)
		tt.Frame()
		before := d.cam.S
		tt.Scroll(x, y, 0, -8)
		tt.Frame()
		if want := before * wheelZoom(-8); abs(d.cam.S-want) > 0.0005 {
			t.Fatalf("move %d: scale %v, want %v", i, d.cam.S, want)
		}
	}
	if d.cam.S <= s0 {
		t.Fatal("the canvas did not zoom")
	}

	// Let go of ⌘: the terminal has its wheel back.
	tt.HoldModifiers(0)
	tt.Frame()
	before := d.cam.S
	tt.Scroll(x, y, 0, -8)
	tt.Frame()
	if d.cam.S != before {
		t.Fatal("the wheel without ⌘ zoomed the canvas")
	}
}

// longFileDeck is a deck with a file block whose text scrolls.
func longFileDeck(t *testing.T) (*deck, *block, *ui.Tester) {
	t.Helper()
	d := testDeck(t)
	text := ""
	for i := 0; i < 200; i++ {
		text += fmt.Sprintf("line %d\n", i)
	}
	if err := os.WriteFile(filepath.Join(d.root, "long.txt"), []byte(text), 0o644); err != nil {
		t.Fatal(err)
	}
	f := &block{ID: "f", Kind: kindFile, Title: "long.txt", Path: "long.txt", X: 300, Y: 100, W: 400, H: 300}
	d.blocks = []*block{f}
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	tt.Frame()
	return d, f, tt
}

func TestPanningOverAnUnfocusedBlockMovesTheCanvas(t *testing.T) {
	d, _, tt := longFileDeck(t)
	tt.Scroll(500, 300, 0, 40) // over the file's text
	tt.Frame()
	if abs(d.cam.Y-40) > 0.01 {
		t.Fatalf("camera at y %v, want 40: the text took the pan", d.cam.Y)
	}
}

func TestAFocusedBlockTakesTheWheel(t *testing.T) {
	d, f, tt := longFileDeck(t)
	tt.ClickAt(500, 300)
	tt.Frame()
	tt.Frame()
	if d.focusBlock != f.ID {
		t.Fatalf("focus is on %q, want the file", d.focusBlock)
	}
	time.Sleep(gestureHold) // no pan under way
	tt.Frame()
	tt.Scroll(500, 300, 0, 40)
	tt.Frame()
	if d.cam.Y != 0 {
		t.Fatalf("camera at y %v: the focused text did not take the wheel", d.cam.Y)
	}

	// A press on the canvas takes the focus back: the wheel pans again.
	tt.ClickAt(100, 500)
	tt.Frame()
	tt.Frame()
	if d.focusBlock != "" {
		t.Fatalf("focus is still on %q", d.focusBlock)
	}
	tt.Scroll(500, 300, 0, 40)
	tt.Frame()
	if abs(d.cam.Y-40) > 0.01 {
		t.Fatalf("camera at y %v, want 40", d.cam.Y)
	}
}

func TestAPanSlidingOntoAFocusedBlockGoesOn(t *testing.T) {
	d, f, tt := longFileDeck(t)
	d.focusBlock = f.ID
	tt.Frame()
	tt.Frame()
	tt.Scroll(100, 300, 20, 0) // on the canvas, left of the block
	tt.Frame()
	tt.Scroll(500, 300, 20, 0) // the same gesture, now over the block
	tt.Frame()
	if abs(d.cam.X-40) > 0.01 {
		t.Fatalf("camera at x %v, want 40: the pan stopped at the block", d.cam.X)
	}
}

func TestClickingTheTapeSelects(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	a, b := d.blocks[0], d.blocks[1]
	at := func(n *block) (float32, float32) {
		tp := d.tapeOf(n)
		return tp.at(0, -tp.t*0.4) // on the tape, above the block
	}

	x, y := at(a)
	tt.ClickAt(x, y)
	tt.Frame()
	if !d.sel["a"] || len(d.sel) != 1 {
		t.Fatalf("selection %v, want just a", d.sel)
	}
	if a.X != 100 || a.Y != 80 {
		t.Fatalf("a click moved the block to (%v, %v)", a.X, a.Y)
	}

	x, y = at(b)
	tt.ClickAtWith(ui.Shift, x, y)
	tt.Frame()
	if !d.sel["a"] || !d.sel["b"] {
		t.Fatalf("Shift+click: selection %v, want a and b", d.sel)
	}
	tt.ClickAtWith(ui.Shift, x, y)
	tt.Frame()
	if !d.sel["a"] || d.sel["b"] {
		t.Fatalf("Shift+click again: selection %v, want just a", d.sel)
	}

	x, y = at(b)
	tt.ClickAt(x, y)
	tt.Frame()
	if d.sel["a"] || !d.sel["b"] {
		t.Fatalf("click on b's tape: selection %v, want just b", d.sel)
	}
}

func TestPressingATapeBringsItsBlockToTheTop(t *testing.T) {
	d := testDeck(t)
	// B over A, where they overlap.
	d.blocks[1].X, d.blocks[1].Y = 160, 120
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	a := d.blocks[0]
	tp := d.tapeOf(a)
	px, py := tp.at(0, -tp.t*0.4)
	tt.Press(px, py)
	tt.Release(px, py)
	tt.Frame()
	if d.blocks[len(d.blocks)-1] != a || len(d.blocks) != 2 {
		t.Fatalf("blocks %v, %v: want A on top", d.blocks[0].ID, d.blocks[1].ID)
	}

	// Where they overlap, A now takes the press: Shift+click toggles it.
	tt.ClickAtWith(ui.Shift, 250, 180)
	tt.Frame()
	if d.sel["b"] {
		t.Fatal("the press went to B, under A")
	}

	// Saved in that order.
	d.save()
	e := newDeck()
	e.root, e.file = d.root, d.file
	e.load()
	if e.blocks[len(e.blocks)-1].ID != "a" {
		t.Fatal("A on top was not saved so")
	}
}

func TestEveryBlocksTitleCanBeRenamed(t *testing.T) {
	d := testDeck(t)
	d.blocks = []*block{
		{ID: "f", Kind: kindFile, Title: "notes.md", Path: "notes.md", X: 100, Y: 80, W: 300, H: 200},
		{ID: "t", Kind: kindTerminal, Title: "终端", X: 450, Y: 80, W: 300, H: 200},
	}
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	// A click on the file block's title, without a drag, edits it.
	tt.ClickAt(200, 90)
	tt.Frame()
	tt.Frame()
	f := d.blocks[0]
	if d.editing != f || d.editPart != editTitle {
		t.Fatalf("editing %v part %v: want the file block's title", d.editing, d.editPart)
	}
	tt.TypeKey(ui.Cmd, ui.KeyA, "")
	tt.Type("会议记录")
	tt.TypeKey(0, ui.KeyEnter, "")
	tt.Frame()
	if f.Title != "会议记录" || !f.Named || d.editing != nil {
		t.Fatalf("title %q named %v editing %v", f.Title, f.Named, d.editing)
	}

	// A drag on the title moves the block and edits nothing.
	tt.Press(600, 90)
	tt.Move(640, 120)
	tt.Release(640, 120)
	tt.Frame()
	if d.editing != nil {
		t.Fatal("dragging a title edited it")
	}

	// A named terminal keeps its name over its shell's titles; cleared,
	// it takes its own title and its shell's again.
	term := d.blocks[1]
	term.autoTitle("zsh")
	if term.Title != "zsh" {
		t.Fatalf("an unnamed terminal's title is %q, want its shell's", term.Title)
	}
	d.startEditing(term, editTitle)
	term.Title = "构建"
	d.stopEditing()
	term.autoTitle("vim")
	if term.Title != "构建" {
		t.Fatalf("a named terminal took its shell's title: %q", term.Title)
	}
	d.startEditing(term, editTitle)
	term.Title = "  "
	d.stopEditing()
	if term.Title != "终端" || term.Named {
		t.Fatalf("cleared: title %q named %v", term.Title, term.Named)
	}
	term.autoTitle("htop")
	if term.Title != "htop" {
		t.Fatal("cleared, the terminal takes no shell titles")
	}

	// Saved with the name.
	d.save()
	e := newDeck()
	e.root, e.file = d.root, d.file
	e.load()
	if e.blocks[0].Title != "会议记录" || !e.blocks[0].Named {
		t.Fatalf("loaded %q named %v", e.blocks[0].Title, e.blocks[0].Named)
	}
}

// Crossing the LOD threshold, a card's content blurs away, or comes clear
// from a blur, rather than popping out or in.
func TestCardContentFadesAcrossTheLODThreshold(t *testing.T) {
	d := testDeck(t)
	const body = "点击编辑，支持 Markdown" // what an empty QuickLook body shows
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	if !tt.HasText(body) {
		t.Fatal("above the threshold, the body does not show")
	}

	d.cam.S = 0.2
	tt.Frame()
	if !tt.HasText(body) {
		t.Fatal("the body went at once: no fade")
	}
	tt.Advance(contentFade + 50*time.Millisecond)
	tt.Frame()
	tt.Frame()
	if tt.HasText(body) {
		t.Fatal("faded out, the body is still built")
	}

	d.cam.S = 1
	tt.Frame()
	if !tt.HasText(body) {
		t.Fatal("coming back, the body is not built at once")
	}
}

// A QuickLook's preview is live: not focused, a pan over it moves the
// canvas; a click focuses it, and the wheel then scrolls its text.
func TestAQuickLookPreviewScrollsOnceFocused(t *testing.T) {
	d := testDeck(t)
	a := d.blocks[0]
	a.Text = strings.Repeat("一行字\n\n", 60)
	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()

	tt.Scroll(200, 160, 0, 40)
	tt.Frame()
	if abs(d.cam.Y-40) > 0.01 {
		t.Fatalf("camera at y %v, want 40: the unfocused note took the pan", d.cam.Y)
	}
	time.Sleep(gestureHold) // that pan is over
	tt.Frame()

	x, y := d.cam.toScreen(a.X+a.W/2, a.Y+a.H/2)
	tt.ClickAt(x, y)
	tt.Frame()
	tt.Frame()
	if d.focusBlock != a.ID || d.editing != nil {
		t.Fatalf("focus %q, editing %v: want the preview focused, not edited", d.focusBlock, d.editing)
	}
	camY := d.cam.Y
	tt.Scroll(x, y, 0, 40)
	tt.Frame()
	if d.cam.Y != camY {
		t.Fatalf("camera moved to y %v: the focused preview did not take the wheel", d.cam.Y)
	}
	if a.rt.liveScroll.Y <= 0 {
		t.Fatalf("the preview scrolled to %v, want down", a.rt.liveScroll.Y)
	}
	if !tt.HasText("一行字") {
		t.Fatal("the preview shows no text")
	}

	// Escape ends an edit with the preview still focused.
	tt.ClickAt(x, y)
	tt.Frame()
	tt.Frame()
	if d.editing != a {
		t.Fatal("a click on the focused preview did not edit it")
	}
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	tt.Frame()
	if d.editing != nil || d.focusBlock != a.ID {
		t.Fatalf("after Escape: editing %v, focus %q", d.editing, d.focusBlock)
	}
}
