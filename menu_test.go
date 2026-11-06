package main

import (
	"testing"

	"github.com/egoist/mygo"
)

func TestMenuListsTheShortcutsThatAddBlocks(t *testing.T) {
	a := &app{}
	m := a.buildMenu()
	for _, k := range newKeys {
		it := m.ItemByID("new-" + string(k.kind))
		if it == nil {
			t.Fatalf("the menu has no item adding a %s block", k.kind)
		}
		if it.Accelerator != k.acc || it.Label != k.label {
			t.Fatalf("item %q has %q, want %q %q", it.ID, it.Accelerator, k.label, k.acc)
		}
	}
	for _, id := range deckItems() {
		if m.ItemByID(id) == nil {
			t.Fatalf("no menu item %q", id)
		}
	}
}

func TestCursorInMapsTheScreenToTheWindow(t *testing.T) {
	content := mygo.Rectangle{X: 100, Y: 50, Width: 1280, Height: 800}
	x, y, ok := cursorIn(mygo.Point{X: 400, Y: 350}, content, 1280, 800)
	if !ok || x != 300 || y != 300 {
		t.Fatalf("cursor at (400,350) is (%v,%v,%v) in the window, want (300,300,true)", x, y, ok)
	}
	if _, _, ok := cursorIn(mygo.Point{X: 50, Y: 350}, content, 1280, 800); ok {
		t.Fatal("a cursor left of the window was taken as in it")
	}
}

func TestBlocksAddedByShortcutLandUnderThePointer(t *testing.T) {
	for _, kind := range []blockKind{kindNote, kindMemo, kindTerminal, kindBrowser, kindStack} {
		d := testDeck(t)
		d.cam = camera{X: -50, Y: 20, S: 0.5}
		d.addKindAt(kind, 700, 500)
		b := d.blocks[len(d.blocks)-1]
		if b.Kind != kind {
			t.Fatalf("added %s, want %s", b.Kind, kind)
		}
		wx, wy := d.cam.toWorld(700, 500)
		if abs(b.X+b.W/2-wx) > 0.5 || abs(b.Y+b.H/2-wy) > 0.5 {
			t.Fatalf("%s block centred at (%v,%v), want under the pointer at (%v,%v)",
				kind, b.X+b.W/2, b.Y+b.H/2, wx, wy)
		}
	}
}

func TestShortcutsWindowListsEveryMenuKey(t *testing.T) {
	for acc, want := range map[string]string{
		"CmdOrCtrl+Shift+N": "⇧⌘N", "CmdOrCtrl+=": "⌘=", "CmdOrCtrl+-": "⌘-",
		"CmdOrCtrl+/": "⌘/", "Ctrl+Alt+Shift+Cmd+K": "⌃⌥⇧⌘K",
	} {
		if got := keyGlyphs(acc); got != want {
			t.Fatalf("keyGlyphs(%q) = %q, want %q", acc, got, want)
		}
	}
	a := &app{}
	m := a.buildMenu()
	if it := m.ItemByID("shortcuts"); it == nil || it.Accelerator != "CmdOrCtrl+/" {
		t.Fatal("帮助 has no 键盘快捷键 item on ⌘/")
	}
	listed := map[string]bool{}
	for _, g := range menuShortcuts(a.menuItems) {
		for _, r := range g.rows {
			listed[r.label+" "+r.keys[0]] = true
		}
	}
	for _, k := range newKeys {
		if !listed[k.label+" "+k.glyph] {
			t.Fatalf("the window does not list %s %s", k.label, k.glyph)
		}
	}
	for _, want := range []string{"撤销 ⌘Z", "删除选中的 block ⌘D", "键盘快捷键 ⌘/"} {
		if !listed[want] {
			t.Fatalf("the window does not list %s", want)
		}
	}
}
