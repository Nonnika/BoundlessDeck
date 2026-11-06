package main

import (
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
	"path/filepath"
	"strings"
)

// paper is the palette of a sticky note, light or dark.
type paper struct {
	top, bottom ui.Color // the sheet, a touch darker toward the bottom
	header      ui.Color // the band the title sits on
	flap        ui.Color // the underside of the folded corner
	text, muted ui.Color
	link        ui.Color
	code        ui.Color // behind code spans and blocks
	shadow      ui.Color
}

func paperFor(t *ui.Theme) paper {
	if t.Dark {
		return paper{
			top:    ui.Hex("#5e5220"),
			bottom: ui.Hex("#544919"),
			header: ui.Hex("#4b4115"),
			flap:   ui.Hex("#6f6229"),
			text:   ui.Hex("#f7ecbf"),
			muted:  ui.Hex("#cdbd80"),
			link:   ui.Hex("#ffd27a"),
			code:   ui.RGBA(0, 0, 0, 0.25),
			shadow: ui.RGBA(0, 0, 0, 0.45),
		}
	}
	return paper{
		top:    ui.Hex("#fff5a8"),
		bottom: ui.Hex("#fdeb85"),
		header: ui.Hex("#f8e06c"),
		flap:   ui.Hex("#eacd4f"),
		text:   ui.Hex("#4a3b10"),
		muted:  ui.Hex("#8a7332"),
		link:   ui.Hex("#9a5b00"),
		code:   ui.RGBA(120, 90, 0, 0.12),
		shadow: ui.RGBA(120, 90, 0, 0.22),
	}
}

// editPart is what of a QuickLook block is being edited.
type editPart uint8

const (
	editBody editPart = iota
	editTitle
)

// startEditing shows an input in place of the block's title or, of a
// QuickLook block, body.
func (d *deck) startEditing(n *block, part editPart) {
	if d.editing != nil && d.editing != n {
		d.stopEditing()
	}
	d.editing, d.editPart, d.editHad, d.editFrom = n, part, false, n.Title
}

// stopEditing keeps what was typed and shows the block as it reads again.
func (d *deck) stopEditing() {
	if n := d.editing; n != nil && n.Kind == kindMemo {
		if d.editPart == editBody {
			d.saveMemo(n, n.rt, false)
		} else if n.Title != d.editFrom {
			// The note's title is its file's name.
			d.editing, d.editHad = nil, false
			d.renameMemo(n)
			return
		}
	}
	if n := d.editing; n != nil && d.editPart == editTitle {
		n.Title = strings.TrimSpace(n.Title)
		switch {
		case n.Title == "":
			// Cleared: the block's own title again, which its shell or
			// page may change.
			n.Title, n.Named = defaultTitle(n), false
		case n.Title != d.editFrom:
			n.Named = true
		}
	}
	d.editing, d.editHad = nil, false
}

// defaultTitle is the title a block has until it is named: its file's
// name, or its kind's.
func defaultTitle(n *block) string {
	switch n.Kind {
	case kindFile:
		if n.Path != "" {
			return filepath.Base(n.Path)
		}
		return "文件"
	case kindTerminal:
		return "终端"
	case kindBrowser:
		return "浏览器"
	case kindMemo:
		if n.Path != "" {
			return memoTitleOf(n.Path)
		}
		return memoTitle
	case kindStack:
		return "块堆"
	}
	return "QuickLook"
}

// autoTitle gives the block the title its shell or page says, unless the
// user named it.
func (n *block) autoTitle(title string) {
	if !n.Named && strings.TrimSpace(title) != "" {
		n.Title = title
	}
}

// watchEditFocus ends the edit once its input, having had the focus, loses
// it: a click anywhere else is done editing.
func (d *deck) watchEditFocus(c *ui.Context) {
	d.editSeen = true
	if d.editInput.Focused(c) {
		d.editHad = true
	} else if d.editHad {
		d.stopEditing()
	}
}

