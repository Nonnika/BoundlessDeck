package main

import (
	"math"

	"github.com/egoist/mygo/ui"
)

// The home screen stands on one canvas seen in 3D: a dotted plane that
// lies back from the window's bottom edge and fades out in the distance,
// as the canvas runs on without end. Blocks lie about on it, the near ones
// out of focus and the far ones sharp. It is all painted and takes no
// input: the home screen's controls are built over it.

// sceneMinWidth is the narrowest window the blocks are shown in: below it
// they would crowd the home screen's column.
const sceneMinWidth = 960

// The plane leans back planeLean degrees, seen from planeDepth times the
// window's height. Its near edge lies planeNear DIPs below the window's
// bottom, and it reaches back far enough that its far edge sits
// planeFarY DIPs below the window's top, where the mist hides it.
const (
	planeLean  = 40
	planeDepth = 1.8
	planeNear  = 80
	planeFarY  = 30
	// planeWidth is the plane's width, in window widths: wide enough that
	// its near corners stay out of the window.
	planeWidth = 2.4
)

// plane is where the plane lies for a window cw by ch DIPs: its rectangle
// before the lean, its bottom edge on the window's (cw/2, oy), and the
// distance it is seen from.
type plane struct {
	rect     ui.Rect
	distance float32
}

func planeFor(cw, ch float32) plane {
	sin, cos := math.Sincos(planeLean * math.Pi / 180)
	d := planeDepth * ch
	oy := ch + planeNear
	// How deep the plane must reach for its far edge to show at
	// planeFarY: depth·cos / (1 + depth·sin/d) = oy - planeFarY.
	rise := float64(oy - planeFarY)
	depth := rise / (cos - rise*sin/float64(d))
	if depth <= 0 || depth > float64(8*ch) {
		depth = float64(8 * ch)
	}
	w := planeWidth * cw
	h := float32(depth)
	return plane{rect: ui.Rect{X: (cw - w) / 2, Y: oy - h, W: w, H: h}, distance: d}
}

// sceneKind is what a block of the scene shows.
type sceneKind uint8

const (
	sceneNote sceneKind = iota
	sceneFile
	sceneTerminal
	sceneImage
)

// sceneBlock is a block lying on the plane: u DIPs right of its middle,
// v DIPs back from its near edge, turned rot degrees on it.
type sceneBlock struct {
	kind sceneKind
	u, v float32
	w, h float32
	rot  float32
}

// sceneBlocks are the blocks on the plane, near to far, kept out of the
// lane down its middle that the home screen's column covers.
var sceneBlocks = []sceneBlock{
	{sceneTerminal, -600, 40, 400, 250, -4},
	{sceneNote, 560, 110, 250, 220, 6},
	{sceneImage, -700, 520, 270, 210, 3},
	{sceneFile, 700, 580, 290, 220, -3},
	{sceneNote, -700, 1000, 210, 190, -5},
	{sceneTerminal, 720, 1080, 320, 200, 2},
	{sceneFile, -820, 1560, 260, 200, 4},
	{sceneNote, 820, 1650, 200, 180, -4},
	{sceneImage, -330, 1820, 230, 170, -2},
	{sceneNote, 380, 1860, 200, 170, 5},
}

// sceneFocus is how far back blocks come into focus, and sceneMaxBlur how
// blurred the nearest are.
const (
	sceneFocus   = 1300
	sceneMaxBlur = 7
)

// sceneBlur is how blurred a block v DIPs back is: the nearer, the more.
func sceneBlur(v float32) float32 {
	if v >= sceneFocus {
		return 0
	}
	f := 1 - v/sceneFocus
	return sceneMaxBlur * f * f
}

