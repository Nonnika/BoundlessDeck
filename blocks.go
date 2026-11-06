package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/egoist/mygo/plugins/terminal"
	"github.com/egoist/mygo/transfer"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/editor"
)

// blockView builds one card: its frame, its title bar and its body.
func (d *deck) blockView(c *ui.Context, n *block) {
	if n.Kind == kindNote {
		d.stickyView(c, n)
		return
	}
	t := c.Theme()
	s := d.lay.S
	sx, sy := d.lay.toScreen(n.X, n.Y)
	w, h := n.W*s, n.H*s

	card := ui.Box(c.Key("block:"+n.ID)).Absolute().Left(sx).Top(sy).Size(w, h).
		Background(t.Surface)
	if d.sel[n.ID] {
		card = card.Border(2, t.Accent)
	} else {
		card = card.Border(1, t.Border)
	}
	card = card.Clip().HandleInput(func(ev ui.InputEvent) bool { return d.onBlockInput(ev, n) })

	// Below the LOD threshold a card is its frame and its title alone: no
	// editor, no terminal, no bitmaps. Its content comes and goes blurred.
	shown := d.contentShown(card)
	if shown <= 0 {
		card.Draw(func(p *ui.Painter, r ui.Rect) {
			d.drawFarTitle(p, r, n, 8, t.Text)
		})
		return
	}

	card.Children(func() {
		d.titleBar(c, n)
		fadeContent(ui.Box(c).Grow(1).FillWidth().Clip(), shown).Children(func() {
			d.body(c, n)
			d.bodyCover(c, n)
		})
	})
}

// As the zoom crosses lodScale, a card's content does not pop in or out:
// it comes clear from a blur, or blurs away, over contentFade. It starts
// contentBlur DIPs blurred, and transparent.
const (
	contentFade = 280 * time.Millisecond
	contentBlur = 10
)

// contentShown is how much of the card's content shows: 0, its frame and
// title alone, below lodScale; 1 above; between while it comes or goes.
// A card seen for the first time shows as it is, with no fade.
func (d *deck) contentShown(card ui.Element) float32 {
	target := float32(0)
	if d.lay.S >= lodScale {
		target = 1
	}
	v := max(card.AnimateWith("content", target, contentFade, ui.EaseInOut), 0)
	if target == 1 {
		// Coming: built at once, fully blurred, so that what it loads is
		// there by the time it is clear.
		v = max(v, 0.001)
	}
	return v
}

// fadeContent blurs and fades a card's content as much as it does not show.
func fadeContent(e ui.Element, shown float32) ui.Element {
	if shown >= 1 {
		return e
	}
	return e.Blur((1 - shown) * contentBlur).Opacity(shown)
}

// titleFontSize is the size of a block's title at 100%: the canvas's zoom
// scales it, with no floor, as it scales the block.
const titleFontSize = 12

// drawFarTitle paints a block's title as the block shows zoomed out, its
// frame alone: where and as large as its title bar would show it, padX
// from the left at 100%, and cut at the block's edges.
func (d *deck) drawFarTitle(p *ui.Painter, r ui.Rect, n *block, padX float32, color ui.Color) {
	s := d.lay.S
	size := titleFontSize * s
	y := r.Y + (d.titleHeight(n)-size*1.25)/2
	p.Clip(r, 0, func() {
		p.Text(r.X+padX*s, y, n.Title, size, color)
	})
}

// titleBar shows the block's title, or the input it is edited in: a click
// on it, without a drag, edits it (onBlockInput).
func (d *deck) titleBar(c *ui.Context, n *block) {
	t := c.Theme()
	s := d.lay.S
	ui.Row(c).Height(d.titleHeight(n)).Padding(0, 8*s).AlignItems(ui.Center).Children(func() {
		if d.editing == n && d.editPart == editTitle {
			in := ui.TextInput(c.Key("title:"+n.ID), &n.Title).Bind(&d.editInput).AutoFocus().
				Label("标题").Grow(1).FontSize(titleFontSize*s).TextColor(t.Text).
				Background(ui.Transparent).Border(0, ui.Transparent).Padding(0).Radius(0).
				OnTextKey(d.endKeys)
			if in.Submitted() {
				d.stopEditing()
			}
			d.watchEditFocus(c)
			return
		}
		ui.Text(c, n.Title).FontSize(titleFontSize * s).SingleLine().Grow(1).TextColor(t.Text)
	})
}

