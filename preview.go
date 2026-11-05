package main

import (
	"bytes"
	"errors"
	"fmt"
	"image"
	"os"
	"path/filepath"
	"strings"

	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

// preview is how a file block shows its file.
type preview uint8

const (
	previewText     preview = iota // in the editor, read-only
	previewImage                   // the picture, on a checkerboard where it is see-through
	previewMarkdown                // .md, rendered
	previewPDF                     // a page at a time
	previewWord                    // .docx and .doc, rendered as Markdown
)

func previewOf(path string) preview {
	switch strings.ToLower(filepath.Ext(path)) {
	case ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg":
		return previewImage
	case ".md", ".markdown":
		return previewMarkdown
	case ".pdf":
		return previewPDF
	case ".docx", ".doc":
		return previewWord
	}
	return previewText
}

// docFontSize is the size of a rendered document's text at 100%: the
// canvas's zoom scales it.
const docFontSize = 13

// document is a .md or Word file read for its preview.
type document struct {
	markdown string
	// media are a Word document's pictures, by the names its Markdown
	// gives them.
	media map[string][]byte
}

// loadDocument reads the block's .md or Word file, off the main thread.
func (d *deck) loadDocument(n *block, rt *runtime) {
	if rt.loading || rt.doc != nil || rt.docErr != nil {
		return
	}
	path, kind := d.path(n), previewOf(n.Path)
	rt.loading = true
	read := func() (*document, error) {
		if kind == previewWord {
			w, err := readWord(path)
			if err != nil {
				return nil, err
			}
			return &document{markdown: w.markdown, media: w.media}, nil
		}
		f, err := os.Open(path)
		if err != nil {
			return nil, err
		}
		defer f.Close()
		buf := make([]byte, maxTextBytes)
		k, err := f.Read(buf)
		if err != nil && k == 0 {
			return nil, err
		}
		return &document{markdown: string(buf[:k])}, nil
	}
	done := func(doc *document, err error) {
		rt.loading, rt.doc, rt.docErr = false, doc, err
	}
	if d.win == nil { // tests: no goroutine to hand results to
		done(read())
		return
	}
	go func() {
		doc, err := read()
		d.win.Update(func() { done(doc, err) })
	}()
}

// documentBody shows a .md or Word file rendered, scrolling once the block
// is focused (bodyCover), its text growing and shrinking with the canvas.
func (d *deck) documentBody(c *ui.Context, n *block) {
	t := c.Theme()
	rt := d.ensureRuntime(n)
	d.loadDocument(n, rt)
	switch {
	case rt.docErr != nil:
		bodyMessage(c, rt.docErr.Error(), t.Danger)
		return
	case rt.doc == nil:
		bodyMessage(c, "加载中…", t.TextMuted)
		return
	}
	s := d.cam.S
	opts := d.documentOptions(n, rt, t, docFontSize*s)
	ui.Scroll(c.Key("doc:"+n.ID)).Grow(1).FillWidth().Focusable().FocusBind(&d.focusBlock, n.ID).
		Children(func() {
			ui.Column(c).FillWidth().Padding(12*s, 16*s, 16*s, 16*s).Children(func() {
				if strings.TrimSpace(rt.doc.markdown) == "" {
					ui.Text(c, "（空文档）").FontSize(docFontSize * s).TextColor(t.TextMuted)
					return
				}
				markdown.View(c, rt.doc.markdown, opts)
			})
		})
}

// documentOptions are the options the block's document renders with: the
// theme's colors at the size the camera gives, and its pictures: files
// beside a .md, those inside a Word document.
func (d *deck) documentOptions(n *block, rt *runtime, t *ui.Theme, size float32) *markdown.Options {
	if rt.mdOpts == nil {
		o := &markdown.Options{}
		if media := rt.doc.media; media != nil {
			o.LoadImage = func(url string) (ui.ImageSource, error) {
				data, ok := media[strings.TrimPrefix(url, wordMedia)]
				if !ok || !strings.HasPrefix(url, wordMedia) {
					return nil, errors.New("文档里没有这张图片")
				}
				return ui.DecodeImage(data)
			}
		} else {
			o.LoadImage = markdown.FileImages(filepath.Dir(d.path(n)))
		}
		rt.mdOpts = o
	}
	if rt.mdOpts.Theme == nil || rt.mdSize != size || rt.mdDark != t.Dark {
		th := markdown.DefaultTheme(t)
		th.FontSize = size
		rt.mdOpts.Theme, rt.mdSize, rt.mdDark = th, size, t.Dark
	}
	return rt.mdOpts
}

// bodyMessage fills a block's body with a line of text: loading, or why
// its file does not show.
func bodyMessage(c *ui.Context, msg string, color ui.Color) {
	ui.Box(c).Fill().Padding(10).Children(func() {
		ui.Text(c, msg).FontSize(12).TextColor(color)
	})
}

// pdfDensity is how many pixels a PDF page renders at for each DIP it
// shows at: the Retina displays of the Macs that run the app.
const pdfDensity = 2

// pdfWidth is how many pixels wide a page renders for a body w DIPs wide:
// in steps, so that zooming renders it again only now and then.
func pdfWidth(w float32) int {
	const step = 256
	px := int(w * pdfDensity)
	return min(max((px+step-1)/step*step, step), 4096)
}

// openPDFDoc opens the block's PDF, off the main thread.
func (d *deck) openPDFDoc(n *block, rt *runtime) {
	if rt.pdf != nil || rt.pdfErr != nil || rt.pdfOpening {
		return
	}
	path, gen := d.path(n), rt.pdfGen
	rt.pdfOpening = true
	done := func(doc *pdfDoc, err error) {
		rt.pdfOpening, rt.pdf, rt.pdfErr = false, doc, err
	}
	if d.win == nil {
		done(openPDF(path))
		return
	}
	go func() {
		doc, err := openPDF(path)
		d.win.Update(func() {
			if rt.pdfGen != gen {
				// Closed while it opened: its block or deck went.
				if doc != nil {
					doc.Close()
				}
				return
			}
			done(doc, err)
		})
	}()
}

// renderPDFPage renders the page the block shows, width pixels wide, off
// the main thread, unless it shows already or one renders.
func (d *deck) renderPDFPage(rt *runtime, page, width int) {
	if rt.pdfRendering || (rt.pdfPage == page && rt.pdfPx == width && rt.pdfImage != nil) {
		return
	}
	rt.pdfRendering = true
	doc, gen := rt.pdf, rt.pdfGen
	done := func(img *image.RGBA, err error) {
		rt.pdfRendering = false
		if err != nil {
			rt.pdfErr = err
			return
		}
		rt.pdfImage, rt.pdfPage, rt.pdfPx = ui.NewBitmap(img), page, width
	}
	if d.win == nil {
		done(doc.Render(page, width))
		return
	}
	go func() {
		img, err := doc.Render(page, width)
		d.win.Update(func() {
			if rt.pdfGen == gen {
				done(img, err)
			}
		})
	}()
}

// pdfBody shows a page of a PDF, crisp at any zoom, with the pages to turn
// below it.
func (d *deck) pdfBody(c *ui.Context, n *block) {
	t := c.Theme()
	rt := d.ensureRuntime(n)
	d.openPDFDoc(n, rt)
	switch {
	case rt.pdfErr != nil:
		bodyMessage(c, rt.pdfErr.Error(), t.Danger)
		return
	case rt.pdf == nil:
		bodyMessage(c, "加载中…", t.TextMuted)
		return
	}
	pages := rt.pdf.Pages()
	n.Page = min(max(n.Page, 0), pages-1)
	s := d.cam.S
	d.renderPDFPage(rt, n.Page, pdfWidth(n.W*s))

	ui.Column(c).Grow(1).FillWidth().Background(pdfBackdrop(t)).Children(func() {
		ui.Box(c).Grow(1).FillWidth().Padding(8 * s).Children(func() {
			if rt.pdfImage != nil {
				ui.Image(c, rt.pdfImage).Fill().Fit(ui.Contain)
			}
		})
		if pages > 1 {
			d.pdfPager(c, n, pages)
		}
	})
}

// pdfPager turns the pages of a PDF: ‹ 3 / 12 ›, under the page.
func (d *deck) pdfPager(c *ui.Context, n *block, pages int) {
	t := c.Theme()
	s := min(d.cam.S, 1.5)
	ui.Row(c).FillWidth().Padding(0, 0, 8*s, 0).Justify(ui.Center).Children(func() {
		ui.Row(c).Gap(2*s).AlignItems(ui.Center).Padding(2*s).
			Background(t.Surface.Alpha(0.92)).Border(1, t.Border).Children(func() {
			d.pageTurn(c, n, "pdf-prev:", "‹", "上一页", n.Page-1, pages, s)
			ui.Text(c, fmt.Sprintf("%d / %d", n.Page+1, pages)).FontSize(12*s).TextColor(t.TextMuted).
				SingleLine().Padding(0, 4*s)
			d.pageTurn(c, n, "pdf-next:", "›", "下一页", n.Page+1, pages, s)
		})
	})
}

// pageTurn is a button that turns to page to. It takes its presses itself,
// as the canvas's bodies do: the block around it would take them first
// from a ui.Button.
func (d *deck) pageTurn(c *ui.Context, n *block, key, glyph, label string, to, pages int, s float32) {
	t := c.Theme()
	can := to >= 0 && to < pages
	el := ui.Box(c.Key(key+n.ID)).Padding(1*s, 9*s).Label(label)
	ink := t.TextMuted.Alpha(0.35)
	if can {
		ink = t.Text
		el = el.Cursor(ui.CursorPointer)
		if el.Hovered() {
			el = el.Background(t.SurfaceHover)
		}
	}
	el.HandleInput(func(ev ui.InputEvent) bool {
		switch ev.Kind {
		case ui.InputPointerDown:
			return ev.Button == 0
		case ui.InputPointerUp:
			if ev.Button == 0 && can {
				n.Page = to
			}
			return true
		}
		return false
	}).Children(func() {
		ui.Text(c, glyph).FontSize(15 * s).TextColor(ink)
	})
}

// pdfBackdrop is what shows around a PDF page, as a reader's gray desk.
func pdfBackdrop(t *ui.Theme) ui.Color {
	if t.Dark {
		return ui.Hex("#2a2b2f")
	}
	return ui.Hex("#ececef")
}

// closePDF lets the block's PDF go, and what opens or renders for it
// meanwhile: shown again, as an undone removal brings it back, it opens
// anew.
func (rt *runtime) closePDF() {
	rt.pdfGen++
	if rt.pdf != nil {
		rt.pdf.Close()
	}
	rt.pdf, rt.pdfErr, rt.pdfImage = nil, nil, nil
	rt.pdfOpening, rt.pdfRendering = false, false
}

// mayBeSeeThrough reports whether a picture can have see-through parts: a
// PNG with an alpha channel or a transparent color, a WebP with alpha, or
// any SVG.
func mayBeSeeThrough(path string, data []byte) bool {
	switch strings.ToLower(filepath.Ext(path)) {
	case ".svg":
		return true
	case ".png":
		// IHDR's color type: 4 is gray and alpha, 6 RGB and alpha.
		if len(data) > 25 && (data[25] == 4 || data[25] == 6) {
			return true
		}
		// Else a tRNS chunk, before the image data, makes a color clear.
		head := data
		if i := bytes.Index(data, []byte("IDAT")); i >= 0 {
			head = data[:i]
		}
		return bytes.Contains(head, []byte("tRNS"))
	case ".webp":
		// An extended WebP's flags say alpha; a lossless one may have it.
		if len(data) > 20 && string(data[12:16]) == "VP8X" {
			return data[20]&0x10 != 0
		}
		return len(data) > 16 && string(data[12:16]) == "VP8L"
	}
	return false
}