// homeScene builds the plane, its blocks, and over them the mist in the
// distance and a clearing under the column at (cx, cy), for a window cw
// by ch.
func homeScene(c *ui.Context, t *ui.Theme, pp paper, cw, ch, cx, cy float32) {
	pl := planeFor(cw, ch)
	lean := ui.RotationX(planeLean).Then(ui.Perspective(pl.distance))
	ui.Box(c).Absolute().Left(pl.rect.X).Top(pl.rect.Y).Size(pl.rect.W, pl.rect.H).
		Transform(lean).TransformOrigin(0.5, 1).IgnoreInput(true).
		Draw(func(p *ui.Painter, r ui.Rect) {
			p.Fill(r, t.Background, 0)
			drawDotsIn(p, r, t.TextMuted.Alpha(0.4))
		}).
		Children(func() {
			if cw < sceneMinWidth {
				return
			}
			for _, b := range sceneBlocks {
				x := pl.rect.W/2 + b.u - b.w/2
				y := pl.rect.H - b.v - b.h
				ui.Box(c).Absolute().Left(x).Top(y).Size(b.w, b.h).Rotate(b.rot).
					Blur(sceneBlur(b.v)).
					Draw(func(p *ui.Painter, r ui.Rect) { drawSceneBlock(p, r, t, pp, b.kind) })
			}
		})

	void := voidColor(t)
	ui.Box(c).Absolute().Left(0).Top(0).Size(cw, ch).IgnoreInput(true).
		Draw(func(p *ui.Painter, r ui.Rect) {
			// The mist the plane fades into, far away.
			mist := ui.Rect{X: r.X, Y: r.Y + planeFarY, W: r.W, H: 70}
			p.FillGradient(mist, ui.LinearGradient{From: void, To: void.Alpha(0), Angle: 180, Oklab: true}, 0)
			p.Fill(ui.Rect{X: r.X, Y: r.Y, W: r.W, H: planeFarY + 2}, void, 0)
			// The clearing under the column.
			p.FillRadialGradient(r, ui.RadialGradient{
				Stops: []ui.GradientStop{
					{Color: t.Background.Alpha(0.9), At: 0},
					{Color: t.Background.Alpha(0.65), At: 0.55},
					{Color: t.Background.Alpha(0), At: 1},
				},
				Center: ui.Point{X: (cx - r.X) / r.W, Y: (cy - r.Y) / r.H},
				Radius: 430,
			}, 0)
		})
}

// voidColor is the color the plane fades into, far away.
func voidColor(t *ui.Theme) ui.Color {
	if t.Dark {
		return ui.Hex("#101114")
	}
	return ui.Hex("#eceef2")
}

func drawSceneBlock(p *ui.Painter, r ui.Rect, t *ui.Theme, pp paper, k sceneKind) {
	switch k {
	case sceneNote:
		drawSceneNote(p, r, t, pp)
	case sceneFile:
		drawSceneFile(p, r, t)
	case sceneTerminal:
		drawSceneTerminal(p, r, t)
	case sceneImage:
		drawSceneImage(p, r, t)
	}
}

// drawDotsIn paints dots over r, on the grid the canvas uses.
func drawDotsIn(p *ui.Painter, r ui.Rect, color ui.Color) {
	const step, dot = 24, 3
	for y := r.Y + step/2; y < r.Y+r.H; y += step {
		for x := r.X + step/2; x < r.X+r.W; x += step {
			p.Fill(ui.Rect{X: x - dot/2, Y: y - dot/2, W: dot, H: dot}, color, dot/2)
		}
	}
}

// tilt paints draw(b) turned by deg degrees around b's middle.
func tilt(p *ui.Painter, b ui.Rect, deg float32, draw func(b ui.Rect)) {
	p.Transform(ui.Rotation(deg), b.X+b.W/2, b.Y+b.H/2, func() { draw(b) })
}

// sceneShadow is the shadow under a scene block.
func sceneShadow(t *ui.Theme) ui.Color {
	if t.Dark {
		return ui.RGBA(0, 0, 0, 0.45)
	}
	return ui.RGBA(0, 0, 0, 0.1)
}

// drawSceneCard paints a block's frame and title, as the canvas shows a
// block (no shadow, its tape behind it), and returns the rectangle of its
// body.
func drawSceneCard(p *ui.Painter, b ui.Rect, t *ui.Theme, title string) ui.Rect {
	const bar = 30
	drawBlockTape(p, b, t, 3)
	p.Fill(b, t.Surface, 0)
	p.Stroke(b, t.Border, 0, 1)
	p.Text(b.X+12, b.Y+(bar-15)/2, title, 12, t.Text)
	p.Fill(ui.Rect{X: b.X, Y: b.Y + bar, W: b.W, H: 1}, t.Border, 0)
	return ui.Rect{X: b.X + 1, Y: b.Y + bar + 1, W: b.W - 2, H: b.H - bar - 2}
}

func drawSceneNote(p *ui.Painter, b ui.Rect, t *ui.Theme, pp paper) {
	p.Shadow(b, 0, 0, 3, 10, 0, pp.shadow)
	p.FillGradient(b, ui.LinearGradient{From: pp.top, To: pp.bottom, Angle: 180}, 0)
	p.Text(b.X+16, b.Y+22, "待办", 15, pp.text)
	lines := []string{"☑ 摊开项目文件", "☐ 起一个终端", "☐ 把想法贴上来"}
	for i, l := range lines {
		p.Text(b.X+16, b.Y+54+float32(i)*24, l, 12, pp.muted)
	}
	tp := tape{cx: b.X + b.W/2, cy: b.Y + 2, l: 74, t: 22, sin: 0.05, cos: 0.9987}
	drawTape(p, tp, 0, 0, tapeLook{dark: t.Dark, accent: t.Accent})
}

