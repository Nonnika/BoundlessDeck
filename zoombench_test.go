package main

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// zoomDeck is a deck as one works in it: notes, a source file and a
// rendered document, all in view.
func zoomDeck(tb testing.TB) *deck {
	d := newDeck()
	d.root = tb.TempDir()
	d.file = filepath.Join(d.root, "deck.json")
	for _, f := range []string{"deck.go", "README.md"} {
		b, err := os.ReadFile(f)
		if err != nil {
			tb.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(d.root, f), b, 0o644); err != nil {
			tb.Fatal(err)
		}
	}
	d.blocks = []*block{
		{ID: "n1", Kind: kindNote, Title: "想法", Text: "画布缩放要顺滑\n一些中文和 English mixed", X: 40, Y: 40, W: 220, H: 160},
		{ID: "n2", Kind: kindNote, Title: "TODO", Text: "- zoom\n- pan\n- terminals", X: 40, Y: 240, W: 220, H: 160},
		{ID: "code", Kind: kindFile, Title: "deck.go", Path: "deck.go", X: 300, Y: 40, W: 420, H: 520},
		{ID: "doc", Kind: kindFile, Title: "README.md", Path: "README.md", X: 750, Y: 40, W: 420, H: 520},
	}
	return d
}

// While a pinch goes on, the blocks keep their layout and their layer is
// scaled; once it rests, they are laid out at the zoom. Pointers act where
// the blocks show meanwhile.
func TestPinchScalesTheLayoutUntilTheZoomRests(t *testing.T) {
	holdZoom(t)
	d := zoomDeck(t)
	tt := ui.NewTester(d.view, 1280, 800)
	tt.Frame()
	code := d.blocks[2]
	size := d.ensureRuntime(code).editor.Font.Size

	tt.Pinch(450, 300, 1.1)
	tt.Frame()
	if k := d.layerScale(); abs(k-1.1) > 0.001 {
		t.Fatalf("mid-pinch the layer is scaled %v, want 1.1", k)
	}
	if got := code.rt.editor.Font.Size; got != size {
		t.Fatalf("mid-pinch the code was laid out anew at %v (was %v)", got, size)
	}

	// A title dragged mid-pinch moves with the pointer, where it shows.
	sx, sy := d.cam.toScreen(code.X+code.W/2, code.Y)
	sy += titleHeightAt1 * d.cam.S / 2
	x0 := code.X
	tt.Press(sx, sy)
	tt.Move(sx+55, sy)
	tt.Release(sx+55, sy)
	if want := x0 + 55/d.cam.S; abs(code.X-want) > 0.5 {
		t.Fatalf("dragged mid-pinch, the block is at x %v, want %v", code.X, want)
	}

	// However far the pinch goes, nothing is laid out anew before it rests.
	tt.Pinch(450, 300, 3)
	tt.Frame()
	if got := code.rt.editor.Font.Size; got != size {
		t.Fatalf("pinched 3 times further, the code was laid out anew at %v", got)
	}

	// At rest: laid out at the zoom, crisp.
	tt.Pinch(450, 300, 1.05)
	tt.Frame()
	d.zoomSettle = time.Time{}
	tt.Frame()
	if k := d.layerScale(); k != 1 {
		t.Fatalf("at rest the layer is scaled %v", k)
	}
	if got, want := code.rt.editor.Font.Size, editorFontSize*d.cam.S; abs(got-want) > 1e-3 {
		t.Fatalf("at rest the code is %v, want %v", got, want)
	}
}

// ZOOM_BENCH=1 go test -run TestZoomFrameTimes -v . : what a frame of a
// pinch costs, with the layer (ZOOM_LAYER=0 without). The Tester draws
// each frame in software, transformed layers slowly, where the app draws
// on the GPU: compare a CPU profile without internal/raster.
func TestZoomFrameTimes(t *testing.T) {
	if os.Getenv("ZOOM_BENCH") == "" {
		t.Skip("ZOOM_BENCH=1 to run")
	}
	d := zoomDeck(t)
	term := quietTerminal(t)
	for range 20 {
		term.Feed([]byte("\x1b[32m~/Code/BoundlessDeck\x1b[0m $ ls -la 一些文件\r\n-rw-r--r--  1 nonnika  staff  12345 Nov  5 18:50 canvas.go\r\n"))
	}
	b := d.addAt(kindTerminal, 40, 600)
	b.W, b.H = 680, 300
	d.ensureRuntime(b).term = term
	tt := ui.NewTester(d.view, 1280, 800)
	d.cam.S, d.cam.X, d.cam.Y = 0.8, 0, 0
	tt.Frame()
	tt.Frame()
	const n = 240
	start := time.Now()
	for i := range n {
		k := float32(1.03)
		if (i/40)%2 == 1 {
			k = 1 / k
		}
		d.zoomAt(450, 300, k)
		if os.Getenv("ZOOM_LAYER") != "0" {
			d.zoomGesture()
		}
		tt.Frame()
	}
	t.Logf("%d zoom frames: %.2f ms a frame", n, float64(time.Since(start).Microseconds())/1000/n)
}

