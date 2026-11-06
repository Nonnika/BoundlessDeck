package main

import (
	"testing"

	"github.com/egoist/mygo/ui"
)

// Zooming makes a font of each size; past 512 MyGo lets go of them all.
// A terminal on the canvas must shape its text anew then, not draw glyphs
// of the fonts let go of (a crash in Core Text).
func TestTerminalSurvivesTheTextSystemForgettingFonts(t *testing.T) {
	d := testDeck(t)
	b := d.addAt(kindTerminal, 100, 100)
	term := quietTerminal(t)
	term.Feed([]byte("$ ls\r\nREADME.md  main.go  一些中文\r\n$ "))
	d.ensureRuntime(b).term = term
	d.cam.X, d.cam.Y = 0, 0

	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	tt.Image() // shapes and draws the terminal's rows
	before := fontEpoch()

	// Fonts of more sizes than MyGo keeps, as a long zoom makes.
	for i := range 600 {
		ui.Shape("Mx一", ui.Font{Size: 5 + float32(i)*0.1})
	}
	tt.Frame() // lets go of them as it ends
	tt.Image()
	tt.Frame()
	tt.Image()

	if fontEpoch() == before {
		t.Fatal("the text system's letting go of its fonts went unnoticed")
	}
}
