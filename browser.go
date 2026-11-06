package main

import (
	"net/url"
	"strings"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// A browser block shows a web page on the canvas: a native web view where
// its body is, under a bar to go back and forward, reload and type an
// address. The page zooms with the canvas (Page.SetZoomFactor), as text
// does: its layout stays as it was, crisp at any zoom. Pages are
// untrusted: they call none of the app's Go.

// browserBar is the height of a browser block's bar at 100%.
const browserBar = 34

// searchURL is where words typed in the address bar are looked up.
const searchURL = "https://www.bing.com/search?q="

// addressFocus is what d.focusBlock holds while a browser block's address
// bar has the keyboard.
func addressFocus(n *block) string { return "addr:" + n.ID }

// normalizeAddress turns what is typed in the address bar into a URL: an
// address as it is, a host without its scheme over https (http for this
// machine), and words a search.
func normalizeAddress(s string) string {
	s = strings.TrimSpace(s)
	switch {
	case s == "":
		return ""
	case strings.Contains(s, "://"), strings.HasPrefix(s, "about:"), strings.HasPrefix(s, "data:"):
		return s
	case strings.HasPrefix(s, "localhost"), strings.HasPrefix(s, "127.0.0.1"):
		return "http://" + s
	case strings.ContainsAny(s, " \t") || !strings.Contains(s, "."):
		return searchURL + url.QueryEscape(s)
	}
	return "https://" + s
}

// startBrowser makes the block's web view, at its URL, off the main
// thread, and follows its page: title, address, history, loading.
func (d *deck) startBrowser(n *block, rt *runtime) {
	if rt.web != nil || rt.webStarting || rt.webErr != nil || n.URL == "" {
		return
	}
	if d.win == nil {
		rt.webErr = errNoWindow
		return
	}
	win := d.win
	var gen int
	var start string
	begin := func() { rt.webStarting, gen, start = true, rt.webGen, n.URL }
	d.wantLoad("browser:"+n.ID, n, begin, func() func() {
		// Blank at first: it loads its page once its listeners are on, so
		// that the page's first redirect is let through (webScheme).
		v, err := win.NewWebView(mygo.WebViewOptions{
			Page: mygo.PageOptions{Untrusted: true},
		})
		return func() {
			if rt.webGen != gen {
				// Closed while it started: its block or deck went.
				if v != nil {
					v.Destroy()
				}
				return
			}
			rt.webStarting, rt.web, rt.webErr = false, v, err
			if v == nil {
				return
			}
			d.followPage(n, rt, v)
			rt.webLoading = true
			if err := v.Page().LoadURL(start); err != nil {
				rt.webLoading, rt.webFailed = false, err.Error()
			}
		}
	})
}

// followPage keeps the block up with its page. The page's events come on
// goroutines of their own; the block changes on the main thread.
func (d *deck) followPage(n *block, rt *runtime, v *mygo.WebView) {
	win, gen, p := d.win, rt.webGen, v.Page()
	update := func(fn func()) {
		win.Update(func() {
			if rt.webGen == gen {
				fn()
			}
		})
	}
	p.OnPageTitleUpdated(func(e *mygo.TitleEvent) {
		title := e.Title
		update(func() { n.autoTitle(title) })
	})
	p.OnDidNavigate(func(u string) {
		update(func() {
			n.URL, rt.webLoading, rt.webFailed = u, true, ""
			if d.focusBlock != addressFocus(n) {
				rt.address = u
			}
			rt.canBack, rt.canForward = p.CanGoBack(), p.CanGoForward()
		})
	})
	// An untrusted page goes to no other origin unless this says so: a
	// browser goes wherever the web takes it, redirects too, but not from
	// a page to the disk or to the app's own schemes.
	p.OnWillNavigate(func(e *mygo.NavigateEvent) {
		if !webScheme(e.URL) {
			e.PreventDefault()
		}
	})
	p.OnDidFinishLoad(func() { update(func() { rt.webLoading = false }) })
	p.OnDidFailLoad(func(e *mygo.LoadError) {
		msg := e.Description
		update(func() { rt.webLoading, rt.webFailed = false, msg })
	})
	// A link to open in a new window opens in a browser block of its own,
	// beside this one; a pop-up the page opens by itself does not.
	p.SetWindowOpenHandler(func(req mygo.WindowOpenRequest) *mygo.WindowOptions {
		if req.UserInitiated && req.URL != "" {
			u := req.URL
			update(func() { d.openBeside(n, u) })
		}
		return nil
	})
}

// webScheme reports whether a page may go to the address: one of the web,
// as a link or a redirect takes it.
func webScheme(address string) bool {
	u, err := url.Parse(address)
	if err != nil {
		return false
	}
	switch strings.ToLower(u.Scheme) {
	case "http", "https", "about", "data", "blob":
		return true
	}
	return false
}

// openBeside adds a browser block at the address, to the right of n.
func (d *deck) openBeside(n *block, address string) *block {
	b := &block{Kind: kindBrowser, Title: "浏览器", URL: address, W: n.W, H: n.H}
	b.X, b.Y = d.freeSpot(n.X+n.W+40, n.Y, b.W, b.H)
	d.addBlock(b)
	return b
}

// navigate takes the block's page to what was typed in its address bar.
func (d *deck) navigate(n *block, rt *runtime) {
	u := normalizeAddress(rt.address)
	if u == "" {
		return
	}
	n.URL, rt.address, rt.webFailed = u, u, ""
	d.focusBlock = n.ID // the page takes the keyboard
	if rt.web != nil {
		if err := rt.web.Page().LoadURL(u); err != nil {
			rt.webFailed = err.Error()
		}
		return
	}
	rt.webErr = nil
	d.startBrowser(n, rt)
}

// browserBody is a browser block's bar, and its page under it.
func (d *deck) browserBody(c *ui.Context, n *block) {
	t := c.Theme()
	rt := d.ensureRuntime(n)
	if rt.address == "" && d.focusBlock != addressFocus(n) {
		rt.address = n.URL
	}
	d.startBrowser(n, rt)
	s := d.lay.S
	d.browserBarView(c, n, rt, s)

	ui.Box(c).Grow(1).FillWidth().Children(func() {
		switch {
		case n.URL == "":
			bodyCentered(c, "输入网址或搜索词，按回车打开", t.TextMuted, 13*s)
		case rt.webErr != nil:
			bodyMessage(c, rt.webErr.Error(), t.Danger)
		case rt.web == nil:
			bodyMessage(c, "打开中…", t.TextMuted)
		default:
			// The page takes the canvas's zoom, as it shows: its layer is
			// scaled to it (zoomlayer.go).
			snap, held := d.zoomPage(c, rt, d.cam.S)
			switch {
			case snap != nil:
				// Zooming: the page as a picture, scaled with the block.
				ui.Image(c, snap).Fill().Fit(ui.FillBox)
			case held:
				// Zooming, its picture not taken yet: the page held at the
				// size it has its zoom for, so that it lays out nothing
				// anew meanwhile.
				// A web view covers its box as the layer's scale shows it.
				z := rt.webZoom / d.layerScale()
				ui.WebView(c, rt.web).Absolute().Left(0).Top(0).
					Size(n.W*z, (n.H-titleHeightAt1-browserBar)*z).FocusBind(&d.focusBlock, n.ID)
			default:
				ui.WebView(c, rt.web).Grow(1).FillWidth().FocusBind(&d.focusBlock, n.ID)
			}
		}
	})
}

// While the canvas zooms, a page shows as a picture of it, scaled with its
// block as a photo is: zooming the page itself lays it out and draws it
// again at each step, behind the block, its text growing in a frame that
// has not yet. Once the zoom rests for webZoomRest, the page takes the
// canvas's zoom (Page.SetZoomFactor), still under its picture for
// webZoomRedraw while it draws itself again, and shows.
const (
	webZoomRest   = 220 * time.Millisecond
	webZoomRedraw = 180 * time.Millisecond
)

// zoomPage keeps the page's zoom up with the canvas's, s, and returns the
// picture to show in its place while the two differ, or, before there is
// one, held: the page is to keep the size it has its zoom for. It returns
// neither once the page shows as it is.
func (d *deck) zoomPage(c *ui.Context, rt *runtime, s float32) (snap ui.ImageSource, held bool) {
	now := time.Now()
	if rt.webZoom == 0 { // first shown: at the canvas's zoom at once
		rt.web.Page().SetZoomFactor(float64(s))
		rt.webZoom, rt.webSeenS = s, s
		return nil, false
	}
	if s != rt.webSeenS {
		// The canvas zoomed since the last frame: a picture of the page as
		// it is, to scale while it goes on.
		rt.webSeenS, rt.webZoomRest = s, now.Add(webZoomRest)
		d.capturePage(rt)
	}
	switch {
	case now.Before(rt.webZoomRest):
		c.After(rt.webZoomRest.Sub(now))
	case rt.webZoom != s:
		// At rest: the page takes the zoom, under its picture until it
		// has drawn itself again.
		rt.web.Page().SetZoomFactor(float64(s))
		rt.webZoom, rt.webRedrawn = s, now.Add(webZoomRedraw)
		c.After(webZoomRedraw)
	case now.Before(rt.webRedrawn):
		c.After(rt.webRedrawn.Sub(now))
	default:
		if rt.webSnap != nil {
			rt.webSnapOld = rt.webSnap
		}
		rt.webSnap = nil // the next zoom takes a picture of the page anew
		return nil, false
	}
	if rt.webSnap == nil {
		// No picture yet: the page as it was, until there is one. Only
		// while it has not taken the new zoom: it then shows at its size.
		return nil, rt.webZoom != s
	}
	if rt.webSnap == nil && rt.webSnapOld != nil {
		// The last zoom's picture, until this one's comes.
		return rt.webSnapOld, false
	}
	return rt.webSnap, false
}

// capturePage takes a picture of the page, off the main thread, unless it
// has one or takes one.
func (d *deck) capturePage(rt *runtime) {
	if rt.webSnap != nil || rt.webCapturing || d.win == nil {
		return
	}
	rt.webCapturing = true
	v, gen, win := rt.web, rt.webGen, d.win
	go func() {
		var snap *ui.Bitmap
		if data, err := v.CapturePage(); err == nil {
			snap, _ = ui.DecodeBitmap(data)
		}
		win.Update(func() {
			if rt.webGen != gen {
				return
			}
			rt.webCapturing = false
			if snap != nil && rt.webZoom != rt.webSeenS {
				rt.webSnap = snap
			}
		})
	}()
}

// browserBarView is the bar of a browser block: back, forward, reload or
// stop, and the address.
func (d *deck) browserBarView(c *ui.Context, n *block, rt *runtime, s float32) {
	t := c.Theme()
	icon := 15 * s
	button := func(key string, svg *ui.SVG, label string, enabled bool, fn func()) {
		ui.Button(c.Key(key+n.ID), "").Label(label).Padding(4*s, 6*s).Radius(0).Disabled(!enabled).
			Children(func() { ui.Icon(c, svg).FontSize(icon) }).
			OnClick(fn)
	}
	page := func() *mygo.Page {
		if rt.web == nil {
			return nil
		}
		return rt.web.Page()
	}
	ui.Row(c).FillWidth().Height(browserBar*s).Padding(0, 4*s).Gap(2*s).AlignItems(ui.Center).
		BorderWidth(0, 0, 1, 0).BorderColor(t.Border).Children(func() {
		button("web-back:", iconArrowLeft, "后退", rt.canBack, func() { page().GoBack() })
		button("web-forward:", iconArrowRight, "前进", rt.canForward, func() { page().GoForward() })
		if rt.webLoading {
			button("web-stop:", iconX, "停止", rt.web != nil, func() { page().Stop(); rt.webLoading = false })
		} else {
			button("web-reload:", iconReload, "重新加载", rt.web != nil, func() { page().Reload() })
		}
		addr := ui.TextInput(c.Key("web-address:"+n.ID), &rt.address).Label("地址").
			Placeholder("输入网址或搜索").FontSize(12*s).Padding(3*s, 8*s).Radius(0).Grow(1).
			FocusBind(&d.focusBlock, addressFocus(n))
		if addr.Submitted() {
			d.navigate(n, rt)
		}
	})
	if rt.webFailed != "" {
		ui.Text(c, "打不开："+rt.webFailed).FontSize(11*s).TextColor(t.Danger).Padding(2*s, 8*s).SingleLine()
	}
}

// inBrowserBody reports whether the point of the window, in DIPs, is in
// the body of a browser block, below its title bar.
func (d *deck) inBrowserBody(x, y float32) bool {
	for _, n := range d.blocks {
		if n.Kind != kindBrowser {
			continue
		}
		sx, sy := d.cam.toScreen(n.X, n.Y)
		top := sy + titleHeightAt1*d.cam.S
		if x >= sx && x < sx+n.W*d.cam.S && y >= top && y < sy+n.H*d.cam.S {
			return true
		}
	}
	return false
}

// bodyCentered fills a block's body with a line of text in its middle.
func bodyCentered(c *ui.Context, msg string, color ui.Color, size float32) {
	ui.Box(c).Fill().Center().Padding(10).Children(func() {
		ui.Text(c, msg).FontSize(size).TextColor(color)
	})
}

// closeBrowser lets the block's web view go, and one starting meanwhile:
// shown again, as an undone removal brings it back, it opens anew.
func (rt *runtime) closeBrowser() {
	rt.webGen++
	if rt.web != nil {
		rt.web.Destroy()
	}
	rt.web, rt.webErr, rt.webStarting, rt.webZoom = nil, nil, false, 0
	rt.webSnap, rt.webSnapOld, rt.webCapturing, rt.webSeenS = nil, nil, false, 0
	rt.webLoading, rt.canBack, rt.canForward = false, false, false
}
