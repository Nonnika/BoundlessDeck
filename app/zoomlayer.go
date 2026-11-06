package main

import (
	"math"
	"time"

	"github.com/egoist/mygo/ui"
)

// While a pinch or ⌘ and the wheel zoom the canvas, nothing is laid out
// or drawn anew: the blocks keep the layout they had as the zoom began, at
// the scale of the camera they were laid out with (deck.lay), and their
// layer is moved and scaled to the canvas's camera on the GPU. Once the
// zoom rests for zoomSettleHold, the blocks are laid out once at the zoom,
// crisp. A zoom renders what was before it and what is after it, and
// nothing between.
//
// Meanwhile the layer is a bitmap (Element.Cache): what it holds stays as
// it was, the camera's moves are the layer's transform alone, so frames
// draw that one bitmap moved and scaled. Drawn as it is under a scale,
// every clip inside it (each card's, each editor's, the terminal's…)
// would be a mask MyGo rasterizes on the CPU, each frame anew as the
// scale changes: some 100 ms a frame with a terminal filling the window.
//
// Inside the layer, sizes and positions are the layout camera's, and so
// are the coordinates the blocks' handlers get; layerToWindow takes them
// to the window's, where the canvas and the world math work, and
// layerDelta the wheel's deltas.
// zoomSettleHold is how long a zoom rests before it ends; tests that look
// mid-gesture hold it longer (holdZoom), whatever their pace.
var zoomSettleHold = 150 * time.Millisecond

// At rest the canvas is on a zoom level: zoomLevels of them to each
// doubling, 1 among them. Laid out at a level, the blocks' text takes
// sizes it took before, whose glyphs the text system keeps: laid out at
// whatever scale a zoom ended at, every glyph in view would be rasterized
// anew once it rests (tens of ms on a full window), and every size would
// make fonts of its own, which MyGo frees all at once past 512
// (textfonts.go). A zoom that rests between levels glides to the nearest
// over zoomSnapTime, the layer scaled on the GPU meanwhile, as mid-gesture.
const (
	zoomLevels   = 12
	zoomSnapTime = 120 * time.Millisecond
	// zoomKeySteps is how many levels a key (⌘= and ⌘-) zooms by: ×1.19.
	zoomKeySteps = 3
	// zoomRebase is how far a zoom goes out of its layout before it is
	// laid out anew mid-gesture.
	zoomRebase = 0.5
)

// zoomSnap is a zoom's glide to a zoom level, once it rests.
type zoomSnap struct {
	on       bool
	from, to float32
	at       time.Time
}

// zoomLevel is the zoom level nearest s, within the zoom's limits.
func zoomLevel(s float32) float32 {
	return zoomLevelOf(math.Round(math.Log2(float64(s)) * zoomLevels))
}

// zoomLevelAtMost is the largest zoom level not above s, within the
// zoom's limits.
func zoomLevelAtMost(s float32) float32 {
	return zoomLevelOf(math.Floor(math.Log2(float64(s))*zoomLevels + 1e-6))
}

// zoomLevelOf is level n, the n-th 12th of a doubling from 1, taken into
// the zoom's limits.
func zoomLevelOf(n float64) float32 {
	lo := math.Ceil(math.Log2(minScale) * zoomLevels)
	hi := math.Floor(math.Log2(maxScale) * zoomLevels)
	return float32(math.Exp2(min(max(n, lo), hi) / zoomLevels))
}

// onLevel says whether s is a zoom level.
func onLevel(s float32) bool { return abs(s-zoomLevel(s)) <= s*1e-5 }

// zoomStep zooms by steps of zoomKeySteps levels about (sx, sy), as the
// keys do: from a level to a level.
func (d *deck) zoomStep(sx, sy float32, steps int) {
	n := math.Round(math.Log2(float64(d.cam.S))*zoomLevels) + float64(steps*zoomKeySteps)
	d.zoomAt(sx, sy, zoomLevelOf(n)/d.cam.S)
	d.cam.S = zoomLevelOf(n) // exactly, whatever the float math made of it
}

// zoomTo sets the camera's scale to s about the last zoom's anchor.
func (d *deck) zoomTo(s float32) {
	d.zoomAt(d.zoomAnchor.X, d.zoomAnchor.Y, s/d.cam.S)
	d.cam.S = s
}