// holdZoom keeps a zoom going until the test ends it (d.zoomSettle), as
// slow as the test runs: under the race detector a frame may take longer
// than a zoom's rest.
func holdZoom(t *testing.T) {
	saved := zoomSettleHold
	zoomSettleHold = time.Minute
	t.Cleanup(func() { zoomSettleHold = saved })
}

// A zoom that rests between zoom levels ends on the nearest, about the
// point it zoomed about; the keys and fit-all zoom from level to level.
func TestZoomRestsOnAZoomLevel(t *testing.T) {
	d := zoomDeck(t)
	tt := ui.NewTester(d.view, 1280, 800)
	tt.Frame()

	tt.Pinch(450, 300, 1.37)
	tt.Frame()
	wx, wy := d.cam.toWorld(450, 300)
	d.zoomSettle = time.Time{}
	tt.Frame()
	if !onLevel(d.cam.S) || abs(d.cam.S-zoomLevel(1.37)) > 1e-5 {
		t.Fatalf("the zoom rested at %v, want the level %v", d.cam.S, zoomLevel(1.37))
	}
	if d.lay != d.cam {
		t.Fatalf("at rest the blocks are laid out with %v, not the camera %v", d.lay, d.cam)
	}
	if x, y := d.cam.toWorld(450, 300); abs(x-wx) > 0.05 || abs(y-wy) > 0.05 { // less than a pixel
		t.Fatalf("snapping to the level moved the point zoomed about: (%v,%v) -> (%v,%v)", wx, wy, x, y)
	}

	// Keys step by levels: from a level to a level, and back.
	s := d.cam.S
	tt.Key(ui.Cmd, ui.KeyEqual)
	tt.Frame()
	if !onLevel(d.cam.S) || d.cam.S <= s {
		t.Fatalf("⌘= zoomed %v -> %v, want a larger level", s, d.cam.S)
	}
	tt.Key(ui.Cmd, ui.KeyMinus)
	tt.Frame()
	if d.cam.S != s {
		t.Fatalf("⌘= then ⌘- left %v, want %v", d.cam.S, s)
	}

	tt.Key(ui.Cmd, ui.Key0)
	tt.Frame()
	if !onLevel(d.cam.S) {
		t.Fatalf("fit-all zoomed to %v, between levels", d.cam.S)
	}
	for _, s := range []float32{1, 2, 4, 0.5} {
		if !onLevel(s) {
			t.Fatalf("%v is not a zoom level", s)
		}
	}
}

// Mid-pinch the layer keeps its layout and is moved and scaled to the
// camera: the blocks show where the camera puts them.
func TestPinchMovesTheLayerToTheCamera(t *testing.T) {
	holdZoom(t)
	d := zoomDeck(t)
	tt := ui.NewTester(d.view, 1280, 800)
	tt.Frame()
	lay := d.lay

	tt.Pinch(450, 120, 1.3)
	tt.Frame()
	tt.Pinch(450, 120, 1.2)
	tt.Frame()
	if d.lay != lay {
		t.Fatalf("mid-pinch the layout moved: %v -> %v", lay, d.lay)
	}
	if !d.layerMoved() {
		t.Fatal("mid-pinch the layer is not moved")
	}
	code := d.blocks[2]
	r, ok := tt.Find("deck.go")
	if !ok {
		t.Fatalf("no title deck.go: %v", tt.Texts())
	}
	sx, sy := d.cam.toScreen(code.X, code.Y)
	if abs(r.X-(sx+8*d.cam.S)) > 3 || r.Y < sy || r.Y > sy+titleHeightAt1*d.cam.S {
		t.Fatalf("mid-pinch the title shows at (%v,%v), the camera puts the block at (%v,%v)", r.X, r.Y, sx, sy)
	}
}

// Zoomed out past half its layout mid-gesture, the layer is laid out
// anew at the zoom's level, and the gesture goes on.
func TestZoomingFarOutLaysTheLayerOutAnew(t *testing.T) {
	holdZoom(t)
	d := zoomDeck(t)
	tt := ui.NewTester(d.view, 1280, 800)
	tt.Frame()
	s0 := d.lay.S

	tt.Pinch(450, 300, 0.8)
	tt.Frame()
	if d.lay.S != s0 {
		t.Fatalf("a little zoomed out, the layout went from %v to %v", s0, d.lay.S)
	}
	tt.Pinch(450, 300, 0.5)
	tt.Frame()
	if d.lay.S != zoomLevel(d.cam.S) || !d.layerMoved() && d.cam.S != d.lay.S {
		t.Fatalf("zoomed out to %v, the layout is at %v, want the level %v", d.cam.S, d.lay.S, zoomLevel(d.cam.S))
	}
	if k := d.layerScale(); k < zoomRebase || k > 1/zoomRebase {
		t.Fatalf("after laying out anew the layer is scaled %v", k)
	}
}