func drawSceneFile(p *ui.Painter, b ui.Rect, t *ui.Theme) {
	body := drawSceneCard(p, b, t, "README.md")
	x, y := body.X+14, body.Y+14
	p.Text(x, y, "# BoundlessDeck", 13, t.Text)
	widths := []float32{0.82, 0.64, 0.74, 0.4}
	for i, w := range widths {
		p.Fill(ui.Rect{X: x, Y: y + 32 + float32(i)*16, W: (body.W - 28) * w, H: 6}, t.TextMuted.Alpha(0.25), 3)
	}
}

func drawSceneTerminal(p *ui.Painter, b ui.Rect, t *ui.Theme) {
	body := drawSceneCard(p, b, t, "终端")
	bg := ui.Hex("#1d1f24")
	p.Fill(body, bg, 0)
	green, text, dim := ui.Hex("#7ee787"), ui.Hex("#e6e6e6"), ui.Hex("#8b919a")
	x, y := body.X+14, body.Y+14
	p.Text(x, y, "~/deck", 12, dim)
	p.Text(x, y+22, "$", 12, green)
	p.Text(x+14, y+22, "go tool mygo dev", 12, text)
	p.Text(x, y+44, "watching for changes…", 12, dim)
	p.Text(x, y+66, "$", 12, green)
	p.Fill(ui.Rect{X: x + 14, Y: y + 67, W: 7, H: 14}, text.Alpha(0.8), 1)
}

func drawSceneImage(p *ui.Painter, b ui.Rect, t *ui.Theme) {
	body := drawSceneCard(p, b, t, "sketch.png")
	p.Clip(body, 0, func() {
		p.FillGradient(body, ui.LinearGradient{From: ui.Hex("#9ec5ff"), To: ui.Hex("#e8f1ff"), Angle: 180}, 0)
		// Hills and a sun.
		p.Fill(ui.Rect{X: body.X + body.W - 56, Y: body.Y + 18, W: 26, H: 26}, ui.Hex("#ffd166"), 13)
		p.Fill(ui.Rect{X: body.X - 30, Y: body.Y + body.H - 46, W: body.W * 0.8, H: 120}, ui.Hex("#6fbf73"), 60)
		p.Fill(ui.Rect{X: body.X + body.W*0.4, Y: body.Y + body.H - 34, W: body.W * 0.9, H: 120}, ui.Hex("#4fa65a"), 60)
	})
}

// drawDeckMark paints a deck: three sheets stacked, each turned a little,
// the top one a sticky note held by tape. It fits r, a square.
func drawDeckMark(p *ui.Painter, r ui.Rect, t *ui.Theme, pp paper, withTape bool) {
	u := r.W / 34 // the mark is drawn on a 34-unit square
	sheet := ui.Rect{X: r.X + 7*u, Y: r.Y + 8*u, W: 21 * u, H: 20 * u}
	back := t.Surface
	if !t.Dark {
		back = ui.Hex("#ffffff")
	}
	shadow := sceneShadow(t)
	// The sheets beneath, fanned out to the left.
	for i, deg := range []float32{-16, -8} {
		under := sheet
		under.X -= float32(2-i) * 2.5 * u
		under.Y += float32(2-i) * 0.8 * u
		tilt(p, under, deg, func(b ui.Rect) {
			p.Shadow(b, 0, 0, 0.5*u, 2*u, 0, shadow)
			p.Fill(b, back, 0)
			p.Stroke(b, t.TextMuted.Alpha(0.35), 0, max(1, u*0.5))
		})
	}
	tilt(p, sheet, 4, func(b ui.Rect) {
		p.Shadow(b, 0, 0, u, 3*u, 0, pp.shadow)
		p.FillGradient(b, ui.LinearGradient{From: pp.top, To: pp.bottom, Angle: 180}, 0)
		if withTape {
			tp := tape{cx: b.X + b.W/2, cy: b.Y + u, l: 14 * u, t: 6 * u, sin: -0.07, cos: 0.9975}
			drawTape(p, tp, 0, 0, tapeLook{dark: t.Dark, accent: t.Accent})
		}
	})
}

// drawBlockTape paints the tape on the back of a block at b, tilted deg
// degrees, as the canvas places it (tapeOf at 100%): across the middle of
// its top edge, sticking out above. Paint the block after it, over it, so
// that only what sticks out shows.
func drawBlockTape(p *ui.Painter, b ui.Rect, t *ui.Theme, deg float64) {
	const th = 30
	a := deg * math.Pi / 180
	tp := tape{
		cx: b.X + b.W/2, cy: b.Y - th*0.3,
		l: min(96, b.W*0.6), t: th,
		sin: float32(math.Sin(a)), cos: float32(math.Cos(a)),
	}
	drawTape(p, tp, 0, 0, tapeLook{dark: t.Dark, accent: t.Accent})
}
