package main

import (
	"reflect"

	"github.com/egoist/mygo/ui"
)

// MyGo's text system (v0.7.4) lets go of every font once more than 512
// pile up, as they do with the canvas's zoom, each step a size of its own:
// it releases their CTFonts. The terminal plugin keeps the glyphs it
// shaped, which point at those fonts, and draws them again: a crash in
// Core Text. fontEpoch tells when that happened, so each terminal can be
// made to shape its text anew (terminalBody).

var (
	fontProbe uintptr // the font a probe glyph was last shaped in
	fontGen   int
)

// fontEpoch returns a number that changes whenever the text system has
// let go of its fonts since it was last called: a glyph shaped in the
// same style then comes in a font made anew. Call it on the UI thread.
func fontEpoch() int {
	gs := ui.Shape("M", ui.Font{Family: "monospace", Size: terminalFontSize})
	if len(gs) == 0 {
		return fontGen
	}
	// Glyph keeps its font unexported; its address is all that is read.
	p := reflect.ValueOf(gs[0]).FieldByName("font").Pointer()
	if fontProbe != 0 && p != fontProbe {
		fontGen++
	}
	fontProbe = p
	return fontGen
}
