package main

import "github.com/egoist/mygo/ui"

// The least a block can be resized to, in world units.
const (
	minBlockW = 140
	minBlockH = 90
)

// resizeSize is the side of a block's resize handle on the screen: about
// the folded corner of a QuickLook block, which is its handle.
func (d *deck) resizeSize() float32 {
	return max(14, 22*d.lay.S)
}

// resizeBox is the handle's box in window DIPs, in the block's
// bottom-right corner.
func (d *deck) resizeBox(n *block) ui.Rect {
	size := d.resizeSize()
	sx, sy := d.lay.toScreen(n.X, n.Y)
	return ui.Rect{X: sx + n.W*d.lay.S - size, Y: sy + n.H*d.lay.S - size, W: size, H: size}
}

// resizeView builds the handle that resizes a block from its bottom-right
// corner, keeping its proportions. A QuickLook block's folded corner is
// the handle as it is; other blocks show a grip of diagonal lines.
func (d *deck) resizeView(c *ui.Context, n *block) {
	size := d.resizeSize()
	if n.W*d.lay.S < size*3 || n.H*d.lay.S < size*3 {
		return
	}
	b := d.resizeBox(n)
	h := ui.Box(c.Key("resize:"+n.ID)).Absolute().Left(b.X).Top(b.Y).Size(b.W, b.H).
		Cursor(ui.CursorResizeNWSE).Tooltip("拖动以等比缩放")
	active := h.Hovered() || d.resizing == n
	if n.Kind != kindNote {
		t := c.Theme()
		color := t.TextMuted.Alpha(0.45)
		if active {
			color = t.TextMuted
		}
		h.Draw(func(p *ui.Painter, r ui.Rect) { drawGrip(p, r, color) })
	}
	h.HandleInput(func(ev ui.InputEvent) bool { return d.onResizeInput(ev, n) })
}

// drawGrip paints three diagonal strokes in the corner, as a window's
// resize grip.
func drawGrip(p *ui.Painter, r ui.Rect, color ui.Color) {
	x1, y1 := r.X+r.W-r.W*0.2, r.Y+r.H-r.H*0.2
	w := max(1, r.W*0.07)
	for i := 1; i <= 3; i++ {
		f := float32(i) * r.W * 0.22
		var line ui.Path
		line.MoveTo(x1-f, y1).LineTo(x1, y1-f).Cap(ui.RoundCap)
		p.StrokePath(&line, w, color)
	}
}

// onResizeInput scales the block with the pointer, from its top-left
// corner, keeping its proportions.
func (d *deck) onResizeInput(ev ui.InputEvent, n *block) bool {
	b := d.resizeBox(n)
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
		d.resizing = n
		wx, wy := d.cam.toWorld(px, py)
		d.resizeFrom = point{wx, wy}
		d.resizeStart = point{n.W, n.H}
		return true
	case ui.InputPointerMove:
		if d.resizing != n {
			return false
		}
		wx, wy := d.cam.toWorld(px, py)
		n.W, n.H = scaleKeepingRatio(d.resizeStart.X, d.resizeStart.Y,
			wx-d.resizeFrom.X, wy-d.resizeFrom.Y)
		return true
	case ui.InputPointerUp:
		if d.resizing == n {
			d.resizing = nil
		}
		return true
	}
	return false
}

// scaleKeepingRatio returns w×h scaled by how far the corner was dragged,
// (dx, dy), along the block's diagonal: the drag's projection on it, so
// that the corner follows the pointer as nearly as the proportions let it.
// It goes no smaller than minBlockW×minBlockH lets, in proportion.
func scaleKeepingRatio(w, h, dx, dy float32) (float32, float32) {
	k := ((w+dx)*w + (h+dy)*h) / (w*w + h*h)
	k = max(k, minBlockW/w, minBlockH/h)
	return w * k, h * k
}