// endKeys ends the edit on Escape, and in the body on ⌘↵ too.
func (d *deck) endKeys(k ui.TextKey) bool {
	if k.Key == ui.KeyEscape || (k.Key == ui.KeyEnter && k.Mods == ui.Cmd) {
		if n := d.editing; n != nil && d.editPart == editBody && livePreviewed(n) {
			// Done typing, the preview keeps the focus: the wheel goes
			// on scrolling what was written.
			d.focusBlock = n.ID
		}
		d.stopEditing()
		return true
	}
	return false
}

// markdownFor returns the options the bodies show their Markdown with, in
// the paper's colors at the size the camera gives.
func (d *deck) markdownFor(pp paper, size float32, dark bool) *markdown.Options {
	if d.md.Theme == nil || d.mdSize != size || d.mdDark != dark {
		d.md.Theme = &markdown.Theme{
			FontSize:       size,
			Text:           pp.text,
			Muted:          pp.muted,
			Strong:         pp.text,
			Link:           pp.link,
			Code:           pp.text,
			CodeBackground: pp.code,
			QuoteBar:       pp.flap,
			QuoteText:      pp.muted,
		}
		d.mdSize, d.mdDark = size, dark
	}
	return &d.md
}

// stickyView builds a QuickLook block (kind note) as a sticky note: yellow
// paper lifted off the canvas, a darker band for its title and a folded
// bottom-right corner. A click edits its title or its body; the body reads
// as Markdown.
func (d *deck) stickyView(c *ui.Context, n *block) {
	t := c.Theme()
	pp := paperFor(t)
	s := d.lay.S
	sx, sy := d.lay.toScreen(n.X, n.Y)
	w, h := n.W*s, n.H*s
	fold := 22 * s

	card := ui.Box(c.Key("block:"+n.ID)).Absolute().Left(sx).Top(sy).Size(w, h).
		Shadow(0, 3*min(s, 1.5), 10*min(s, 1.5), 0, pp.shadow)
	if d.sel[n.ID] {
		card = card.Border(2, t.Accent)
	}
	card = card.Clip().FocusBind(&d.focusBlock, cardFocus(n)).
		Draw(func(p *ui.Painter, r ui.Rect) {
			p.FillGradient(r, ui.LinearGradient{From: pp.top, To: pp.bottom, Angle: 180}, 0)
		}).
		DrawOver(func(p *ui.Painter, r ui.Rect) {
			x1, y1 := r.X+r.W, r.Y+r.H
			// The corner folded over: the canvas shows where it was, and
			// the flap lies on the sheet.
			var cut, flap ui.Path
			cut.MoveTo(x1-fold, y1).LineTo(x1, y1-fold).LineTo(x1, y1).Close()
			p.FillPath(&cut, t.Background)
			flap.MoveTo(x1-fold, y1).LineTo(x1, y1-fold).LineTo(x1-fold, y1-fold).Close()
			p.FillPath(&flap, pp.flap)
		}).
		HandleInput(func(ev ui.InputEvent) bool { return d.onBlockInput(ev, n) })
	if o := d.noteOpacity(n); o < 1 {
		card = card.Opacity(o) // its time is nearly up
	}

	shown := d.contentShown(card) // its body comes and goes blurred
	if shown <= 0 {
		card.Draw(func(p *ui.Painter, r ui.Rect) {
			p.Fill(r, pp.top, 0)
			d.drawFarTitle(p, r, n, 10, pp.text)
		})
		return
	}

	titleSize := titleFontSize * s
	bodySize := 14 * s
	editing := d.editing == n
	card.Children(func() {
		ui.Row(c).Height(d.titleHeight(n)).FillWidth().Padding(0, 10*s).AlignItems(ui.Center).
			Background(pp.header).Children(func() {
			if editing && d.editPart == editTitle {
				in := ui.TextInput(c.Key("note-title:"+n.ID), &n.Title).Bind(&d.editInput).AutoFocus().
					Grow(1).FontSize(titleSize).FontWeight(600).TextColor(pp.text).
					Background(ui.Transparent).Border(0, ui.Transparent).Padding(0).
					OnTextKey(d.endKeys)
				if in.Submitted() {
					d.stopEditing()
				}
				d.watchEditFocus(c)
				return
			}
			ui.Text(c, n.Title).FontSize(titleSize).FontWeight(600).SingleLine().Grow(1).
				TextColor(pp.text)
			d.lifeChip(c, n, pp, s)
		})
		body := fadeContent(ui.Box(c).Grow(1).FillWidth().Clip(), shown)
		if editing && d.editPart == editBody {
			body.Padding(6*s, 8*s, fold, 8*s).Children(func() {
				ui.TextArea(c.Key("note-body:"+n.ID), &n.Text).Bind(&d.editInput).AutoFocus().Fill().
					FontSize(bodySize).TextColor(pp.text).PlaceholderColor(pp.muted).
					Placeholder("支持 Markdown · Esc 或 ⌘↵ 完成").
					Background(ui.Transparent).Border(0, ui.Transparent).
					OnTextKey(d.endKeys)
				d.watchEditFocus(c)
			})
			return
		}
		// Reading, it is a live preview: focused, it scrolls, and a click
		// on it edits it.
		body.Children(func() {
			d.livePreview(c, n, [4]float32{12 * s, 14 * s, fold + 4*s, 14 * s}, func() {
				if n.Text == "" {
					ui.Text(c, "点击编辑，支持 Markdown").FontSize(bodySize).TextColor(pp.muted)
					return
				}
				markdown.View(c, n.Text, d.markdownFor(pp, bodySize, t.Dark))
			})
		})
	})
}

