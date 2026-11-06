package main

import (
	"math"
	"path/filepath"
	goruntime "runtime"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// view builds the window's frame: the canvas, its blocks and the chrome.
func (d *deck) view(c *ui.Context) {
	d.perfBegin()
	var built []*block
	defer func() { d.perfEnd(built) }()
	cw, ch := c.Size()
	d.viewW, d.viewH = cw, ch
	d.dragBand = c.TitleBar().Height
	t := c.Theme()

	if c.Shortcut(ui.Cmd, ui.Key0) {
		d.fitAll(cw, ch)
	}
	if c.Shortcut(ui.Cmd, ui.KeyEqual) {
		d.zoomStep(cw/2, ch/2, 1)
	}
	if c.Shortcut(ui.Cmd, ui.KeyMinus) {
		d.zoomStep(cw/2, ch/2, -1)
	}
	// While a block's title or a QuickLook body is edited, ⌘Z and ⌘D are
	// the input's: the window's shortcuts would otherwise take them first.
	if d.editing == nil {
		if c.Shortcut(ui.Cmd|ui.Shift, ui.KeyZ) {
			d.redoLast()
		} else if c.Shortcut(ui.Cmd, ui.KeyZ) {
			d.undoLast()
		}
		// ⌘D on macOS, Ctrl+D elsewhere: ui.Cmd is the platform's modifier.
		if c.Shortcut(ui.Cmd, ui.KeyD) {
			d.deleteSelected()
		}
	}

	ui.Box(c).Fill().Background(t.Background).Children(func() {
		ui.Box(c).Fill().Clip().
			Draw(func(p *ui.Painter, r ui.Rect) { d.drawGrid(p, r, t) }).
			HandleInput(d.onCanvasInput).
			Children(func() {
				d.editSeen = false
				for _, n := range d.visible(cw, ch) {
					d.tapeView(c, n) // behind the block
					d.blockView(c, n)
					d.resizeView(c, n)
				}
				// The block edited went: off the canvas, out of view, or
				// too small to show its body.
				if d.editing != nil && !d.editSeen {
					d.stopEditing()
				}
				d.marqueeView(c)
			})

		d.homePill(c)
		d.toolbar(c)
		d.statusBar(c)
	})
}

// drawGrid paints the world's dots, a step apart chosen so that they are at
// least 24 DIPs apart on the screen.
func (d *deck) drawGrid(p *ui.Painter, r ui.Rect, t *ui.Theme) {
	step := float32(20)
	for step*d.cam.S < 24 {
		step *= 2.5
	}
	for step*d.cam.S > 90 {
		step /= 2.5
	}
	// Round dots of a fixed size on the screen, centered on the grid
	// points, in the muted text color faded back: plain to see, quieter
	// than the blocks.
	const dot = 3
	dotColor := t.TextMuted.Alpha(0.55)
	x0 := math.Floor(float64(d.cam.X/step)) * float64(step)
	y0 := math.Floor(float64(d.cam.Y/step)) * float64(step)
	for wy := y0; ; wy += float64(step) {
		py := (float32(wy) - d.cam.Y) * d.cam.S
		if py > r.H {
			break
		}
		for wx := x0; ; wx += float64(step) {
			px := (float32(wx) - d.cam.X) * d.cam.S
			if px > r.W {
				break
			}
			if px >= 0 && py >= 0 {
				p.Fill(ui.Rect{X: r.X + px - dot/2, Y: r.Y + py - dot/2, W: dot, H: dot}, dotColor, dot/2)
			}
		}
	}
}

// wheelZoom is the factor ⌘ and the wheel zoom by for dy DIPs of
// scrolling: a notch zooms about the pointer, a touchpad's pixels less.
func wheelZoom(dy float32) float32 {
	return float32(math.Pow(1.0016, float64(-dy)))
}

// onCanvasInput handles what no block takes: panning, zooming, the marquee.
func (d *deck) onCanvasInput(ev ui.InputEvent) bool {
	switch ev.Kind {
	case ui.InputScroll:
		d.canvasScrollUntil = time.Now().Add(gestureHold)
		if ev.Mods&ui.Cmd != 0 || ev.Mods&ui.Ctrl != 0 {
			perff("event ⌘-wheel dy %.1f precise %v at %.0f,%.0f", ev.DY, ev.Precise, ev.X, ev.Y)
			// A notch zooms about the pointer; a touchpad's pixels zoom less.
			d.zoomAt(ev.X, ev.Y, wheelZoom(ev.DY))
			d.zoomGesture()
			return true
		}
		if ev.Mods&ui.Shift != 0 && ev.DX == 0 {
			ev.DX, ev.DY = ev.DY, 0
		}
		d.cam.X += ev.DX / d.cam.S
		d.cam.Y += ev.DY / d.cam.S
		return true
	case ui.InputZoom:
		perff("event pinch ×%.4f at %.0f,%.0f", ev.Scale, ev.X, ev.Y)
		perfStart()
		d.zoomAt(ev.X, ev.Y, ev.Scale)
		d.zoomGesture()
		return true
	case ui.InputPointerDown:
		// Empty canvas in the band along the top is the window's to drag
		// (titlebar.go).
		if ev.Button == 0 && ev.Y < d.dragBand {
			return false
		}
		if ev.Button != 0 {
			d.panning = true
			d.panFrom = point{ev.X, ev.Y}
			d.panCam = d.cam
			return true
		}
		// A press in a browser's body, which its card left to its bar,
		// is the bar's: its buttons' and its address bar's.
		if d.inBrowserBody(ev.X, ev.Y) {
			return false
		}
		// A press on the canvas takes the focus from the blocks: none
		// takes the wheel then.
		d.focusBlock = ""
		if ev.Clicks == 2 {
			d.addAt(kindNote, ev.X, ev.Y)
			return true
		}
		wx, wy := d.cam.toWorld(ev.X, ev.Y)
		d.marquee = marquee{on: true, from: point{wx, wy}, to: point{wx, wy}}
		return true
	case ui.InputPointerMove:
		if d.panning {
			d.cam.X = d.panCam.X - (ev.X-d.panFrom.X)/d.cam.S
			d.cam.Y = d.panCam.Y - (ev.Y-d.panFrom.Y)/d.cam.S
			return true
		}
		if d.marquee.on {
			wx, wy := d.cam.toWorld(ev.X, ev.Y)
			d.marquee.to = point{wx, wy}
			return true
		}
	case ui.InputPointerUp:
		if d.panning {
			d.panning = false
			return true
		}
		if d.marquee.on {
			d.selectIn(d.marqueeRect())
			d.marquee.on = false
			return true
		}
	}
	return false
}

func (d *deck) marqueeRect() ui.Rect {
	x0, y0 := d.marquee.from.X, d.marquee.from.Y
	x1, y1 := d.marquee.to.X, d.marquee.to.Y
	x, y := min(x0, x1), min(y0, y1)
	return ui.Rect{X: x, Y: y, W: abs(x1 - x0), H: abs(y1 - y0)}
}

func (d *deck) selectIn(r ui.Rect) {
	d.sel = map[string]bool{}
	// A press alone is no marquee: it clears the selection.
	if r.W*d.cam.S < 3 && r.H*d.cam.S < 3 {
		return
	}
	for _, n := range d.blocks {
		if float32(n.X) < r.X+r.W && float32(n.X+n.W) > r.X && float32(n.Y) < r.Y+r.H && float32(n.Y+n.H) > r.Y {
			d.sel[n.ID] = true
		}
	}
}

// onBlockInput takes the presses a card's body leaves. A click works the
// block rather than selecting it: the title bar drags it, and on a
// QuickLook block a click on the title or the body edits it. Shift+click
// adds it to the selection or takes it out; the marquee selects too.
func (d *deck) onBlockInput(ev ui.InputEvent, n *block) bool {
	// Zoomed out, a block is its frame alone, its title bar too thin to
	// aim at: it drags from anywhere, and edits nothing.
	far := d.cam.S < lodScale
	inTitle := ev.Y <= d.titleHeight(n) || far
	px := (n.X-d.cam.X)*d.cam.S + ev.X
	py := (n.Y-d.cam.Y)*d.cam.S + ev.Y
	// A browser's bar and page take their presses themselves: its
	// buttons and address bar, which the card would take them from.
	if n.Kind == kindBrowser && !inTitle {
		switch ev.Kind {
		case ui.InputPointerDown, ui.InputPointerMove, ui.InputPointerUp:
			return false
		}
	}
	switch ev.Kind {
	case ui.InputScroll, ui.InputZoom:
		// The wheel and a pinch over a card are the canvas's: a card holds
		// no scroller of its own, and its bodies that do take them first.
		ev.X, ev.Y = px, py
		return d.onCanvasInput(ev)
	case ui.InputPointerDown:
		if ev.Button != 0 {
			return false
		}
		if ev.Mods&ui.Shift != 0 {
			d.sel[n.ID] = !d.sel[n.ID]
			return true
		}
		d.sel = map[string]bool{}
		switch {
		case inTitle:
			// A drag moves the block; a click without one edits its
			// title, once released.
			d.beginDrag(n, px, py)
		case n.Kind == kindNote:
			d.startEditing(n, editBody)
		}
		return true
	case ui.InputPointerMove:
		if d.dragging != n {
			return false
		}
		d.dragTo(n, px, py)
		return true
	case ui.InputPointerUp:
		if d.dragging == n {
			d.dragging = nil
			// A click on a title, without a drag, edits it.
			if !d.dragMoved && inTitle && !far {
				d.startEditing(n, editTitle)
			}
		}
		return true
	}
	return false
}

// beginDrag starts moving n with the pointer, at (px, py) in window DIPs.
func (d *deck) beginDrag(n *block, px, py float32) {
	d.dragging = n
	d.dragFrom, d.dragMoved = point{px, py}, false
	wx, wy := d.cam.toWorld(px, py)
	d.grab = point{wx, wy}
	d.dragOff = point{n.X - wx, n.Y - wy}
}

// dragTo moves the block being dragged so that the point grabbed is under
// the pointer, at (px, py) in window DIPs.
func (d *deck) dragTo(n *block, px, py float32) {
	// A few DIPs of jitter is still a click.
	if abs(px-d.dragFrom.X) > 3 || abs(py-d.dragFrom.Y) > 3 {
		d.dragMoved = true
	}
	wx, wy := d.cam.toWorld(px, py)
	n.X, n.Y = wx+d.dragOff.X, wy+d.dragOff.Y
}

// titleHeightAt1 is the height of a block's title bar at 100%.
const titleHeightAt1 = 26

// titleHeight is the height of a block's title bar in its layer, laid
// out with the layout camera (zoomlayer.go).
func (d *deck) titleHeight(n *block) float32 {
	return titleHeightAt1 * d.lay.S
}

func (d *deck) marqueeView(c *ui.Context) {
	if !d.marquee.on {
		return
	}
	m := d.marqueeRect()
	x, y := d.cam.toScreen(m.X, m.Y)
	t := c.Theme()
	ui.Box(c).Absolute().Left(x).Top(y).Size(m.W*d.cam.S, m.H*d.cam.S).
		Border(1, t.Accent).Radius(2)
}

// toolbar is the new-block bar at the window's top right.
func (d *deck) toolbar(c *ui.Context) {
	t := c.Theme()
	shade := ui.RGBA(0, 0, 0, 0.08)
	if t.Dark {
		shade = ui.RGBA(0, 0, 0, 0.4)
	}
	ui.Toolbar(c, func() {
		if toolButton(c.Key("new-note"), iconNote, "QuickLook", "新建 QuickLook（或在空白处双击）").Clicked() {
			d.addAtCenter(c, kindNote)
		}
		if toolButton(c.Key("new-file"), iconFile, "文件…", "链接本地文件（也可从访达拖入）").Clicked() {
			go d.pickFile()
		}
		if toolButton(c.Key("new-term"), iconTerminal, "终端", "新建终端").Clicked() {
			d.addAtCenter(c, kindTerminal)
		}
		if toolButton(c.Key("new-browser"), iconGlobe, "浏览器", "新建浏览器").Clicked() {
			d.addAtCenter(c, kindBrowser)
		}
		ui.Divider(c).Margin(6, 2)
		n := len(d.sel)
		del := toolButton(c.Key("delete-selected"), iconTrash, "删除", "删除选中（"+deleteKeys+"）", func() {
			// The count is a badge after the label, there only while
			// something is selected.
			if n > 0 {
				ui.Textf(c, "%d", n).FontSize(10).SingleLine().
					Padding(1, 6).Radius(999).Background(t.Danger).TextColor(t.AccentText)
			}
		}).Disabled(n == 0)
		if n > 0 {
			del = del.TextColor(t.Danger)
		}
		del.OnClick(d.deleteSelected)
	}).Label("新建").Absolute().Top(chromeTop).Right(12).Padding(5).Gap(2).
		Background(t.Surface).Border(1, t.Border).Radius(12).
		Shadow(0, 4, 16, 0, shade)
}

// deleteKeys names the delete-selection shortcut as the platform writes it.
var deleteKeys = func() string {
	if goruntime.GOOS == "darwin" {
		return "⌘D"
	}
	return "Ctrl+D"
}()

// toolButton is a toolbar button with an icon before its label.
// extra, when given, builds what follows the label.
func toolButton(c *ui.Context, icon *ui.SVG, label, tip string, extra ...func()) ui.Element {
	return ui.Button(c, "").Label(label).Tooltip(tip).Padding(5, 10).Gap(6).Radius(8).
		Children(func() {
			ui.Icon(c, icon).FontSize(15)
			ui.Text(c, label).FontSize(12).SingleLine()
			for _, fn := range extra {
				fn()
			}
		})
}

// homePill names the deck's folder at the window's top left; a click
// saves the deck and goes back to the home screen.
func (d *deck) homePill(c *ui.Context) {
	if d.onHome == nil {
		return
	}
	t := c.Theme()
	shade := ui.RGBA(0, 0, 0, 0.08)
	if t.Dark {
		shade = ui.RGBA(0, 0, 0, 0.4)
	}
	ui.Row(c).Absolute().Top(chromeTop).Left(chromeLeft(c)).Padding(5).
		Background(t.Surface).Border(1, t.Border).Radius(12).Shadow(0, 4, 16, 0, shade).
		Children(func() {
			ui.Button(c.Key("go-home"), "").Label("返回主页").Tooltip("保存并返回主页（"+shortPath(d.root)+"）").
				Padding(5, 10, 5, 6).Gap(6).Radius(8).
				Children(func() {
					ui.Icon(c, iconBack).FontSize(15)
					ui.Icon(c, iconFolder).FontSize(15)
					ui.Text(c, filepath.Base(d.root)).FontSize(12).FontWeight(600).SingleLine()
				}).
				OnClick(d.onHome)
		})
}

// statusBar shows the zoom and what is selected, at the window's bottom left.
func (d *deck) statusBar(c *ui.Context) {
	t := c.Theme()
	ui.Row(c).Absolute().Left(12).Bottom(10).Padding(4, 8).Gap(10).
		Background(t.Surface).Border(1, t.Border).Radius(9).Children(func() {
		ui.Textf(c, "%.0f%%", d.cam.S*100).FontSize(11).TextColor(t.TextMuted)
		ui.Textf(c, "%d 个 block · 选中 %d", len(d.blocks), len(d.sel)).FontSize(11).TextColor(t.TextMuted)
	})
}

// addAtCenter adds a block of the kind at the middle of the view.
func (d *deck) addAtCenter(c *ui.Context, kind blockKind) {
	cw, ch := c.Size()
	d.addAt(kind, float32(cw)/2, float32(ch)/2)
}

// addAt adds a block of the kind at a point of the window, in DIPs, or as
// near it as no other block is.
func (d *deck) addAt(kind blockKind, sx, sy float32) *block {
	wx, wy := d.cam.toWorld(sx, sy)
	b := &block{Kind: kind, W: 360, H: 260}
	switch kind {
	case kindNote:
		b.Title = "QuickLook"
	case kindFile:
		b.Title = "文件"
		b.W, b.H = 560, 420
	case kindTerminal:
		b.Title = "终端"
		b.W, b.H = 560, 340
	case kindBrowser:
		b.Title = "浏览器"
		b.W, b.H = 760, 540
	}
	b.X, b.Y = d.freeSpot(wx-b.W/2, wy-b.H/2, b.W, b.H)
	d.addBlock(b)
	switch kind {
	case kindNote:
		// A new QuickLook block is there to be written in.
		d.startEditing(b, editBody)
	case kindBrowser:
		// A new browser is there to type an address in.
		d.focusBlock = addressFocus(b)
	}
	return b
}

// pickFile asks for a file and adds the block that shows it.
func (d *deck) pickFile() {
	files, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
		Parent: d.win,
		Title:  "链接一个本地文件",
		Filters: []mygo.FileFilter{
			{Name: "所有文件"},
		},
	})
	if err != nil || len(files) == 0 {
		return
	}
	path := files[0]
	rel := d.linkPath(path)
	win := d.win
	win.Update(func() {
		b := &block{Kind: kindFile, Title: filepath.Base(path), Path: rel}
		b.W, b.H = fileBlockSize(path)
		wx, wy := d.cam.toWorld(640, 400)
		b.X, b.Y = d.freeSpot(wx-b.W/2, wy-b.H/2, b.W, b.H)
		d.addBlock(b)
	})
}

// fitAll frames every block, or resets the camera when there is none.
func (d *deck) fitAll(cw, ch float32) {
	if len(d.blocks) == 0 {
		d.cam = camera{X: 0, Y: 0, S: 1}
		return
	}
	minX, minY := d.blocks[0].X, d.blocks[0].Y
	maxX, maxY := d.blocks[0].X+d.blocks[0].W, d.blocks[0].Y+d.blocks[0].H
	for _, n := range d.blocks[1:] {
		minX, minY = min(minX, n.X), min(minY, n.Y)
		maxX, maxY = max(maxX, n.X+n.W), max(maxY, n.Y+n.H)
	}
	const pad = 60
	w, h := maxX-minX+2*pad, maxY-minY+2*pad
	s := min(cw/w, ch/h)
	d.cam.S = min(max(s, minScale), 1)
	d.cam.X = minX - (cw/d.cam.S-w)/2
	d.cam.Y = minY - (ch/d.cam.S-h)/2
}

func abs(v float32) float32 {
	if v < 0 {
		return -v
	}
	return v
}
