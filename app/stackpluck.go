package main

import (
	"fmt"
	"time"

	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

// A card of a 块堆 is plucked out by its top-right corner, where its kind
// shows: held there for pluckHold, it lifts out of the pile and follows
// the pointer, as large as its block will be on the canvas. Let go over
// the canvas, the block is taken out and lands there; over another stack,
// it goes into that one; over its own, it goes back.
type stackPluck struct {
	from, n *block
	seq     int   // tells one hold's animation from the last's
	down    point // the press, in window DIPs
	at      point // the pointer now
	// grab is the pointer from the card's top-right corner, in window
	// DIPs: the ghost and the block keep it under the pointer.
	grab   point
	fresh  bool // the hold starts this frame
	active bool // held long enough: the card is out of the pile
}

// pluckHold is how long the corner is held before the card comes out;
// pluckSlop how far the pointer may wander meanwhile.
const (
	pluckHold = 450 * time.Millisecond
	pluckSlop = 6
)

// pluckCornerW is how wide, at 100%, the corner of a card's header is
// that a hold plucks it by.
const pluckCornerW = 76

// stackCornerAt is the card whose top-right corner is at (x, y) in st's
// body, laid out with the layout camera, and that corner's top-right in
// the body; nil when none's is.
func (d *deck) stackCornerAt(st *block, x, y float32) (*block, point) {
	_, members := d.stackPicked(st)
	if len(members) == 0 {
		return nil, point{}
	}
	s := d.lay.S
	rt := d.ensureRuntime(st)
	g := d.stackGeomOf(st, len(members))
	right := g.w - g.pad
	if x < right-pluckCornerW*s || x > right+4*s {
		return nil, point{}
	}
	out := -1
	if rt.stackOut && d.stackOpen == st {
		out = rt.stackPick
	}
	// The front card where headers overlap: the last one drawn.
	for i := len(members) - 1; i >= 0; i-- {
		top := g.slot(i, rt.stackScroll)
		switch {
		case out >= 0 && i > out:
			top += g.reveal
		case i == out:
			top -= g.lift
		}
		if y >= top && y <= top+g.peek {
			return members[i], point{right, top}
		}
		if y > top+g.peek {
			return nil, point{} // on a face, under a header
		}
	}
	return nil, point{}
}

// startPluck starts holding n's corner in st, pressed at (px, py) in the
// window; corner is the corner's top-right in st's body.
func (d *deck) startPluck(st, n *block, corner point, px, py float32) {
	cx, cy := d.layerToWindow((st.X-d.lay.X)*d.lay.S+corner.X, (st.Y-d.lay.Y)*d.lay.S+d.titleHeight(st)+corner.Y)
	d.pluckSeq++
	d.pluck = &stackPluck{
		from: st, n: n, seq: d.pluckSeq,
		down: point{px, py}, at: point{px, py},
		grab:  point{px - cx, py - cy},
		fresh: true,
	}
}

// pluckInput takes the pointer while a corner of st is held or its card
// is out: true when it took ev.
func (d *deck) pluckInput(ev ui.InputEvent, st *block, px, py float32) bool {
	p := d.pluck
	if p == nil || p.from != st {
		return false
	}
	switch ev.Kind {
	case ui.InputPointerMove:
		p.at = point{px, py}
		if !p.active && (abs(px-p.down.X) > pluckSlop || abs(py-p.down.Y) > pluckSlop) {
			d.pluck = nil // moved too soon: no hold
		}
		return true
	case ui.InputPointerUp:
		d.pluck = nil
		if p.active {
			d.dropPluck(p, px, py)
		}
		return true
	}
	return false
}

// dropPluck lets go of a plucked card at (px, py) in the window.
func (d *deck) dropPluck(p *stackPluck, px, py float32) {
	if d.indexOf(p.from) < 0 || !containsID(p.from.Items, p.n.ID) {
		return // the pile or the card went meanwhile
	}
	switch st := d.stackAt(px, py, nil); {
	case st == p.from:
		// Back where it was.
	case st != nil:
		d.stackAdd(st, p.n)
	default:
		r := d.pluckGhost(p)
		wx, wy := d.cam.toWorld(r.X, r.Y)
		d.stackTake(p.from, p.n, wx, wy)
	}
}

// pluckGhost is where the card out shows in the window: as large as its
// block on the canvas, its top-right corner held where it was grabbed.
func (d *deck) pluckGhost(p *stackPluck) ui.Rect {
	w, h := p.n.W*d.cam.S, p.n.H*d.cam.S
	return ui.Rect{X: p.at.X - p.grab.X - w, Y: p.at.Y - p.grab.Y, W: w, H: h}
}

// pluckHeld is how far the hold on a corner of st has gone, 0 to 1, and
// the card held; it brings the card out once the hold is complete. box
// is st's body, where the hold animates.
func (d *deck) pluckHeld(c *ui.Context, box ui.Element, st *block) (float32, *block) {
	p := d.pluck
	if p == nil || p.from != st {
		return 0, nil
	}
	if p.active {
		return 1, p.n
	}
	// Just pressed, it starts at 0: the value starts at its first target.
	target := float32(1)
	if p.fresh {
		target, p.fresh = 0, false
		c.Invalidate()
	}
	k := box.AnimateTo(fmt.Sprint("hold:", p.seq), target, ui.Animation{Duration: pluckHold, Ease: ui.Linear})
	if k >= 1 {
		p.active = true
		d.ensureRuntime(st).stackOut = false // the cards close up behind it
		c.Invalidate()
	}
	return k, p.n
}

// plucking says whether n is out of its pile, following the pointer.
func (d *deck) plucking(n *block) bool {
	return d.pluck != nil && d.pluck.active && d.pluck.n == n
}

// pluckView draws the card out of its pile under the pointer, in the
// window over the canvas.
func (d *deck) pluckView(c *ui.Context) {
	p := d.pluck
	if p == nil || !p.active {
		return
	}
	if d.indexOf(p.from) < 0 || !containsID(p.from.Items, p.n.ID) {
		d.pluck = nil
		return
	}
	t := c.Theme()
	r := d.pluckGhost(p)
	s := d.cam.S
	n := p.n
	title := n.Title
	if title == "" {
		title = defaultTitle(n)
	}
	md := d.stackFaceMarkdown(n)
	face := ""
	if md == "" {
		face = d.stackFace(n)
	}
	target := d.stackAt(p.at.X, p.at.Y, nil)
	hint := "松手放到这里"
	switch {
	case target == p.from:
		hint = "松手放回"
	case target != nil:
		hint = "松手收进这一堆"
	}
	shade := ui.RGBA(0, 0, 0, 0.22)
	if t.Dark {
		shade = ui.RGBA(0, 0, 0, 0.55)
	}
	ghost := ui.Box(c.Key("stack-pluck")).Absolute().Left(r.X).Top(r.Y).Size(r.W, r.H).IgnoreInput(true)
	// It comes up out of the pile: a little larger, then its size.
	lift := ghost.AnimateTo("lift", 1, ui.Animation{Spring: ui.SpringOptions{Duration: 300 * time.Millisecond, Bounce: 0.25}})
	ghost.TransformOrigin(1, 0).Scale(0.94+0.06*lift, 0.94+0.06*lift).Opacity(0.94).
		Draw(func(pt *ui.Painter, r ui.Rect) {
			tint := stackTint(n.Kind)
			radius := 10 * s
			pt.Shadow(r, radius, 0, 10*s, 28*s, 0, shade)
			pt.Fill(r, t.Surface.Mix(tint, 0.1), radius)
			pt.Stroke(r, t.Border.Mix(tint, 0.5), radius, max(1*s, 0.5))
			head := titleHeightAt1 * s
			pt.Clip(r, radius, func() {
				pt.Fill(ui.Rect{X: r.X, Y: r.Y, W: r.W, H: 3 * s}, tint, 0)
				label := ui.Span{Text: kindLabel(n.Kind), Size: 10 * s, Color: t.TextMuted}
				lw, _ := pt.MeasureText(0, label)
				right := r.X + r.W - 12*s
				pt.RichText(right-lw, r.Y+head/2-6.5*s, 0, label)
				pt.Clip(ui.Rect{X: r.X, Y: r.Y, W: max(right-lw-8*s-r.X, 0), H: head}, 0, func() {
					pt.RichText(r.X+12*s, r.Y+head/2-8.5*s, 0,
						ui.Span{Text: title, Size: 12.5 * s, Weight: 600, Color: t.Text})
				})
				pt.Line(r.X+12*s, r.Y+head, r.X+r.W-12*s, r.Y+head, max(1*s, 0.5), t.Border.Alpha(0.6))
				if face != "" {
					pt.RichText(r.X+14*s, r.Y+head+8*s, r.W-28*s,
						ui.Span{Text: face, Size: 11 * s, Color: t.TextMuted})
				}
			})
		}).
		Children(func() {
			if md == "" {
				return
			}
			// Its Markdown rendered, as on its face in the pile.
			head := titleHeightAt1 * s
			ui.Box(c).Absolute().Left(14*s).Top(head+1*s).Size(max(r.W-28*s, 0), max(r.H-head-8*s, 0)).
				Clip().Children(func() {
				ui.Column(c).Width(max(r.W-28*s, 0)).Padding(7*s, 0, 0, 0).Children(func() {
					markdown.View(c, md, d.faceMarkdown(t, 11*s, t.Dark))
				})
			})
		})
	// What letting go does, by the pointer.
	ui.Box(c.Key("stack-pluck-hint")).Absolute().Left(p.at.X+14).Top(p.at.Y+18).IgnoreInput(true).
		Padding(4, 9).Radius(10).Background(t.Surface.Alpha(0.96)).Border(1, t.Border).
		Shadow(0, 1, 6, 0, shade).Children(func() {
		ui.Text(c, hint).FontSize(11).SingleLine().TextColor(t.Accent)
	})
}
