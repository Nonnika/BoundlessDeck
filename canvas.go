package main

import (
	"math"
	"path/filepath"
	"slices"
	"strings"
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
	d.trackActive()
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
	// Space previews the selected 块堆 for as long as it is open; arrows
	// turn its cards.
	d.stackKeys(c)

	// QuickLook blocks whose time is up go; the frame comes again when
	// the next goes or its time left reads otherwise.
	if wake := d.expireNotes(); wake > 0 {
		c.After(wake)
	}

	d.settleLayer(c)
	ui.Box(c).Fill().Background(t.Background).Children(func() {
		ui.Box(c).Fill().Clip().
			Draw(func(p *ui.Painter, r ui.Rect) { d.drawGrid(p, r, t) }).
			HandleInput(d.onCanvasInput).
			Children(func() {
				// Under the blocks: empty canvas at the top drags the window.
				dragBand(c, d.dragBand)
				// The blocks, laid out with the layout camera, scaled to
				// the canvas's zoom while it goes on (zoomlayer.go).
				built = d.visible(cw, ch)
				// PassThrough: the layer itself is no target, so a press between
				// its blocks reaches what lies under it: the drag band, the canvas.
				layer := ui.Box(c.Key("blocks")).Absolute().Left(0).Top(0).Fill().PassThrough()
				if d.layerMoved() {
					k := d.layerScale()
					ox, oy := d.layerOffset()
					layer = layer.TransformOrigin(0, 0).Scale(k, k).Translate(ox, oy)
					if d.layerCached(built) {
						layer = layer.Cache()
					}
				}
				layer.Children(func() {
					d.editSeen = false
					for _, n := range built {
						d.tapeView(c, n) // behind the block
						d.blockView(c, n)
						d.resizeView(c, n)
					}
					// The block edited went: off the canvas, out of view,
					// or too small to show its body.
					if d.editing != nil && !d.editSeen {
						d.stopEditing()
					}
				})
				d.marqueeView(c) // in the window's DIPs, over the layer
			})
		// What the cards asked to load starts now, the middlemost first.
		if mx, my := d.cam.toWorld(cw/2, ch/2); d.runLoads(mx, my) && d.win == nil {
			c.Invalidate() // tests: it loaded here and now, to show next frame
		}

		d.homePill(c)
		d.toolbar(c)
		d.statusBar(c)
		d.stackPreviewView(c)
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
		d.focusBlock, d.active = "", ""
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
	hidden := d.stackedIDs()
	for _, n := range d.blocks {
		if hidden[n.ID] {
			continue
		}
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
	// The card is laid out with the layout camera: so is ev.
	far := d.lay.S < lodScale
	inTitle := ev.Y <= d.titleHeight(n) || far
	px, py := d.layerToWindow((n.X-d.lay.X)*d.lay.S+ev.X, (n.Y-d.lay.Y)*d.lay.S+ev.Y)
	// The wheel over the body of an open 块堆 scrolls its cards, in the
	// layer's axes as they are laid out.
	if ev.Kind == ui.InputScroll && n.Kind == kindStack && d.stackOpen == n && !inTitle &&
		ev.Mods&(ui.Cmd|ui.Ctrl) == 0 && d.stackScrollBy(n, ev.DY) {
		return true
	}
	// The wheel moved as far in the window: the layer's axes are scaled.
	ev.DX, ev.DY = d.layerDelta(ev.DX, ev.DY)
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
		if d.active != n.ID {
			d.active = "" // a press on another block: the one worked in is left
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
		case n.Kind == kindStack:
			// A click on the pile opens it: hovering then pushes its
			// cards out.
			d.openStack(n, ev.X, ev.Y-d.titleHeight(n))
		case n.Kind == kindNote, n.Kind == kindMemo:
			d.startEditing(n, editBody)
		case n.Kind == kindChart:
			// A click selects the chart, its pointer shows the values; a
			// double click edits its table.
			d.sel[n.ID] = true
			if ev.Clicks >= 2 {
				d.startEditing(n, editBody)
			}
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
			if d.dragMoved {
				// Let go of a block over a stack: the stack takes it.
				if st := d.stackAt(px, py, n); st != nil {
					d.stackAdd(st, n)
					return true
				}
			}
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
	d.dragFrom, d.dragAt, d.dragMoved = point{px, py}, point{px, py}, false
	wx, wy := d.cam.toWorld(px, py)
	d.grab = point{wx, wy}
	d.dragOff = point{n.X - wx, n.Y - wy}
}

// dragTo moves the block being dragged so that the point grabbed is under
// the pointer, at (px, py) in window DIPs.
func (d *deck) dragTo(n *block, px, py float32) {
	d.dragAt = point{px, py}
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
		if toolButton(c.Key("new-note"), iconNote, "QuickLook", "新建 QuickLook（"+newKeyOf(kindNote)+"，或在空白处双击）· 默认 1 小时后消失").Clicked() {
			d.addAtCenter(c, kindNote)
		}
		if toolButton(c.Key("new-memo"), iconMemo, "笔记", "新建笔记：在工作目录里建一个 Markdown 文件（"+newKeyOf(kindMemo)+"）").Clicked() {
			cw, ch := c.Size()
			d.newMemo(cw/2, ch/2)
		}
		if toolButton(c.Key("new-file"), iconFile, "文件", "链接本地文件（"+newKeyOf(kindFile)+"，也可从访达拖入）").Clicked() {
			cw, ch := c.Size()
			go d.pickFile(cw/2, ch/2)
		}
		if toolButton(c.Key("new-term"), iconTerminal, "终端", "新建终端（"+newKeyOf(kindTerminal)+"）").Clicked() {
			d.addAtCenter(c, kindTerminal)
		}
		if toolButton(c.Key("new-browser"), iconGlobe, "浏览器", "新建浏览器（"+newKeyOf(kindBrowser)+"）").Clicked() {
			d.addAtCenter(c, kindBrowser)
		}
		if toolButton(c.Key("new-chart"), iconChart, "图表", "新建图表：柱状图、条形图、折线图、面积图、饼图，双击编辑数据（"+newKeyOf(kindChart)+"）").Clicked() {
			d.addAtCenter(c, kindChart)
		}
		if toolButton(c.Key("new-stack"), iconStack, "块堆", "新建块堆：拖 block 的胶带或标题栏进来，点击后悬停推出卡片、上下边缘滚动，空格长预览（"+newKeyOf(kindStack)+"）").Clicked() {
			d.addAtCenter(c, kindStack)
		}
	}).Label("新建").Absolute().Top(chromeTop).Right(12).Padding(5).Gap(2).
		Background(t.Surface).Border(1, t.Border).Radius(12).
		Shadow(0, 4, 16, 0, shade)
}

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
	// A toolbar, as the one at the right: its buttons have no face of
	// their own until hovered, the pill's alone shows.
	ui.Toolbar(c, func() {
		ui.Button(c.Key("go-home"), "").Label("返回主页").Tooltip("保存并返回主页").
			Padding(5, 7).Radius(8).
			Children(func() { ui.Icon(c, iconBack).FontSize(15) }).
			OnClick(d.onHome)
		ws := ui.ButtonBase(c.Key("workspace")).Label("切换工作区").Tooltip(shortPath(d.root)+" · 点击切换工作区").
			Padding(5, 8, 5, 8).Gap(6).Radius(8).Row().AlignItems(ui.Center)
		if ws.Hovered() {
			ws = ws.Background(t.SurfaceHover)
		}
		ws.Children(func() {
			ui.Icon(c, iconFolder).FontSize(15)
			ui.Text(c, filepath.Base(d.root)).FontSize(12).FontWeight(600).SingleLine()
			ui.Icon(c, iconChevronDown).FontSize(13).TextColor(t.TextMuted)
		}).Menu(d.workspaceMenu)
	}).Label("工作区").Absolute().Top(chromeTop).Left(chromeLeft(c)).Padding(5).Gap(2).
		Background(t.Surface).Border(1, t.Border).Radius(12).Shadow(0, 4, 16, 0, shade)
}

// workspaceMenu lists the workspaces opened lately, this one checked, to
// switch to, and opens another. Home is the pill's back button.
func (d *deck) workspaceMenu(m *ui.Menu) {
	var paths []string
	if d.workspaces != nil {
		paths = d.workspaces()
	}
	if !slices.Contains(paths, d.root) {
		paths = append([]string{d.root}, paths...)
	}
	for i, path := range paths {
		if i == maxWorkspaceItems {
			break
		}
		label := filepath.Base(path) + "    " + shortPath(filepath.Dir(path))
		if m.Item(label).Checked(path == d.root).Chosen() && path != d.root && d.onOpen != nil {
			d.onOpen(path)
		}
	}
	m.Separator()
	if m.Item("打开文件夹…").Shortcut(ui.Cmd, ui.KeyO).Chosen() && d.onPick != nil {
		d.onPick()
	}
}

// maxWorkspaceItems is how many workspaces the menu lists.
const maxWorkspaceItems = 12

// statusBar shows the zoom and what is selected, at the window's bottom left.
func (d *deck) statusBar(c *ui.Context) {
	t := c.Theme()
	ui.Row(c).Absolute().Left(12).Bottom(10).Padding(4, 8).Gap(10).
		Background(t.Surface).Border(1, t.Border).Radius(9).Children(func() {
		ui.Textf(c, "%.0f%%", d.cam.S*100).FontSize(11).TextColor(t.TextMuted)
		ui.Textf(c, "%d 个 block · 选中 %d", len(d.blocks), len(d.sel)).FontSize(11).TextColor(t.TextMuted)
		d.mcpStatus(c, t)
	})
}

// mcpStatus shows the workspace's MCP endpoint: its tools, and the
// project-level config it wrote, in the tooltip.
func (d *deck) mcpStatus(c *ui.Context, t *ui.Theme) {
	switch {
	case d.mcp != nil:
		tip := mcpProjectSummary(d.mcpProject, d.projectMCP) + "\n" +
			filepath.Join(deckDir, "mcp.json") + " · " + d.mcp.sock
		if d.mcpProject != nil && len(d.mcpProject.Warnings) > 0 {
			tip += "\n" + strings.Join(d.mcpProject.Warnings, "\n")
		}
		if d.mcpErr != nil {
			tip += "\n" + d.mcpErr.Error()
		}
		ui.Textf(c, "MCP · %d 个工具", len(mcpToolList)).FontSize(11).TextColor(t.TextMuted).Tooltip(tip)
	case d.mcpErr != nil:
		ui.Text(c, "MCP 未启动").FontSize(11).TextColor(t.Danger).Tooltip(d.mcpErr.Error())
	}
}

// addAtPointer adds a block of the kind under the mouse, as a shortcut
// of the menu does: in the middle of the view when the mouse is outside
// the window.
func (d *deck) addAtPointer(kind blockKind) {
	x, y := d.viewW/2, d.viewH/2
	if d.win != nil {
		if px, py, ok := cursorIn(mygo.Screen.CursorScreenPoint(), d.win.ContentBounds(), d.viewW, d.viewH); ok {
			x, y = px, py
		}
	}
	d.addKindAt(kind, x, y)
}

// cursorIn is where the mouse, at p on the screen, is in a window whose
// content lies at content and shows w × h DIPs; ok says it is in it.
func cursorIn(p mygo.Point, content mygo.Rectangle, w, h float32) (x, y float32, ok bool) {
	x, y = float32(p.X-content.X), float32(p.Y-content.Y)
	if content.Width > 0 && content.Height > 0 {
		// The content in DIPs, should the UI be zoomed.
		x *= w / float32(content.Width)
		y *= h / float32(content.Height)
	}
	return x, y, x >= 0 && y >= 0 && x < w && y < h
}

// addKindAt adds a block of any kind at a point of the window, in DIPs:
// a 笔记 makes its file, a file block asks which file first.
func (d *deck) addKindAt(kind blockKind, sx, sy float32) {
	switch kind {
	case kindMemo:
		d.newMemo(sx, sy)
	case kindFile:
		go d.pickFile(sx, sy)
	default:
		d.addAt(kind, sx, sy)
	}
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
		b.Expires = d.now().Add(noteLife).UnixMilli()
	case kindFile:
		b.Title = "文件"
		b.W, b.H = 560, 420
	case kindTerminal:
		b.Title = "终端"
		b.W, b.H = 560, 340
	case kindBrowser:
		b.Title = "浏览器"
		b.W, b.H = 760, 540
	case kindStack:
		b.Title = "块堆"
		b.W, b.H = 380, 300
	case kindChart:
		b.Title = "图表"
		b.W, b.H = 480, 320
		b.Chart, b.Text = string(chartBar), chartSample
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

// pickFile asks for a file and adds the block that shows it at (sx, sy)
// in the window, in DIPs.
func (d *deck) pickFile(sx, sy float32) {
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
		wx, wy := d.cam.toWorld(sx, sy)
		b.X, b.Y = d.freeSpot(wx-b.W/2, wy-b.H/2, b.W, b.H)
		d.addBlock(b)
	})
}

