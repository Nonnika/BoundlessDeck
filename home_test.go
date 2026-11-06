package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// testApp is an app with no window, keeping its recents in a test folder.
func testApp(t *testing.T) *app {
	t.Helper()
	a := &app{recentsFile: filepath.Join(t.TempDir(), "recents.json")}
	return a
}

func TestOpeningAFolderMakesItTheDeck(t *testing.T) {
	a := testApp(t)
	dir := t.TempDir()
	a.openFolder(dir)
	if a.deck == nil || a.deck.root != dir {
		t.Fatalf("deck %v, want one rooted at %s", a.deck, dir)
	}
	if len(a.deck.blocks) != 1 {
		t.Fatalf("a new deck has %d blocks, want the welcome note", len(a.deck.blocks))
	}
	if len(a.recents) != 1 || a.recents[0].Path != dir {
		t.Fatalf("recents %v, want the folder", a.recents)
	}
}

func TestGoingHomeSavesTheDeckInItsFolder(t *testing.T) {
	a := testApp(t)
	dir := t.TempDir()
	a.openFolder(dir)
	a.deck.addAt(kindNote, 300, 200)
	a.deck.cam = camera{X: 12, Y: 34, S: 1.5}
	a.goHome()

	if a.deck != nil {
		t.Fatal("the deck is still open")
	}
	if _, err := os.Stat(filepath.Join(dir, deckDir, "deck.json")); err != nil {
		t.Fatalf("no deck.json in the folder: %v", err)
	}
	if a.recents[0].Blocks != 2 {
		t.Fatalf("recents count %d blocks, want 2", a.recents[0].Blocks)
	}

	// Opened again, from a fresh start: the deck comes back as it was.
	b := testApp(t)
	b.recentsFile = a.recentsFile
	b.loadRecents()
	if len(b.recents) != 1 || b.recents[0].Path != dir {
		t.Fatalf("recents after restart: %v", b.recents)
	}
	b.openFolder(dir)
	if len(b.deck.blocks) != 2 || b.deck.cam != (camera{X: 12, Y: 34, S: 1.5}) {
		t.Fatalf("reopened: %d blocks, camera %v", len(b.deck.blocks), b.deck.cam)
	}
}

func TestOpeningAnotherFolderSwitchesDecks(t *testing.T) {
	a := testApp(t)
	one, two := t.TempDir(), t.TempDir()
	a.openFolder(one)
	a.openFolder(two)
	if a.deck.root != two {
		t.Fatalf("deck root %s, want %s", a.deck.root, two)
	}
	if _, err := os.Stat(filepath.Join(one, deckDir, "deck.json")); err != nil {
		t.Fatal("the first folder's deck was not saved as the second opened")
	}
	if len(a.recents) != 2 || a.recents[0].Path != two || a.recents[1].Path != one {
		t.Fatalf("recents %v, want two then one", a.recents)
	}
}