// settleLayer picks the camera the blocks are laid out with this frame:
// the zoom's at rest, the one they had mid-gesture, and the level a zoom
// that rested glides to.
func (d *deck) settleLayer(c *ui.Context) {
	now := time.Now()
	if d.lay.S == 0 {
		d.lay = d.cam
		return
	}
	if now.Before(d.zoomSettle) {
		// Mid-gesture: the layout stays as it began, the layer's transform
		// moves and scales it to the camera. Zoomed out below half of it,
		// the layer would take in the blocks of a view many times the
		// window's, past what a bitmap holds (8192 px across): it is laid
		// out anew at the zoom's level, once, and the gesture goes on.
		d.snap.on = false
		if d.cam.S < d.lay.S*zoomRebase {
			d.lay = camera{X: d.cam.X, Y: d.cam.Y, S: zoomLevel(d.cam.S)}
			perff("zoomed out past the layout: laid out anew at %.3f", d.lay.S)
		}
		c.After(d.zoomSettle.Sub(now))
		return
	}
	if !d.snap.on && d.lay.S != d.cam.S && !onLevel(d.cam.S) {
		// A zoom rested between levels: to the nearest, at once without
		// a window (tests), else gliding.
		to := zoomLevel(d.cam.S)
		if d.win == nil {
			d.zoomTo(to)
		} else {
			d.snap = zoomSnap{on: true, from: d.cam.S, to: to, at: now}
			// Laid out once at the level it ends on, then moved and scaled
			// to the glide as mid-gesture.
			d.lay = camera{X: d.cam.X, Y: d.cam.Y, S: to}
			perff("zoom rests at %.3f: gliding to the level %.3f", d.cam.S, to)
		}
	}
	if d.snap.on {
		t := float32(now.Sub(d.snap.at)) / float32(zoomSnapTime)
		if t < 1 {
			e := 1 - (1-t)*(1-t) // easing out
			d.zoomTo(d.snap.from + (d.snap.to-d.snap.from)*e)
			c.After(8 * time.Millisecond)
			return
		}
		d.zoomTo(d.snap.to)
		d.snap.on = false
		perff("glide ends at %.3f", d.cam.S)
	}
	d.lay = d.cam // at rest: at the zoom, crisp
}

// layerScale is how much the blocks' layer is scaled to show at the
// canvas's zoom: 1 once the zoom rests.
func (d *deck) layerScale() float32 {
	if d.lay.S == 0 {
		return 1
	}
	return d.cam.S / d.lay.S
}

// layerOffset is how far the blocks' layer is moved in the window, in
// DIPs: as far as the camera moved since the layout's.
func (d *deck) layerOffset() (float32, float32) {
	if d.lay.S == 0 {
		return 0, 0
	}
	return (d.lay.X - d.cam.X) * d.cam.S, (d.lay.Y - d.cam.Y) * d.cam.S
}

// layerMoved says whether the blocks' layer is moved or scaled: while a
// zoom goes on or glides to its level.
func (d *deck) layerMoved() bool {
	x, y := d.layerOffset()
	return d.layerScale() != 1 || x != 0 || y != 0
}

// layerToWindow takes a point of the blocks' layer, as the layout camera
// places it, to the window, in DIPs.
func (d *deck) layerToWindow(x, y float32) (float32, float32) {
	k := d.layerScale()
	ox, oy := d.layerOffset()
	return x*k + ox, y*k + oy
}

// layerDelta takes a distance in the blocks' layer, as the wheel's, to
// the window's.
func (d *deck) layerDelta(dx, dy float32) (float32, float32) {
	k := d.layerScale()
	return dx * k, dy * k
}

// layerCached says whether the blocks' layer, moved, can show as a bitmap
// this frame: unless a browser in view shows its live page, which a
// bitmap does not hold, as it does while its picture is being taken.
func (d *deck) layerCached(built []*block) bool {
	for _, n := range built {
		if n.Kind == kindBrowser && n.rt != nil && n.rt.web != nil && n.rt.webSnap == nil && n.rt.webSnapOld == nil && d.lay.S >= lodScale {
			return false
		}
	}
	return true
}

// zoomGesture marks the zoom as one that goes on: the blocks are laid out
// anew only once it rests.
func (d *deck) zoomGesture() { d.zoomSettle = time.Now().Add(zoomSettleHold) }
