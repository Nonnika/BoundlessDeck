package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// deckDir is the folder inside a deck's folder that holds its canvas.
const deckDir = ".boundlessdeck"

// maxRecents is how many folders the home screen remembers.
const maxRecents = 12

// app is the window's state: the home screen, or the deck of the folder
// open in it.
type app struct {
	win *mygo.Window
	// deck is the deck open, nil while the home screen shows.
	deck *deck

	recents []recent
	// recentsFile is where recents are kept: the app's data folder, or a
	// test's.
	recentsFile string
	// err says why the last folder did not open, on the home screen.
	err string
	// draggingRecent says a recent is being dragged, as this build found:
	// the trash shows while it is.
	draggingRecent bool

	// animate has the window blur out and in as it goes from one screen
	// to another: opening a folder, going home; without it, as in tests,
	// it goes at once.
	animate bool
	// switching is what changes the screen, done once the window has
	// blurred out; nil while none is under way.
	switching func()
	// blur is how blurred the window is, as the last build had it.
	blur float32
}

// recent is a folder opened before, as the home screen lists it.
type recent struct {
	Path     string    `json:"path"`
	Blocks   int       `json:"blocks"`
	OpenedAt time.Time `json:"openedAt"`
	// missing marks a folder no longer there: moved, renamed or deleted.
	missing bool
}

func newApp() *app {
	a := &app{animate: true}
	if dir, err := mygo.App.Path(mygo.PathUserData); err == nil {
		a.recentsFile = filepath.Join(dir, "recents.json")
	}
	return a
}

// view builds the window: the deck open, or the home screen.
func (a *app) view(c *ui.Context) {
	if c.Shortcut(ui.Cmd, ui.KeyO) {
		go a.pickFolder()
	}
	// Going to another screen, the whole window blurs out; at its
	// blurriest the other screen takes its place, and comes clear.
	target, d := float32(0), stageClear
	if a.switching != nil {
		target, d = stageBlur, stageBlurOut
	}
	stage := ui.Box(c.Key("stage")).Fill()
	a.blur = stage.AnimateWith("blur", target, d, ui.EaseInOut)
	if a.switching != nil && a.blur >= stageBlur-0.01 {
		do := a.switching
		a.switching = nil
		do()
		c.Invalidate() // to come clear
	}
	stage.Blur(a.blur).IgnoreInput(a.switching != nil).Children(func() {
		if a.deck != nil {
			a.deck.view(c)
			return
		}
		a.homeView(c)
	})
}

// How blurred the window gets going to another screen, how long it takes
// to, and how long the other screen then takes to come clear.
const (
	stageBlur    = 28
	stageBlurOut = 260 * time.Millisecond
	stageClear   = 420 * time.Millisecond
)

// switchTo does do, which changes the screen, once the window has blurred
// out, as the app animates; at once otherwise. A switch under way keeps
// on, and this one is dropped.
func (a *app) switchTo(do func()) {
	if !a.animate {
		do()
		return
	}
	if a.switching == nil {
		a.switching = do
	}
}

// beginOpen opens the folder as the window shows it, blurring out first.
// A folder that will not open shows why at once.
func (a *app) beginOpen(path string) {
	if info, err := os.Stat(path); err != nil || !info.IsDir() {
		a.openFolder(path)
		return
	}
	a.err = ""
	a.switchTo(func() { a.openFolder(path) })
}

// beginHome goes home as the window shows it, blurring out first.
func (a *app) beginHome() { a.switchTo(a.goHome) }

// pickFolder asks for a folder and opens it. It runs on a goroutine of
// its own, as dialogs do.
func (a *app) pickFolder() {
	dirs, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
		Parent:            a.win,
		Title:             "打开文件夹",
		Message:           "选择一个文件夹作为 deck：画布会保存在它里面的 " + deckDir + " 中。",
		ButtonLabel:       "打开",
		Directory:         true,
		CreateDirectories: true,
	})
	if err != nil || len(dirs) == 0 || a.win == nil {
		return
	}
	a.win.Update(func() { a.beginOpen(dirs[0]) })
}

// openFolder closes the deck open, if any, and opens the folder's: the
// one saved in it, or a new one.
func (a *app) openFolder(path string) {
	path = filepath.Clean(path)
	info, err := os.Stat(path)
	switch {
	case err != nil:
		a.err = fmt.Sprintf("打不开「%s」：文件夹不存在或没有权限。", filepath.Base(path))
		a.markMissing(path)
		return
	case !info.IsDir():
		a.err = fmt.Sprintf("「%s」不是文件夹。请打开一个文件夹，或把文件夹拖进窗口。", filepath.Base(path))
		return
	}
	a.closeDeck()
	d := newDeck()
	d.root = path
	d.file = filepath.Join(path, deckDir, "deck.json")
	d.win = a.win
	d.onHome = a.beginHome
	d.load()
	a.deck, a.err = d, ""
	a.touch(path, len(d.blocks))
	a.setTitle()
}