func (d *deck) body(c *ui.Context, n *block) {
	switch n.Kind {
	case kindFile:
		switch previewOf(n.Path) {
		case previewImage:
			d.imageBody(c, n)
		case previewMarkdown, previewWord:
			d.documentBody(c, n)
		case previewPDF:
			d.pdfBody(c, n)
		default:
			d.textBody(c, n)
		}
	case kindTerminal:
		d.terminalBody(c, n)
	case kindBrowser:
		d.browserBody(c, n)
	}
}

// textBody shows a linked file read-only, in the editor.
func (d *deck) textBody(c *ui.Context, n *block) {
	t := c.Theme()
	rt := d.ensureRuntime(n)
	d.loadText(n, rt)
	switch {
	case rt.textErr != nil:
		ui.Box(c).Fill().Padding(10).Children(func() {
			ui.Text(c, rt.textErr.Error()).FontSize(12).TextColor(t.Danger)
		})
	case rt.editor == nil:
		ui.Box(c).Fill().Padding(10).Children(func() {
			ui.Text(c, "加载中…").FontSize(12).TextColor(t.TextMuted)
		})
	default:
		// The text grows and shrinks with the canvas, as the block does.
		if size := editorFontSize * d.lay.S; rt.editor.Font.Size != size {
			rt.editor.Font.Size = size
		}
		editor.View(c, rt.editor).Grow(1).FillWidth().FocusBind(&d.focusBlock, n.ID)
	}
}

func (d *deck) imageBody(c *ui.Context, n *block) {
	t := c.Theme()
	rt := d.ensureRuntime(n)
	d.loadImage(n, rt)
	switch {
	case rt.imgErr != nil:
		ui.Box(c).Fill().Padding(10).Children(func() {
			ui.Text(c, rt.imgErr.Error()).FontSize(12).TextColor(t.Danger)
		})
	case rt.source == nil:
		ui.Box(c).Fill().Padding(10).Children(func() {
			ui.Text(c, "加载中…").FontSize(12).TextColor(t.TextMuted)
		})
	default:
		src, alpha, s := rt.source, rt.imgAlpha, d.lay.S
		dark := t.Dark
		ui.Box(c).Grow(1).FillWidth().
			Draw(func(p *ui.Painter, r ui.Rect) {
				if alpha {
					drawChecker(p, contained(src, r), 8*s, dark)
				}
			}).
			Children(func() {
				ui.Image(c, src).Fill().Fit(ui.Contain)
			})
	}
}

// contained is where a picture shows in r, as Fit(ui.Contain) puts it:
// as large as fits, its proportions kept, in the middle.
func contained(src ui.ImageSource, r ui.Rect) ui.Rect {
	iw, ih := ui.ImageSize(src)
	if iw <= 0 || ih <= 0 || r.W <= 0 || r.H <= 0 {
		return r
	}
	k := min(r.W/iw, r.H/ih)
	w, h := iw*k, ih*k
	return ui.Rect{X: r.X + (r.W-w)/2, Y: r.Y + (r.H-h)/2, W: w, H: h}
}