// livePreviewed reports whether the block reads in a livePreview.
func livePreviewed(n *block) bool { return n.Kind == kindNote || n.Kind == kindMemo }

// livePreview shows a QuickLook or 笔记 body rendered, as it reads, in a
// scroller of its own. Not focused, it lies under the block's bodyCover: a
// two-finger pan over it moves the canvas, and a click focuses it. Focused,
// the wheel scrolls what it shows, and a click on it edits it. pad is its
// padding, top, right, bottom, left.
func (d *deck) livePreview(c *ui.Context, n *block, pad [4]float32, content func()) {
	rt := d.ensureRuntime(n)
	ui.Scroll(c.Key("live:"+n.ID)).TrackScroll(&rt.liveScroll).Fill().Focusable().FocusBind(&d.focusBlock, n.ID).
		Children(func() {
			ui.Column(c.Key("live-page:"+n.ID)).FillWidth().Padding(pad[0], pad[1], pad[2], pad[3]).
				HandleInput(func(ev ui.InputEvent) bool {
					if ev.Kind == ui.InputScroll {
						// The card and the canvas around it would take the
						// wheel before the scroller could: the page scrolls
						// itself. ⌘ or Ctrl and the wheel zoom the canvas.
						if ev.Mods&(ui.Cmd|ui.Ctrl) != 0 {
							return false
						}
						rt.liveScroll.Y = min(max(rt.liveScroll.Y+ev.DY, 0), rt.liveScroll.MaxY)
						return true
					}
					if ev.Kind != ui.InputPointerDown || ev.Button != 0 {
						return false
					}
					if ev.Mods&ui.Shift != 0 {
						d.sel[n.ID] = !d.sel[n.ID]
						return true
					}
					d.sel = map[string]bool{}
					d.startEditing(n, editBody)
					return true
				}).
				Children(func() {
					// Its text and links leave their presses to the page.
					ui.Column(c).FillWidth().IgnoreInput(true).Children(content)
				})
		})
	if n.Kind == kindNote {
		d.bodyCover(c, n) // a 笔记's card puts it on, as every card's
	}
}