// goHome saves the deck open and shows the home screen.
func (a *app) goHome() {
	a.closeDeck()
	a.setTitle()
	a.checkRecents()
}

// closeDeck leaves the deck open: it keeps what its terminals show and
// where their shells were (keepTerminals), saves it, and hangs the shells
// up. Opened again, its terminals start anew there, under what they showed.
func (a *app) closeDeck() {
	d := a.deck
	if d == nil {
		return
	}
	d.keepTerminals()
	d.save()
	d.closeAll()
	a.touch(d.root, len(d.blocks))
	a.deck = nil
}

func (a *app) setTitle() {
	if a.win == nil {
		return
	}
	if a.deck == nil {
		a.win.SetTitle("BoundlessDeck")
		return
	}
	a.win.SetTitle(filepath.Base(a.deck.root) + " — BoundlessDeck")
}

// dropped opens a folder dropped on the home screen; on a deck, the
// files dropped become blocks where they landed.
func (a *app) dropped(paths []string, x, y float32) {
	if a.deck != nil {
		a.deck.dropFiles(paths, x, y)
		return
	}
	for _, p := range paths {
		if info, err := os.Stat(p); err == nil && info.IsDir() {
			a.beginOpen(p)
			return
		}
	}
	if len(paths) > 0 {
		a.err = "拖进来的是文件。请拖入一个文件夹，它会作为 deck 打开。"
	}
}

// touch puts the folder first among the recents, with its block count.
func (a *app) touch(path string, blocks int) {
	out := []recent{{Path: path, Blocks: blocks, OpenedAt: time.Now()}}
	for _, r := range a.recents {
		if r.Path != path {
			out = append(out, r)
		}
	}
	if len(out) > maxRecents {
		out = out[:maxRecents]
	}
	a.recents = out
	a.saveRecents()
}

func (a *app) forget(path string) {
	out := a.recents[:0]
	for _, r := range a.recents {
		if r.Path != path {
			out = append(out, r)
		}
	}
	a.recents = out
	a.saveRecents()
}

func (a *app) markMissing(path string) {
	for i := range a.recents {
		if a.recents[i].Path == path {
			a.recents[i].missing = true
		}
	}
}

func (a *app) loadRecents() {
	if a.recentsFile == "" {
		return
	}
	data, err := os.ReadFile(a.recentsFile)
	if err != nil {
		return
	}
	var rs []recent
	if json.Unmarshal(data, &rs) == nil {
		a.recents = rs
	}
	a.checkRecents()
}

// checkRecents marks the recents whose folders are gone, off the main
// thread when there is a window.
func (a *app) checkRecents() {
	paths := make([]string, len(a.recents))
	for i, r := range a.recents {
		paths[i] = r.Path
	}
	check := func() map[string]bool {
		gone := map[string]bool{}
		for _, p := range paths {
			if info, err := os.Stat(p); err != nil || !info.IsDir() {
				gone[p] = true
			}
		}
		return gone
	}
	apply := func(gone map[string]bool) {
		for i := range a.recents {
			a.recents[i].missing = gone[a.recents[i].Path]
		}
	}
	if a.win == nil {
		apply(check())
		return
	}
	go func() {
		gone := check()
		a.win.Update(func() { apply(gone) })
	}()
}

// saveRecents writes the recents atomically.
func (a *app) saveRecents() {
	if a.recentsFile == "" {
		return
	}
	data, err := json.MarshalIndent(a.recents, "", "  ")
	if err != nil {
		return
	}
	if err := os.MkdirAll(filepath.Dir(a.recentsFile), 0o755); err != nil {
		return
	}
	tmp := a.recentsFile + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return
	}
	_ = os.Rename(tmp, a.recentsFile)
}

