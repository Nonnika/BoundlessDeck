package main

import (
	"testing"

	"github.com/egoist/mygo/ui"
)

func TestTheAddressBarTakesAddressesAndWords(t *testing.T) {
	for in, want := range map[string]string{
		"":                        "",
		"  example.com ":          "https://example.com",
		"https://go.dev/doc":      "https://go.dev/doc",
		"localhost:3000":          "http://localhost:3000",
		"127.0.0.1:8080/x":        "http://127.0.0.1:8080/x",
		"about:blank":             "about:blank",
		"golang generics":         searchURL + "golang+generics",
		"无限画布":                    searchURL + "%E6%97%A0%E9%99%90%E7%94%BB%E5%B8%83",
		"file:///Users/me/a.html": "file:///Users/me/a.html",
	} {
		if got := normalizeAddress(in); got != want {
			t.Errorf("normalizeAddress(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestANewBrowserTakesAnAddress(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 1100, 760)
	tt.Frame()
	if err := tt.Click("浏览器"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	tt.Frame()
	b := d.blocks[len(d.blocks)-1]
	if b.Kind != kindBrowser || d.focusBlock != addressFocus(b) {
		t.Fatalf("added %v, focus %q: want a browser, its address bar focused", b.Kind, d.focusBlock)
	}
	if !tt.HasText("输入网址或搜索词") {
		t.Fatal("a new browser shows no hint")
	}

	tt.Type("example.com")
	tt.Frame()
	tt.TypeKey(0, ui.KeyEnter, "")
	tt.Frame()
	tt.Frame()
	if b.URL != "https://example.com" {
		t.Fatalf("after Enter the browser is at %q", b.URL)
	}
	if d.focusBlock != b.ID {
		t.Fatalf("focus %q: want the page", d.focusBlock)
	}
	// No window in tests: the page cannot show, and says so.
	if !tt.HasText("需要一个窗口") {
		t.Fatalf("shows %q", tt.Texts())
	}

	// Clicked again, the address bar takes the keyboard: the card and the
	// canvas leave presses in a browser's body to it.
	if err := tt.Click("地址"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	tt.Type("/docs")
	tt.Frame()
	tt.TypeKey(0, ui.KeyEnter, "")
	tt.Frame()
	tt.Frame()
	if b.URL != "https://example.com/docs" {
		t.Fatalf("after a click, typing and Enter, the browser is at %q", b.URL)
	}
	if d.marquee.on {
		t.Fatal("the click on the address bar went to the canvas")
	}

	// Saved with its address.
	d.save()
	e := newDeck()
	e.root, e.file = d.root, d.file
	e.load()
	if got := e.blocks[len(e.blocks)-1]; got.Kind != kindBrowser || got.URL != "https://example.com/docs" {
		t.Fatalf("loaded %v at %q", got.Kind, got.URL)
	}
}

func TestALinkToANewWindowOpensBeside(t *testing.T) {
	d := testDeck(t)
	n := &block{ID: "w", Kind: kindBrowser, URL: "https://a.example", X: 100, Y: 100, W: 600, H: 400}
	d.addBlock(n)
	b := d.openBeside(n, "https://b.example")
	if b.Kind != kindBrowser || b.URL != "https://b.example" || b.X < n.X+n.W {
		t.Fatalf("opened %+v", b)
	}
	// Removed, a browser lets its web view go: one starting is dropped.
	rt := d.ensureRuntime(b)
	rt.webStarting = true
	gen := rt.webGen
	d.removeBlock(b)
	if rt.webGen == gen || rt.webStarting {
		t.Fatal("a removed browser block keeps its web view starting")
	}
}

func TestAPageGoesAnywhereOnTheWebButNotToTheDisk(t *testing.T) {
	for address, ok := range map[string]bool{
		"https://cn.bing.com/search?q=hello": true, // a redirect to another origin
		"http://example.com":                 true,
		"about:blank":                        true,
		"data:text/html,hi":                  true,
		"file:///etc/passwd":                 false,
		"boundlessdeck://x":                  false,
		"javascript:alert(1)":                false,
	} {
		if webScheme(address) != ok {
			t.Errorf("webScheme(%q) = %v, want %v", address, !ok, ok)
		}
	}
}
