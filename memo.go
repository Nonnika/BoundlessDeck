package main

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

// A 笔记 block (kind memo) is a Markdown file of the deck's folder: the
// block shows it rendered, a click edits it, and what is typed goes back to
// the file. The file is the note: its name is the block's title, renaming
// the block renames it, and what else writes to it (an agent, an editor)
// shows in the block. Taking the block off the canvas leaves the file.
//
// Its runtime keeps the text as edited (memo) and as the file holds it
// (memoSaved); what differs is written once typing rests for memoSaveIdle,
// when the edit ends and when the block is hung up.

const (
	memoTitle    = "笔记"
	memoSaveIdle = 800 * time.Millisecond
	// memoPoll is how often a note shown, and not being edited, looks
	// whether its file changed.
	memoPoll = 2 * time.Second
)

// memoName is the file name, without .md, a title gives a note: what a
// file name cannot hold becomes "-".
func memoName(title string) string {
	name := strings.Map(func(r rune) rune {
		switch r {
		case '/', '\\', ':', 0:
			return '-'
		}
		return r
	}, strings.TrimSpace(title))
	name = strings.Trim(name, ". ")
	if name == "" {
		return memoTitle
	}
	return name
}

// memoTitleOf is the title of a note at path: its file's name, without .md.
func memoTitleOf(path string) string {
	base := filepath.Base(path)
	return strings.TrimSuffix(base, filepath.Ext(base))
}