// homeView is the home screen: open a folder as a deck, or one opened
// before. It stands on the canvas seen in 3D, running off into the
// distance (homeScene).
func (a *app) homeView(c *ui.Context) {
	t := c.Theme()
	pp := paperFor(t)
	cw, ch := c.Size()
	a.draggingRecent = false
	ui.Box(c).Fill().Background(voidColor(t)).Clip().Children(func() {
		homeScene(c, t, pp, cw, ch, cw/2, homeTop+150)
		// Absolute, as the scene is, to paint over it.
		ui.Scroll(c).Absolute().Left(0).Top(0).Size(cw, ch).Children(func() {
			ui.Column(c).FillWidth().AlignItems(ui.Center).Padding(homeTop, 24, 48, 24).Children(func() {
				ui.Column(c).Width(560).Gap(32).Children(func() {
					a.homeHead(c, t)
					a.homeBlock(c, t, pp)
				})
			})
		})
		// Over the column's top: there is none to click up there.
		dragBand(c, c.TitleBar().Height)
		a.trashView(c, t, cw, ch)
	})
}

// homeTop is the room above the home screen's column.
const homeTop = 96

func (a *app) homeHead(c *ui.Context, t *ui.Theme) {
	ui.Column(c).Gap(12).Children(func() {
		// Pulled together: Deck's line box has room above its capitals.
		ui.Column(c).Gap(-14).Children(func() {
			boundlessWord(c, t)
			ui.Text(c, "Deck").FontSize(96).FontWeight(800).LetterSpacing(-3).LineHeight(0.9).
				TextColor(t.Text)
		})
		endless(c, t)
	})
}

// endlessWords take turns before "无止境" under the title.
var endlessWords = []string{"创意", "创造", "创作"}

// How long a word of endlessWords stays, and how long the next takes to
// slide in over it.
const (
	endlessHold  = 2200 * time.Millisecond
	endlessSlide = 450 * time.Millisecond
)

// endlessAt says which word shows at now and, while the next slides in,
// how far it has come, from 0 to 1 (0 while none slides).
func endlessAt(now time.Time) (word int, slide float32) {
	period := endlessHold + endlessSlide
	n := now.UnixNano() / int64(period)
	word = int(n % int64(len(endlessWords)))
	in := time.Duration(now.UnixNano() % int64(period))
	if in < endlessHold {
		return word, 0
	}
	return word, float32(in-endlessHold) / float32(endlessSlide)
}

// endless is the line under the title: "创意无止境", its first word taking
// turns with the others of endlessWords, each sliding up out of the way of
// the next. Only the painting moves: the view is not built again for it.
func endless(c *ui.Context, t *ui.Theme) {
	const size, weight = 22, 600
	accent, text := t.Accent, t.Text
	ui.Row(c).AlignItems(ui.Baseline).Children(func() {
		ui.Box(c).Label("创意、创造、创作").
			Draw(func(p *ui.Painter, r ui.Rect) {
				word, slide := endlessAt(p.Now())
				draw := func(s string, dy, alpha float32) {
					sp := ui.Span{Text: s, Size: size, Weight: weight, Color: accent.Alpha(alpha)}
					// Centered in the box, as the unseen word is in its
					// line, so it sits on the line "无止境" sits on.
					_, th := p.MeasureText(r.W+size, sp)
					p.RichText(r.X, r.Y+(r.H-th)/2+dy, r.W+size, sp)
				}
				p.Clip(r, 0, func() {
					if slide == 0 {
						draw(endlessWords[word], 0, 1)
						return
					}
					e := 1 - (1-slide)*(1-slide)*(1-slide) // ease out
					next := endlessWords[(word+1)%len(endlessWords)]
					draw(endlessWords[word], -e*r.H, 1-e)
					draw(next, (1-e)*r.H, e)
				})
				if slide == 0 {
					p.After(endlessHold - time.Duration(p.Now().UnixNano()%int64(endlessHold+endlessSlide)))
				} else {
					p.AnimationFrame()
				}
			}).
			Children(func() {
				// Unseen, it gives the box its size and baseline; the words
				// are all as wide.
				ui.Text(c, endlessWords[0]).FontSize(size).FontWeight(weight).SingleLine().
					TextColor(accent.Alpha(0))
			})
		ui.Text(c, "无止境").FontSize(size).FontWeight(weight).TextColor(text)
	})
}

// boundlessWord is "Boundless" in motion: the word, and behind it its trail
// to the left, fainter and more blurred the farther back, as though it
// were rushing on and had no end.
func boundlessWord(c *ui.Context, t *ui.Theme) {
	const size, weight = 38, 600
	word := func() ui.Element {
		return ui.Text(c, "Boundless").FontSize(size).FontWeight(weight).SingleLine()
	}
	ui.Box(c).Children(func() {
		// In the flow but unseen, it gives the box its size and baseline.
		word().TextColor(t.Text.Alpha(0))
		// Many faint copies, close together, read as one streak; little
		// blur on each keeps it from smearing up and down.
		const trail, length = 18, 90
		for i := trail; i >= 1; i-- {
			f := float32(i) / trail
			word().Absolute().Left(-f * length).Top(0).
				TextColor(t.Text.Alpha(0.09 * (1 - f) * (1 - f))).
				Blur(0.4 + 1.6*f)
		}
		word().Absolute().Left(0).Top(0).TextColor(t.Text)
	})
}