// fitAll frames every block, or resets the camera when there is none.
func (d *deck) fitAll(cw, ch float32) {
	hidden := d.stackedIDs()
	vis := make([]*block, 0, len(d.blocks))
	for _, n := range d.blocks {
		if !hidden[n.ID] {
			vis = append(vis, n)
		}
	}
	if len(vis) == 0 {
		d.cam = camera{X: 0, Y: 0, S: 1}
		return
	}
	minX, minY := vis[0].X, vis[0].Y
	maxX, maxY := vis[0].X+vis[0].W, vis[0].Y+vis[0].H
	for _, n := range vis[1:] {
		minX, minY = min(minX, n.X), min(minY, n.Y)
		maxX, maxY = max(maxX, n.X+n.W), max(maxY, n.Y+n.H)
	}
	const pad = 60
	w, h := maxX-minX+2*pad, maxY-minY+2*pad
	s := min(cw/w, ch/h)
	// On a zoom level at most as large, so that it still frames them all.
	d.cam.S = zoomLevelAtMost(min(max(s, minScale), 1))
	d.cam.X = minX - (cw/d.cam.S-w)/2
	d.cam.Y = minY - (ch/d.cam.S-h)/2
}

func abs(v float32) float32 {
	if v < 0 {
		return -v
	}
	return v
}
