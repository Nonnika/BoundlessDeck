package main

import (
	"fmt"
	"math"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// A QuickLook block is a passing thought: it goes by itself once its time
// is up, an hour after it was made unless the user gives it another span,
// or keeps it (block.Expires, Unix ms; 0 keeps it). Its header shows the
// time left, and a click on that picks another span, counted from then. In
// its last minutes it fades; then it is removed as a deletion is, so ⌘Z
// brings it back, with a little more time. One whose time ran out while
// the deck was closed is gone when the deck opens.

// noteLife is how long a new QuickLook block stays.
const noteLife = time.Hour

// noteFade is how long before it goes a QuickLook block starts fading, and
// noteFadeTo how faint it is as it goes.
const (
	noteFade   = 10 * time.Minute
	noteFadeTo = 0.4
)

// noteRescue is the time a QuickLook block brought back by an undo after
// it went has left.
const noteRescue = 10 * time.Minute

// lifeChoices are the spans the time-left menu offers; 0 keeps the block.
var lifeChoices = []struct {
	label string
	span  time.Duration
}{
	{"10 分钟", 10 * time.Minute},
	{"30 分钟", 30 * time.Minute},
	{"1 小时", time.Hour},
	{"3 小时", 3 * time.Hour},
	{"1 天", 24 * time.Hour},
	{"1 周", 7 * 24 * time.Hour},
	{"永久保留", 0},
}

// now is the deck's clock: the wall clock, or a test's.
func (d *deck) now() time.Time {
	if d.clock != nil {
		return d.clock()
	}
	return time.Now()
}

// expiresAt is when n goes, and whether it does.
func expiresAt(n *block) (time.Time, bool) {
	if n.Kind != kindNote || n.Expires <= 0 {
		return time.Time{}, false
	}
	return time.UnixMilli(n.Expires), true
}

// setLife gives n span from now, or keeps it for 0.
func (d *deck) setLife(n *block, span time.Duration) {
	if span <= 0 {
		n.Expires = 0
	} else {
		n.Expires = d.now().Add(span).UnixMilli()
	}
	d.requestSave()
}

// expireNotes removes the QuickLook blocks whose time is up, but not the one
// being written in: it goes once the edit ends. It returns how long until
// something of theirs changes on screen (one goes, a time left reads
// otherwise, a fading one fades further), or 0 when nothing will.
func (d *deck) expireNotes() time.Duration {
	now := d.now()
	gone := false
	var wake time.Duration
	soonest := func(w time.Duration) {
		w = max(w, time.Second)
		if wake == 0 || w < wake {
			wake = w
		}
	}
	for i := len(d.blocks) - 1; i >= 0; i-- {
		n := d.blocks[i]
		at, ok := expiresAt(n)
		if !ok {
			continue
		}
		left := at.Sub(now)
		if left <= 0 {
			if d.editing == n {
				continue // stopEditing removes it
			}
			d.removeBlock(n)
			gone = true
			continue
		}
		soonest(left)
		switch {
		case left <= noteFade:
			soonest(3 * time.Second) // it fades
		default:
			// Its time left reads in minutes, hours or days: until the
			// next minute it shows changes.
			soonest(left % time.Minute)
		}
	}
	if gone {
		d.requestSave()
	}
	return wake
}

// dropExpired takes out of a deck just read the QuickLook blocks whose time
// ran out while it was closed. It is not an edit: there is no undo.
func (d *deck) dropExpired() {
	now := d.now()
	keep := d.blocks[:0]
	for _, n := range d.blocks {
		if at, ok := expiresAt(n); ok && !at.After(now) {
			continue
		}
		keep = append(keep, n)
	}
	d.blocks = keep
}

// rescueNote gives a QuickLook block an undo brought back after it went a
// little more time.
func (d *deck) rescueNote(n *block) {
	if at, ok := expiresAt(n); ok && at.Sub(d.now()) < noteRescue {
		n.Expires = d.now().Add(noteRescue).UnixMilli()
	}
}

// noteOpacity is how much a QuickLook block shows: fully until noteFade
// before it goes, then less and less, to noteFadeTo.
func (d *deck) noteOpacity(n *block) float32 {
	at, ok := expiresAt(n)
	if !ok {
		return 1
	}
	left := at.Sub(d.now())
	if left >= noteFade {
		return 1
	}
	f := float32(max(left, 0)) / float32(noteFade)
	return noteFadeTo + (1-noteFadeTo)*f
}

// lifeLeft reads a time left: minutes up to an hour, then hours, then days.
func lifeLeft(left time.Duration) string {
	switch {
	case left <= time.Minute:
		return "< 1 分钟"
	case left <= time.Hour:
		return fmt.Sprintf("%d 分钟", int(math.Ceil(left.Minutes())))
	case left < 48*time.Hour:
		return fmt.Sprintf("%d 小时", int(math.Round(left.Hours())))
	default:
		return fmt.Sprintf("%d 天", int(math.Round(left.Hours()/24)))
	}
}

// lifeChip shows in a QuickLook block's header how long it has left, ∞
// for one kept; a press picks another span.
func (d *deck) lifeChip(c *ui.Context, n *block, pp paper, s float32) {
	at, ok := expiresAt(n)
	label, tip := "永久", "永久保留 · 点击设置消失时间"
	color := pp.muted
	if ok {
		left := at.Sub(d.now())
		label = lifeLeft(left)
		tip = at.Format("将在 1月2日 15:04 消失") + " · 点击修改"
		if left <= noteFade {
			color = pp.link
		}
	}
	size := 11 * s
	ui.Row(c.Key("life:"+n.ID)).AlignItems(ui.Center).Gap(3*s).Padding(2*s, 6*s).Radius(8 * s).
		Background(pp.code).Tooltip(tip).Label(tip).
		HandleInput(func(ev ui.InputEvent) bool {
			if ev.Kind != ui.InputPointerDown || ev.Button != 0 {
				return false
			}
			d.lifeMenu(n)
			return true
		}).
		Children(func() {
			ui.Icon(c, iconTimer).FontSize(size).TextColor(color)
			ui.Text(c, label).FontSize(size).FontWeight(500).SingleLine().TextColor(color)
		})
}

// lifeMenu pops up the spans n can be given, the one closest to its time
// left checked.
func (d *deck) lifeMenu(n *block) {
	d.lifeAsked = n // tests have no window to pop a menu up in
	if d.win == nil {
		return
	}
	at, ok := expiresAt(n)
	left := at.Sub(d.now())
	near := len(lifeChoices) - 1 // kept
	if ok {
		for i, ch := range lifeChoices {
			if ch.span > 0 && (lifeChoices[near].span == 0 ||
				absDur(ch.span-left) < absDur(lifeChoices[near].span-left)) {
				near = i
			}
		}
	}
	items := []*mygo.MenuItem{{Label: "多久后消失", Disabled: true}}
	for i, ch := range lifeChoices {
		span := ch.span
		if span == 0 {
			items = append(items, mygo.Separator())
		}
		items = append(items, &mygo.MenuItem{
			Label: ch.label, Type: mygo.MenuItemCheckbox, Checked: i == near,
			Click: func(*mygo.MenuItem, *mygo.Window) {
				d.win.Update(func() { d.setLife(n, span) })
			},
		})
	}
	mygo.NewMenu(items).Popup(d.win)
}

func absDur(v time.Duration) time.Duration {
	if v < 0 {
		return -v
	}
	return v
}