// homeBlock is the home screen's block: the folders opened before, and in
// its title bar the button to open another. It is drawn as a block on the
// canvas is: a frame with a title bar over its body and its tape behind
// it, sticking out above; pure white outlined in gray, a faint shadow
// under it, square buttons and no line under the title bar.
func (a *app) homeBlock(c *ui.Context, t *ui.Theme, pp paper) {
	// The box behind the block paints the tape; the block, painted after,
	// covers all of it but what sticks out above.
	ui.Box(c).FillWidth().Draw(func(p *ui.Painter, r ui.Rect) { drawBlockTape(p, r, t, -2) }).
		Children(func() { a.homeBlockFrame(c, t, pp) })
}

// homeBlockLook is the home block's face, its gray outline and the faint
// shadow it casts: pure white, or, dark, pure black.
func homeBlockLook(t *ui.Theme) (face, outline, shadow ui.Color) {
	if t.Dark {
		return ui.Hex("#000000"), ui.Hex("#5a5a5f"), ui.RGBA(0, 0, 0, 0.6)
	}
	return ui.Hex("#ffffff"), ui.Hex("#b4b4b9"), ui.RGBA(0, 0, 0, 0.12)
}

func (a *app) homeBlockFrame(c *ui.Context, t *ui.Theme, pp paper) {
	face, outline, shadow := homeBlockLook(t)
	ui.Column(c).FillWidth().Background(face).Border(1, outline).Shadow(0, 2, 6, 0, shadow).
		Clip().Children(func() {
		ui.Row(c).FillWidth().Height(46).Padding(0, 8, 0, 14).Gap(8).AlignItems(ui.Center).
			Children(func() {
				ui.Text(c, "最近的 deck").FontSize(13).FontWeight(600).TextColor(t.Text)
				if len(a.recents) > 0 {
					ui.Text(c, fmt.Sprint(len(a.recents))).FontSize(12).TextColor(t.TextMuted)
				}
				ui.Box(c).Grow(1)
				ui.PrimaryButton(c.Key("open-folder"), "").Label("打开文件夹").Tooltip("⌘O，或把文件夹拖进窗口").
					Padding(6, 12).Gap(6).Radius(0).
					Children(func() {
						ui.Icon(c, iconFolder).FontSize(14)
						ui.Text(c, "打开文件夹…").FontSize(13).FontWeight(600).SingleLine()
					}).
					OnClick(func() { go a.pickFolder() })
			})
		ui.Column(c).FillWidth().Padding(4).Gap(2).Children(func() {
			if a.err != "" {
				ui.Text(c, a.err).FontSize(13).TextColor(t.Danger).Padding(8, 12)
			}
			if len(a.recents) == 0 {
				ui.Text(c, "还没有打开过文件夹。打开一个，它就是一张 deck。").
					FontSize(13).TextColor(t.TextMuted).Padding(20, 12)
				return
			}
			now := time.Now()
			for _, r := range a.recents {
				a.recentRow(c, t, pp, r, now)
			}
		})
	})
}

// recentDrag is a recent being dragged, by its folder's path: to the
// trash, which takes it off the list.
type recentDrag string