func TestDroppingOnTheHomeScreen(t *testing.T) {
	a := testApp(t)
	dir := t.TempDir()
	file := filepath.Join(dir, "a.txt")
	if err := os.WriteFile(file, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	a.dropped([]string{file}, 0, 0)
	if a.deck != nil || a.err == "" {
		t.Fatalf("a file dropped: deck %v, err %q; want no deck and a message", a.deck, a.err)
	}
	a.dropped([]string{file, dir}, 0, 0)
	if a.deck == nil || a.deck.root != dir || a.err != "" {
		t.Fatalf("a folder dropped: deck %v, err %q", a.deck, a.err)
	}
}

func TestAMissingFolderShowsAndCanBeForgotten(t *testing.T) {
	a := testApp(t)
	dir := t.TempDir()
	gone := filepath.Join(dir, "gone")
	if err := os.Mkdir(gone, 0o755); err != nil {
		t.Fatal(err)
	}
	a.openFolder(gone)
	a.goHome()
	if err := os.Remove(filepath.Join(gone, deckDir, "deck.json")); err != nil {
		t.Fatal(err)
	}
	if err := os.RemoveAll(gone); err != nil {
		t.Fatal(err)
	}
	a.checkRecents()
	if !a.recents[0].missing {
		t.Fatal("the folder gone is not marked")
	}
	a.openFolder(gone)
	if a.deck != nil || a.err == "" {
		t.Fatal("a folder gone opened, or said nothing")
	}
	a.forget(gone)
	if len(a.recents) != 0 {
		t.Fatalf("recents %v after forgetting", a.recents)
	}
}

func TestTheHomeScreenOpensARecentFolder(t *testing.T) {
	a := testApp(t)
	dir := t.TempDir()
	a.openFolder(dir)
	a.goHome()

	tt := ui.NewTester(a.view, 900, 600)
	tt.Frame()
	if !tt.HasText("打开文件夹…") || !tt.HasText(filepath.Base(dir)) {
		t.Fatalf("home screen shows %q", tt.Texts())
	}
	if err := tt.Click(filepath.Base(dir)); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if a.deck == nil || a.deck.root != dir {
		t.Fatal("a click on the recent folder did not open it")
	}
}

func TestEndlessWordsTakeTurns(t *testing.T) {
	period := endlessHold + endlessSlide
	base := time.Unix(0, 0).Add(period * 30) // 30 is a multiple of 3: word 0
	for i := range 2 * len(endlessWords) {
		start := base.Add(time.Duration(i) * period)
		if w, s := endlessAt(start); w != i%len(endlessWords) || s != 0 {
			t.Fatalf("period %d starts on word %d, slide %v; want word %d, still", i, w, s, i%len(endlessWords))
		}
		if _, s := endlessAt(start.Add(endlessHold + endlessSlide/2)); s < 0.49 || s > 0.51 {
			t.Fatalf("halfway through the slide, slide is %v", s)
		}
	}
}

func TestDraggingARecentToTheTrashForgetsIt(t *testing.T) {
	a := testApp(t)
	keep, gone := t.TempDir(), t.TempDir()
	a.touch(keep, 1)
	a.touch(gone, 2)
	tt := ui.NewTester(a.view, 1100, 760)
	tt.Frame()
	if _, ok := tt.Find("拖到这里，从列表移除"); ok && a.draggingRecent {
		t.Fatal("the trash shows before anything is dragged")
	}
	row, ok := tt.Find(filepath.Base(gone))
	if !ok {
		t.Fatal("no row for the folder")
	}
	x, y := row.X+row.W/2, row.Y+row.H/2
	tt.Press(x, y)
	tt.Move(x, y+20)
	tt.Move(550, 700)
	tt.Frame()
	if !a.draggingRecent {
		t.Fatal("dragging a recent shows no trash")
	}
	tt.Move(550, 698) // over the trash, 60 high, 32 from the bottom
	tt.Release(550, 698)
	tt.Frame()

	if a.deck != nil {
		t.Fatal("the drag opened the folder")
	}
	if len(a.recents) != 1 || a.recents[0].Path != keep {
		t.Fatalf("recents %v, want only %s", a.recents, keep)
	}
	if _, err := os.Stat(gone); err != nil {
		t.Fatalf("the folder itself went: %v", err)
	}
	if a.draggingRecent {
		t.Fatal("the trash still shows after the drop")
	}
}

func TestDroppingARecentElsewhereKeepsIt(t *testing.T) {
	a := testApp(t)
	dir := t.TempDir()
	a.touch(dir, 1)
	tt := ui.NewTester(a.view, 1100, 760)
	tt.Frame()
	row, _ := tt.Find(filepath.Base(dir))
	x, y := row.X+row.W/2, row.Y+row.H/2
	tt.Press(x, y)
	tt.Move(x, y+20)
	tt.Move(900, 300)
	tt.Release(900, 300)
	tt.Frame()
	if len(a.recents) != 1 || a.deck != nil {
		t.Fatalf("recents %v, deck %v: want the folder kept, none opened", a.recents, a.deck)
	}
}

func TestOpeningAFolderBlursOutAndTheDeckComesClear(t *testing.T) {
	a := testApp(t)
	a.animate = true
	dir := t.TempDir()
	a.touch(dir, 0)
	tt := ui.NewTester(a.view, 1100, 760)
	tt.Frame()
	if err := tt.Click(filepath.Base(dir)); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if a.deck != nil {
		t.Fatal("the folder opened before the window blurred out")
	}
	tt.Advance(stageBlurOut / 2)
	if a.blur <= 0 || a.blur >= stageBlur {
		t.Fatalf("halfway out, blur %v", a.blur)
	}
	tt.Advance(stageBlurOut)
	if a.deck == nil || a.deck.root != dir {
		t.Fatal("blurred out, the folder did not open")
	}
	if a.blur <= 0 {
		t.Fatal("the deck shows clear at once")
	}
	tt.Advance(stageClear + 100*time.Millisecond)
	tt.Frame()
	if a.blur != 0 {
		t.Fatalf("the deck still blurred %v", a.blur)
	}
}

func TestGoingHomeBlursOutAndHomeComesClear(t *testing.T) {
	a := testApp(t)
	a.animate = true
	dir := t.TempDir()
	a.openFolder(dir)
	tt := ui.NewTester(a.view, 1100, 760)
	tt.Frame()
	if err := tt.Click("返回主页"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if a.deck == nil {
		t.Fatal("the deck closed before the window blurred out")
	}
	tt.Advance(stageBlurOut + 50*time.Millisecond)
	if a.deck != nil {
		t.Fatal("blurred out, the deck is still open")
	}
	if _, err := os.Stat(filepath.Join(dir, deckDir, "deck.json")); err != nil {
		t.Fatalf("going home did not save the deck: %v", err)
	}
	if a.blur <= 0 {
		t.Fatal("home shows clear at once")
	}
	tt.Advance(stageClear + 100*time.Millisecond)
	tt.Frame()
	if a.blur != 0 || !tt.HasText(filepath.Base(dir)) {
		t.Fatalf("home blurred %v, shows %q", a.blur, tt.Texts())
	}
}

// The workspace's name at the top left lists the folders opened lately:
// choosing one switches to it.
func TestTheWorkspaceMenuSwitchesWorkspaces(t *testing.T) {
	a := testApp(t)
	first, second := filepath.Join(t.TempDir(), "甲"), filepath.Join(t.TempDir(), "乙")
	for _, dir := range []string{first, second} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	a.openFolder(first)
	a.openFolder(second)
	tt := ui.NewTester(a.view, 1100, 700)
	tt.Frame()

	if err := tt.Click("切换工作区"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	items := tt.Menu()
	want := []string{
		"乙    " + shortPath(filepath.Dir(second)), "甲    " + shortPath(filepath.Dir(first)),
		"-", "打开文件夹…",
	}
	if strings.Join(items, "|") != strings.Join(want, "|") {
		t.Fatalf("the menu lists %q, want %q", items, want)
	}
	if err := tt.ChooseMenuItem(want[1]); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	tt.Frame()
	if a.deck == nil || a.deck.root != first {
		t.Fatalf("choosing 甲 opened %v", a.deck)
	}
}