// drawChecker paints the checkerboard that shows through a picture where
// it is see-through, as image editors do, in squares cell DIPs wide.
func drawChecker(p *ui.Painter, r ui.Rect, cell float32, dark bool) {
	cell = max(cell, 4)
	light, shade := ui.Hex("#ffffff"), ui.Hex("#e3e3e6")
	if dark {
		light, shade = ui.Hex("#3a3b40"), ui.Hex("#2c2d31")
	}
	p.Fill(r, light, 0)
	p.Clip(r, 0, func() {
		for row, y := 0, r.Y; y < r.Y+r.H; row, y = row+1, y+cell {
			for col, x := 0, r.X; x < r.X+r.W; col, x = col+1, x+cell {
				if (row+col)%2 == 1 {
					p.Fill(ui.Rect{X: x, Y: y, W: cell, H: cell}, shade, 0)
				}
			}
		}
	})
}

// terminalBody shows a shell in the block, started the first time the body
// is built.
func (d *deck) terminalBody(c *ui.Context, n *block) {
	t := c.Theme()
	rt := d.ensureRuntime(n)
	if rt.term == nil {
		d.startTerminal(n, rt)
		msg := "启动终端…"
		if rt.termErr != nil {
			msg = rt.termErr.Error()
		}
		ui.Box(c).Fill().Padding(10).Children(func() {
			ui.Text(c, msg).FontSize(12).TextColor(t.TextMuted)
		})
		return
	}
	// The terminal's text grows and shrinks with the canvas, as the block
	// does: its grid keeps its rows and columns, and nothing reflows.
	// After MyGo's text system lets go of its fonts (fontsForgotten), the
	// terminal is handed a font it has not seen, so it shapes its rows anew
	// rather than drawing glyphs of fonts that are gone.
	epoch := fontEpoch()
	if size := terminalFontSize * d.lay.S; rt.termFont != size || rt.termEpoch != epoch {
		f := terminal.Font{Size: size}
		if epoch%2 == 1 {
			f.LineHeight = 1 + 1e-6 // the same rows, as another font to the terminal
		}
		rt.term.SetFont(f)
		rt.termFont, rt.termEpoch = size, epoch
	}
	terminal.View(c, rt.term).Grow(1).FillWidth().FocusBind(&d.focusBlock, n.ID)
}

// termZoomHold is how long after its last ⌘ and wheel a terminal's zoom
// still goes to the canvas directly.
const termZoomHold = 400 * time.Millisecond

// gestureHold is how long after its last move a scroll the canvas took
// is still the canvas's: a pan that slides over a block goes on panning.
const gestureHold = 300 * time.Millisecond

// hasScroller reports whether the block's body scrolls what it shows, as
// a terminal's scrollback or a file's text does, and so takes the wheel
// from the canvas once focused.
func hasScroller(n *block) bool {
	switch n.Kind {
	case kindTerminal, kindBrowser:
		return true
	case kindFile:
		switch previewOf(n.Path) {
		case previewText, previewMarkdown, previewWord:
			return true
		}
	}
	return false
}