// recentRow is one folder of the recents: a click opens it; dragged to the
// trash that shows at the window's bottom meanwhile, it leaves the list
// (the folder stays as it is).
func (a *app) recentRow(c *ui.Context, t *ui.Theme, pp paper, r recent, now time.Time) {
	path := r.Path
	row := ui.ButtonBase(c.Key("recent:"+path)).FillWidth().Padding(10, 12).Gap(12).Radius(0).
		AlignItems(ui.Center).Label(filepath.Base(path)).Drag(recentDrag(path))
	dragging := row.Dragging()
	if dragging {
		// The copy following the pointer is this row, as it is but for a
		// card's face and outline, the block's. In place, it stays faint.
		a.draggingRecent = true
		face, outline, _ := homeBlockLook(t)
		row = row.Background(face).Border(1, outline).Opacity(0.35)
	} else if row.Hovered() {
		row = row.Background(t.SurfaceHover)
	}
	muted := t.TextMuted
	missing := r.missing
	row.Children(func() {
		mark := ui.Box(c).Size(30, 30).Draw(func(p *ui.Painter, b ui.Rect) {
			drawDeckMark(p, b, t, pp, false)
		})
		if missing {
			mark.Opacity(0.4)
		}
		ui.Column(c).Grow(1).Gap(2).Children(func() {
			name := ui.Text(c, filepath.Base(path)).FontSize(14).FontWeight(600).SingleLine()
			if missing {
				name.TextColor(muted)
			} else {
				name.TextColor(t.Text)
			}
			ui.Text(c, shortPath(path)).FontSize(12).SingleLine().Ellipsis("…").TextColor(muted)
		})
		switch {
		case missing:
			ui.Text(c, "找不到这个文件夹").FontSize(12).TextColor(t.Danger)
		default:
			ui.Text(c, fmt.Sprintf("%d 个 block · %s", r.Blocks, ago(now, r.OpenedAt))).
				FontSize(12).SingleLine().TextColor(muted)
		}
	})
	row.OnClick(func() { a.beginOpen(path) })
}

// trashZone is how high the band along the window's bottom is that a
// recent dragged onto leaves the list.
const trashZone = 170

// trashView is the band along the window's bottom that a recent dragged
// onto leaves the list: no control, a red glow rising from the window's
// bottom edge, blurred more the higher it reaches, and a word saying what
// letting go there does. It shows
// while a recent is dragged, and glows more with one over it; hidden, it
// is still built, to take the recent let go on it.
func (a *app) trashView(c *ui.Context, t *ui.Theme, cw, ch float32) {
	shown := a.draggingRecent
	el := ui.Box(c.Key("recent-trash")).Absolute().Left(0).Top(ch-trashZone).Size(cw, trashZone).
		Label("移除").IgnoreInput(!shown)
	if p, ok := ui.Drop[recentDrag](el); ok {
		a.forget(string(p))
		c.Invalidate()
	}
	_, over := ui.DragOver[recentDrag](el)
	if !shown {
		el = el.Opacity(0)
	}
	red := t.Danger
	strength := float32(1)
	if over {
		strength = 1.7
	}
	// Under it all, the band tinted, fading out upward.
	el.Draw(func(p *ui.Painter, r ui.Rect) {
		p.FillGradient(r, ui.LinearGradient{From: red.Alpha(0), To: red.Alpha(0.1 * strength), Angle: 180, Oklab: true}, 0)
	}).Children(func() {
		// The glow: red layers along the bottom edge, the outer ones
		// wider, higher and more blurred, so that it softens and fades
		// the farther from the edge.
		for _, l := range []struct{ inset, h, blur, alpha float32 }{
			{-0.15, 150, 60, 0.18},
			{0.05, 90, 36, 0.22},
			{0.2, 46, 18, 0.28},
		} {
			ui.Box(c).Absolute().Left(cw * l.inset).Width(cw * (1 - 2*l.inset)).
				Top(trashZone - l.h/2).Height(l.h).
				Background(red.Alpha(min(l.alpha*strength, 0.6))).Blur(l.blur)
		}
		ui.Row(c).Absolute().Left(0).Bottom(36).Width(cw).Gap(8).AlignItems(ui.Center).Justify(ui.Center).
			Children(func() {
				ui.Icon(c, iconTrash).FontSize(16).TextColor(red)
				label := "拖到这里，从列表移除"
				if over {
					label = "松开即可移除（文件夹不会被删除）"
				}
				ui.Text(c, label).FontSize(13).FontWeight(600).TextColor(red).SingleLine()
			})
	})
}

// shortPath writes a path under the home folder from ~.
func shortPath(p string) string {
	if home, err := os.UserHomeDir(); err == nil && home != "" {
		if p == home {
			return "~"
		}
		if strings.HasPrefix(p, home+string(filepath.Separator)) {
			return "~" + p[len(home):]
		}
	}
	return p
}

// ago says how long ago t was, as the recents show it.
func ago(now, t time.Time) string {
	d := now.Sub(t)
	switch {
	case d < time.Minute:
		return "刚刚"
	case d < time.Hour:
		return fmt.Sprintf("%d 分钟前", int(d.Minutes()))
	case d < 24*time.Hour:
		return fmt.Sprintf("%d 小时前", int(d.Hours()))
	case d < 7*24*time.Hour:
		return fmt.Sprintf("%d 天前", int(d.Hours()/24))
	}
	return t.Format("2006-01-02")
}
