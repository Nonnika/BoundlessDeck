package main

import (
	"testing"

	"github.com/egoist/mygo/ui"
)

// Under a hidden title bar the deck runs to the window's top: its pill
// keeps clear of the traffic lights, empty canvas along the top is left
// to drag the window, and a block up there still takes its presses.
func TestTheDeckRunsUnderTheTitleBar(t *testing.T) {
	d := testDeck(t)
	d.onHome = func() {}
	d.blocks[0].X, d.blocks[0].Y = 420, 2 // its title bar in the band, clear of the pill
	tt := ui.NewTester(d.view, 1100, 700)
	tt.SetTitleBar(ui.TitleBar{Height: 61, Left: 78})
	tt.Frame()

	if r, ok := tt.Find("返回主页"); !ok || r.X < 78 {
		t.Fatalf("the home pill is at %v, under the traffic lights", r)
	}

	// Empty canvas in the band: the canvas leaves the press alone, so the
	// selection stays (a press on the canvas would clear it).
	d.sel = map[string]bool{"b": true}
	tt.Press(300, 30) // left of the toolbar
	tt.Release(300, 30)
	if !d.sel["b"] {
		t.Fatal("a press on the canvas in the title bar's band was the canvas's")
	}
	// Below it, the canvas's as ever.
	tt.Press(500, 200)
	tt.Release(500, 200)
	if d.sel["b"] {
		t.Fatal("a press on the canvas below the band did not clear the selection")
	}

	// A block over the band drags by its title as anywhere.
	a := d.blocks[0]
	x, y := d.cam.toScreen(a.X+a.W/2, a.Y+10)
	x0 := a.X
	tt.Press(x, y)
	tt.Move(x+40, y)
	tt.Release(x+40, y)
	if a.X == x0 {
		t.Fatal("a block over the band did not take its drag")
	}
}