// bodyCover covers the body of a block that scrolls (hasScroller) while
// the wheel is the canvas's, so that its moves come to the canvas:
//
//   - while the block is not focused: a two-finger pan, or ⌘ and the
//     wheel, over it moves the canvas, as over the canvas itself. A click
//     on the cover focuses the block, whose body then takes the wheel;
//   - while a scroll the canvas took goes on, for a pan that slides onto
//     the focused block;
//   - while ⌘ and the wheel zoom over a terminal: the terminal would add a
//     touchpad's moves up to steps of 40 DIPs, the canvas holding still
//     and then jumping. Its first step starts the zoom (zoomTerminal), the
//     cover takes the rest, until ⌘ is let go of or the wheel rests.
func (d *deck) bodyCover(c *ui.Context, n *block) {
	if !hasScroller(n) || d.editing == n && d.editPart == editBody {
		return // what is typed in takes its presses and the wheel itself
	}
	now := time.Now()
	termZoom := d.termZoomBlock == n && now.Before(d.termZoomUntil) && c.Modifiers()&ui.Cmd != 0
	gesture := now.Before(d.canvasScrollUntil)
	if d.focusBlock == n.ID && !gesture && !termZoom {
		return
	}
	// A frame to take the cover away once the gesture rests.
	if termZoom {
		c.After(d.termZoomUntil.Sub(now))
	} else if gesture {
		c.After(d.canvasScrollUntil.Sub(now))
	}
	// A browser's bar stays uncovered: only its page takes the wheel.
	var top float32
	if n.Kind == kindBrowser {
		top = browserBar * d.lay.S
	}
	ui.Box(c.Key("cover:"+n.ID)).Absolute().Left(0).Top(top).
		Size(n.W*d.lay.S, n.H*d.lay.S-d.titleHeight(n)-top).
		HandleInput(func(ev ui.InputEvent) bool {
			sx, sy := d.lay.toScreen(n.X, n.Y)
			px, py := d.layerToWindow(sx+ev.X, sy+d.titleHeight(n)+top+ev.Y)
			// The wheel moved as far in the window: the layer's axes are scaled.
			ev.DX, ev.DY = d.layerDelta(ev.DX, ev.DY)
			switch ev.Kind {
			case ui.InputScroll:
				if termZoom {
					d.termZoomUntil = time.Now().Add(termZoomHold)
					d.zoomAt(px, py, wheelZoom(ev.DY))
					d.zoomGesture()
					return true
				}
				ev.X, ev.Y = px, py
				return d.onCanvasInput(ev)
			case ui.InputZoom:
				d.zoomAt(px, py, ev.Scale)
				d.zoomGesture()
				return true
			case ui.InputPointerDown:
				if ev.Button != 0 {
					return false
				}
				// The click that focuses the block: its body takes the
				// clicks and the wheel from now on.
				if ev.Mods&ui.Shift != 0 {
					d.sel[n.ID] = !d.sel[n.ID]
					return true
				}
				d.sel = map[string]bool{}
				d.focusBlock = n.ID
				d.canvasScrollUntil = time.Time{}
				return true
			}
			return false
		})
}

// The sizes of the code and terminal text at 100%: the canvas's zoom
// scales them.
const (
	editorFontSize   = 12
	terminalFontSize = 13
)

func (d *deck) ensureRuntime(n *block) *runtime {
	if n.rt == nil {
		n.rt = newRuntime()
	}
	return n.rt
}

const maxTextBytes = 4 << 20

// loadText reads the file once, off the main thread, and hands the window
// the editor it makes.
func (d *deck) loadText(n *block, rt *runtime) {
	if rt.loading || rt.editor != nil || rt.textErr != nil {
		return
	}
	path := d.path(n)
	read := func() (string, error) {
		f, err := os.Open(path)
		if err != nil {
			return "", err
		}
		defer f.Close()
		buf := make([]byte, maxTextBytes)
		k, err := f.Read(buf)
		if err != nil && k == 0 {
			return "", err
		}
		return string(buf[:k]), nil
	}
	makeState := func(text string) *editor.State {
		return &editor.State{
			Buffer:      editor.NewBuffer(text),
			Font:        ui.Font{Size: editorFontSize * d.lay.S},
			ReadOnly:    true,
			LineNumbers: editor.LineNumbersOn,
		}
	}
	d.wantLoad("text:"+n.ID, n, func() { rt.loading = true }, func() func() {
		text, err := read()
		return func() {
			rt.loading = false
			if err != nil {
				rt.textErr = err
				return
			}
			rt.text = text
			rt.editor = makeState(text)
		}
	})
}

// loadImage decodes the picture once, off the main thread.
func (d *deck) loadImage(n *block, rt *runtime) {
	if rt.imgDone || rt.imgErr != nil {
		return
	}
	path := d.path(n)
	decode := func() (ui.ImageSource, bool, error) {
		data, err := os.ReadFile(path)
		if err != nil {
			return nil, false, err
		}
		src, err := ui.DecodeImage(data)
		return src, mayBeSeeThrough(path, data), err
	}
	d.wantLoad("image:"+n.ID, n, func() { rt.imgDone = true }, func() func() {
		src, alpha, err := decode()
		return func() { rt.imgErr, rt.source, rt.imgAlpha = err, src, alpha }
	})
}

