package main

import (
	"hash/fnv"
	"math"

	"github.com/egoist/mygo/ui"
)

// tape is a strip of washi tape on a block's back, sticking out above its
// top edge as if it held the block to the canvas, seen in window DIPs.
type tape struct {
	cx, cy float32 // its middle
	l, t   float32 // its length and thickness
	sin    float32 // of its tilt
	cos    float32
}

// tapeOf returns the tape of a block, centered across its top edge and
// tilted a few degrees one way or the other, the same each frame for the
// same block.
func (d *deck) tapeOf(n *block) tape {
	s := d.lay.S
	sx, sy := d.lay.toScreen(n.X, n.Y)
	w := n.W * s
	l := min(96*s, w*0.6)
	th := 30 * s
	h := fnv.New32a()
	h.Write([]byte(n.ID))
	deg := float64(h.Sum32()%9) - 4 // -4°…4°
	a := deg * math.Pi / 180
	return tape{
		cx: sx + w/2, cy: sy - th*0.3,
		l: l, t: th,
		sin: float32(math.Sin(a)), cos: float32(math.Cos(a)),
	}
}

// at returns the point (u, v) of the tape's own axes in window DIPs: u
// along it from its middle, v across it.
func (tp tape) at(u, v float32) (float32, float32) {
	return tp.cx + u*tp.cos - v*tp.sin, tp.cy + u*tp.sin + v*tp.cos
}

// box is the tape's bounds in window DIPs.
func (tp tape) box() ui.Rect {
	x0, y0 := float32(math.Inf(1)), float32(math.Inf(1))
	x1, y1 := float32(math.Inf(-1)), float32(math.Inf(-1))
	for _, c := range [4][2]float32{{-1, -1}, {1, -1}, {1, 1}, {-1, 1}} {
		x, y := tp.at(c[0]*tp.l/2, c[1]*tp.t/2)
		x0, y0, x1, y1 = min(x0, x), min(y0, y), max(x1, x), max(y1, y)
	}
	return ui.Rect{X: x0, Y: y0, W: x1 - x0, H: y1 - y0}
}

// path outlines the tape, its ends torn in a zigzag, moved by (dx, dy).
func (tp tape) path(dx, dy float32) *ui.Path {
	const teeth = 5
	var p ui.Path
	hl, ht := tp.l/2, tp.t/2
	tooth := tp.t * 0.09
	move := func(first bool, u, v float32) {
		x, y := tp.at(u, v)
		if first {
			p.MoveTo(x+dx, y+dy)
		} else {
			p.LineTo(x+dx, y+dy)
		}
	}
	move(true, -hl, -ht)
	move(false, hl, -ht)
	for i := 1; i <= teeth*2; i++ { // the right end, down
		u := hl
		if i%2 == 1 {
			u -= tooth
		}
		move(false, u, -ht+tp.t*float32(i)/(teeth*2))
	}
	move(false, -hl, ht)
	for i := 1; i < teeth*2; i++ { // the left end, up
		u := -hl
		if i%2 == 1 {
			u += tooth
		}
		move(false, u, ht-tp.t*float32(i)/(teeth*2))
	}
	p.Close()
	return &p
}

// tapeView builds a block's tape, behind the block: built before it, so
// that the block covers the tape's lower part and only what sticks out
// above shows. Dragging the tape moves the block.
func (d *deck) tapeView(c *ui.Context, n *block) {
	tp := d.tapeOf(n)
	if tp.l < 24 {
		return
	}
	b := tp.box()
	el := ui.Box(c.Key("tape:"+n.ID)).Absolute().Left(b.X).Top(b.Y).Size(b.W, b.H)
	hovered := el.Hovered() || d.dragging == n
	held := d.dragging == n
	cursor := ui.CursorGrab
	if held {
		cursor = ui.CursorGrabbing
	}
	t := c.Theme()
	selected := d.sel[n.ID]
	el.Cursor(cursor).Tooltip("点击选中，拖动以移动").
		Draw(func(p *ui.Painter, r ui.Rect) {
			drawTape(p, tp, r.X-b.X, r.Y-b.Y, tapeLook{hovered, held, selected, t.Dark, t.Accent})
		}).
		HandleInput(func(ev ui.InputEvent) bool { return d.onTapeInput(ev, n) })
	if o := d.noteOpacity(n); o < 1 {
		el.Opacity(o) // it fades with its QuickLook block
	}
}

// tapeLook is the state a tape is drawn in.
type tapeLook struct {
	hovered, held, selected, dark bool
	accent                        ui.Color
}

// drawTape paints the tape, moved by (dx, dy) into the painter's space:
// clear, frosted white tape, the canvas showing through it, its edges
// catching the light, and a faint shadow that lifts as it is held. The
// tape of a selected block takes the accent: outlined in it, faintly
// tinted.
func drawTape(p *ui.Painter, tp tape, dx, dy float32, look tapeLook) {
	hovered, held, dark := look.hovered, look.held, look.dark
	lift := tp.t * 0.04
	if held {
		lift = tp.t * 0.12
	}
	shadow := ui.RGBA(0, 0, 0, 0.07)
	if dark {
		shadow = ui.RGBA(0, 0, 0, 0.3)
	}
	p.FillPath(tp.path(dx+lift*0.4, dy+lift), shadow)

	// White, mostly see-through: frosted on a light canvas, a pale veil on
	// a dark one; its outline keeps it from vanishing on white.
	fill, edge, shine := float32(0.62), ui.RGBA(0, 0, 0, 0.16), float32(0.75)
	if dark {
		fill, edge, shine = 0.2, ui.RGBA(255, 255, 255, 0.28), 0.3
	}
	if hovered {
		fill += 0.12
	}
	paper := ui.RGBA(255, 255, 255, fill)
	width := max(0.75, tp.t*0.03)
	if look.selected {
		paper = ui.RGBA(255, 255, 255, 1).Mix(look.accent, 0.18).Alpha(min(1, fill+0.1))
		edge, width = look.accent, max(1.5, tp.t*0.06)
	}
	p.FillPath(tp.path(dx, dy), paper)
	p.StrokePath(tp.path(dx, dy), width, edge)

	// The gloss: a thin bright streak near its upper edge.
	gloss := tp
	gloss.l, gloss.t = tp.l*0.9, tp.t*0.1
	gloss.cx, gloss.cy = tp.at(0, -tp.t*0.28)
	p.FillPath(gloss.path(dx, dy), ui.RGBA(255, 255, 255, shine))
}

// onTapeInput selects the block by its tape, and moves it.
func (d *deck) onTapeInput(ev ui.InputEvent, n *block) bool {
	b := d.tapeOf(n).box()
	px, py := d.layerToWindow(b.X+ev.X, b.Y+ev.Y)
	// The wheel moved as far in the window: the layer's axes are scaled.
	ev.DX, ev.DY = d.layerDelta(ev.DX, ev.DY)
	switch ev.Kind {
	case ui.InputScroll, ui.InputZoom:
		ev.X, ev.Y = px, py
		return d.onCanvasInput(ev)
	case ui.InputPointerDown:
		if ev.Button != 0 {
			return false
		}
		// A press on the tape brings the block to the top and selects
		// it, as Shift+click adds it to the selection or takes it out; a
		// drag then carries it.
		d.raise(n)
		switch {
		case ev.Mods&ui.Shift != 0:
			d.sel[n.ID] = !d.sel[n.ID]
		case !d.sel[n.ID]:
			d.sel = map[string]bool{n.ID: true}
		}
		d.beginDrag(n, px, py)
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
		}
		return true
	}
	return false
}