// createMemoFile makes an empty Markdown file in dir, named after title,
// or "title 2", "title 3"… when that is taken, and returns its name.
func createMemoFile(dir, title string) (string, error) {
	base := memoName(title)
	for i := 1; i < 1000; i++ {
		name := base + ".md"
		if i > 1 {
			name = fmt.Sprintf("%s %d.md", base, i)
		}
		f, err := os.OpenFile(filepath.Join(dir, name), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
		if errors.Is(err, fs.ErrExist) {
			continue
		}
		if err != nil {
			return "", err
		}
		return name, f.Close()
	}
	return "", fmt.Errorf("%s 下已有太多同名笔记", dir)
}

// renameMemoFile renames the note at rel, inside root, after title, and
// returns its new path: never over a file there already.
func renameMemoFile(root, rel, title string) (string, error) {
	next := filepath.Join(filepath.Dir(rel), memoName(title)+".md")
	if next == rel {
		return rel, nil
	}
	abs := func(p string) string {
		if filepath.IsAbs(p) {
			return p
		}
		return filepath.Join(root, p)
	}
	// A rename that only changes case is the same file on macOS.
	if _, err := os.Stat(abs(next)); err == nil && !strings.EqualFold(next, rel) {
		return rel, fmt.Errorf("已有文件 %s", filepath.Base(next))
	}
	if err := os.Rename(abs(rel), abs(next)); err != nil {
		return rel, err
	}
	return next, nil
}

// newMemo adds a 笔记 at a point of the window, in DIPs, and makes its
// file, off the main thread; the block is there at once, to be written in.
func (d *deck) newMemo(sx, sy float32) *block {
	wx, wy := d.cam.toWorld(sx, sy)
	def := mcpKindDefaults[kindMemo]
	b := &block{Kind: kindMemo, Title: def.Title, W: float32(def.W), H: float32(def.H)}
	b.X, b.Y = d.freeSpot(wx-b.W/2, wy-b.H/2, b.W, b.H)
	rt := d.ensureRuntime(b)
	rt.loading = true
	d.addBlock(b)
	d.startEditing(b, editBody)
	root := d.root
	done := func(name string, err error) {
		rt.loading = false
		if err != nil {
			rt.memoErr = err
			return
		}
		b.Path, b.Title = name, memoTitleOf(name)
		rt.memoLoaded = true
		d.requestSave()
	}
	if d.win == nil { // tests: no goroutine to hand results to
		done(createMemoFile(root, memoTitle))
		return b
	}
	go func() {
		name, err := createMemoFile(root, memoTitle)
		d.win.Update(func() { done(name, err) })
	}()
	return b
}

// loadMemo reads the note's file, off the main thread.
func (d *deck) loadMemo(n *block, rt *runtime) {
	if rt.loading || rt.memoLoaded || rt.memoErr != nil || n.Path == "" {
		return
	}
	path := d.path(n)
	d.wantLoad("memo:"+n.ID, n, func() { rt.loading = true }, func() func() {
		text, mod, err := readMemo(path)
		return func() {
			rt.loading = false
			if err != nil {
				rt.memoErr = err
				return
			}
			rt.memo, rt.memoSaved, rt.memoSeen, rt.memoMod = text, text, text, mod
			rt.memoLoaded, rt.memoCheck = true, time.Now()
		}
	})
}

func readMemo(path string) (string, time.Time, error) {
	info, err := os.Stat(path)
	if err != nil {
		return "", time.Time{}, err
	}
	if info.Size() > maxTextBytes {
		return "", time.Time{}, fmt.Errorf("笔记太大（%d MB 以上）", maxTextBytes>>20)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return "", time.Time{}, err
	}
	return string(data), info.ModTime(), nil
}

// saveMemo writes what was typed to the note's file, unless the file holds
// it already: here and now when wait, else off the main thread.
func (d *deck) saveMemo(n *block, rt *runtime, wait bool) {
	if rt == nil || !rt.memoLoaded || rt.memoSaving || rt.memo == rt.memoSaved || n.Path == "" {
		return
	}
	path, text := d.path(n), rt.memo
	write := func() (time.Time, error) {
		if err := writeFileAtomic(path, []byte(text)); err != nil {
			return time.Time{}, err
		}
		info, err := os.Stat(path)
		if err != nil {
			return time.Time{}, err
		}
		return info.ModTime(), nil
	}
	done := func(mod time.Time, err error) {
		rt.memoSaving = false
		if err != nil {
			rt.memoSaveErr = err
			return
		}
		rt.memoSaved, rt.memoMod, rt.memoSaveErr = text, mod, nil
	}
	if wait || d.win == nil {
		done(write())
		return
	}
	rt.memoSaving = true
	go func() {
		mod, err := write()
		d.win.Update(func() { done(mod, err) })
	}()
}

// checkMemo reads the note's file again when something else changed it,
// at most every memoPoll, while nothing typed waits to be written.
func (d *deck) checkMemo(c *ui.Context, n *block, rt *runtime) {
	if !rt.memoLoaded || rt.loading || rt.memoSaving || n.Path == "" {
		return
	}
	now := time.Now()
	if wait := rt.memoCheck.Add(memoPoll).Sub(now); wait > 0 {
		c.After(wait)
		return
	}
	rt.memoCheck = now
	path, known := d.path(n), rt.memoMod
	check := func() (text string, mod time.Time, err error) {
		info, err := os.Stat(path)
		if err != nil || info.ModTime().Equal(known) {
			return "", time.Time{}, err
		}
		return readMemo(path)
	}
	done := func(text string, mod time.Time, err error) {
		rt.loading = false
		switch {
		case err != nil:
			rt.memoErr = err // gone or unreadable: say so
		case !mod.IsZero() && rt.memo == rt.memoSaved:
			rt.memo, rt.memoSaved, rt.memoSeen, rt.memoMod = text, text, text, mod
		}
	}
	if d.win == nil { // tests: no goroutine to hand results to
		done(check())
		return
	}
	rt.loading = true
	go func() {
		text, mod, err := check()
		d.win.Update(func() { done(text, mod, err) })
	}()
}

// renameMemo names the note's file after the title it was given, off the
// main thread; a name taken leaves the file, and the title, as they were.
func (d *deck) renameMemo(n *block) {
	if n.Path == "" {
		return
	}
	root, rel, title := d.root, n.Path, n.Title
	done := func(next string, err error) {
		if n.Path != rel { // moved meanwhile
			return
		}
		n.Path, n.Title, n.Named = next, memoTitleOf(next), false
		if err != nil {
			d.ensureRuntime(n).memoSaveErr = fmt.Errorf("没能改名：%w", err)
		}
		d.requestSave()
	}
	if d.win == nil {
		done(renameMemoFile(root, rel, title))
		return
	}
	go func() {
		next, err := renameMemoFile(root, rel, title)
		d.win.Update(func() { done(next, err) })
	}()
}

// memoBody shows the note rendered, or, being edited, its Markdown.
func (d *deck) memoBody(c *ui.Context, n *block) {
	t := c.Theme()
	rt := d.ensureRuntime(n)
	editing := d.editing == n && d.editPart == editBody
	switch {
	case rt.memoErr != nil:
		bodyMessage(c, rt.memoErr.Error(), t.Danger)
		return
	case !rt.memoLoaded:
		d.loadMemo(n, rt)
		if editing {
			d.editSeen = true // still to be written in, once it is read
		}
		bodyMessage(c, "加载中…", t.TextMuted)
		return
	}
	s := d.lay.S
	size := docFontSize * s
	if editing {
		ui.Box(c).Grow(1).FillWidth().Padding(8*s, 12*s).Children(func() {
			ui.TextArea(c.Key("memo-body:"+n.ID), &rt.memo).Bind(&d.editInput).AutoFocus().Fill().
				FontSize(size).TextColor(t.Text).
				Placeholder("Markdown · Esc 或 ⌘↵ 完成").
				Background(ui.Transparent).Border(0, ui.Transparent).
				OnTextKey(d.endKeys)
			d.watchEditFocus(c)
		})
		d.autosaveMemo(c, n, rt)
		return
	}
	d.saveMemo(n, rt, false) // what an edit left, should its save have failed
	d.checkMemo(c, n, rt)
	// Reading, it is a live preview of its file: focused, it scrolls, and
	// a click on it edits it.
	ui.Box(c).Grow(1).FillWidth().Clip().Children(func() {
		d.livePreview(c, n, [4]float32{12 * s, 16 * s, 12 * s, 16 * s}, func() {
			if rt.memoSaveErr != nil {
				ui.Text(c, rt.memoSaveErr.Error()).FontSize(12 * s).TextColor(t.Danger)
			}
			if strings.TrimSpace(rt.memo) == "" {
				ui.Text(c, "点击开始写，支持 Markdown").FontSize(size).TextColor(t.TextMuted)
				return
			}
			markdown.View(c, rt.memo, d.documentOptions(n, rt, t, size))
		})
	})
}

// autosaveMemo writes the note once typing rests for memoSaveIdle.
func (d *deck) autosaveMemo(c *ui.Context, n *block, rt *runtime) {
	now := time.Now()
	if rt.memo != rt.memoSeen {
		rt.memoSeen, rt.memoEdited = rt.memo, now
	}
	if rt.memo == rt.memoSaved {
		return
	}
	if wait := rt.memoEdited.Add(memoSaveIdle).Sub(now); wait > 0 {
		c.After(wait)
		return
	}
	d.saveMemo(n, rt, false)
}

// flushMemo writes what was typed in the note, here and now: the block is
// leaving the canvas or the deck is closing.
func (d *deck) flushMemo(n *block) {
	if n.Kind == kindMemo && n.rt != nil {
		n.rt.memoSaving = false // a save under way wrote older text
		d.saveMemo(n, n.rt, true)
	}
}