// startTerminal runs the shell in a pseudo-terminal of its own: where the
// block's last shell was, under what its terminal showed (termState).
func (d *deck) startTerminal(n *block, rt *runtime) {
	if rt.starting || rt.term != nil {
		return
	}
	root, st, history := d.root, n.Term, restored(n.Term)
	newTerm := func() (*terminal.Terminal, error) {
		return terminal.New(terminal.Options{
			Dir: termDir(root, st),
			// ⌘ and the wheel over a terminal zoom the canvas, about the
			// block's middle, as they do anywhere else, rather than the
			// terminal's font alone.
			// Typing or pasting into it is using it: what it shows is
			// then worth keeping as the deck closes. ⌘ and a key, a
			// shortcut such as copying or zooming, is not.
			OnKey: func(mods ui.Modifiers, key ui.Key) bool {
				if mods&ui.Cmd == 0 {
					rt.termUsed = true
				}
				return false
			},
			OnPaste: func(transfer.Data) bool {
				rt.termUsed = true
				return false
			},
			OnZoom: func(z terminal.Zoom) bool {
				d.zoomTerminal(n, z)
				return true
			},
			OnTitle: func(title string) {
				if d.win == nil {
					return
				}
				d.win.Update(func() { n.autoTitle(title) })
			},
			OnExit: func(code int) {
				if d.win == nil {
					return
				}
				d.win.Update(func() {
					n.Title = fmt.Sprintf("%s — 已退出 (%d)", n.Title, code)
				})
			},
		})
	}
	if d.win == nil {
		rt.termErr = errNoWindow
		return
	}
	var gen int
	start := func() { rt.starting, gen = true, rt.termGen }
	d.wantLoad("terminal:"+n.ID, n, start, func() func() {
		term, err := newTerm()
		if err == nil && len(history) > 0 {
			// What the terminal showed as the deck closed, before the
			// new shell starts under it.
			term.Feed(history)
		}
		return func() {
			if rt.termGen != gen {
				// Hung up while it started: its block or deck is gone.
				if term != nil {
					term.Close()
				}
				return
			}
			rt.starting = false
			rt.termErr = err
			rt.term = term
		}
	})
}

// zoomTerminal zooms the canvas as a terminal's zoom asks, about the
// middle of its block; a reset frames every block, as ⌘0 does.
func (d *deck) zoomTerminal(n *block, z terminal.Zoom) {
	if z.Reset {
		d.fitAll(d.viewW, d.viewH)
		return
	}
	// A step is 40 DIPs of the wheel, which the canvas zooms by as much:
	// the terminal's first step of a zoom, after which the canvas takes
	// the wheel itself (terminalZoomCatcher).
	sx, sy := d.cam.toScreen(n.X+n.W/2, n.Y+n.H/2)
	d.zoomAt(sx, sy, wheelZoom(-40*float32(z.Steps)))
	d.zoomGesture()
	d.termZoomBlock, d.termZoomUntil = n, time.Now().Add(termZoomHold)
}

// openLink opens a path printed in a terminal in a block of the canvas, or
// with the system for anything else.
func (d *deck) openLink(path string) bool {
	path = strings.TrimSpace(path)
	if path == "" {
		return false
	}
	if !filepath.IsAbs(path) {
		path = filepath.Join(d.root, path)
	}
	if _, err := os.Stat(path); err != nil {
		return false
	}
	rel := d.linkPath(path)
	d.win.Update(func() {
		for _, n := range d.blocks {
			if n.Kind == kindFile && n.Path == rel {
				d.sel = map[string]bool{n.ID: true}
				return
			}
		}
		wx, wy := d.cam.toWorld(640, 400)
		d.addBlock(&block{
			Kind:  kindFile,
			Title: filepath.Base(path),
			Path:  rel,
			X:     wx - 280,
			Y:     wy - 210,
			W:     560,
			H:     420,
		})
	})
	return true
}
