package main

import (
	"bytes"
	"image"
	"image/color"
	"image/jpeg"
	"image/png"
	"os"
	"path/filepath"
	goruntime "runtime"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"

	"boundlessdeck/internal/pdf/pdftest"
	"boundlessdeck/internal/word/wordtest"
)

// writePNG writes a 64×64 PNG, see-through on its left half or not.
func writePNG(t *testing.T, path string, seeThrough bool) {
	t.Helper()
	img := image.NewNRGBA(image.Rect(0, 0, 64, 64))
	for y := range 64 {
		for x := range 64 {
			a := uint8(255)
			if seeThrough && x < 32 {
				a = 0
			}
			img.SetNRGBA(x, y, color.NRGBA{30, 120, 255, a})
		}
	}
	var b bytes.Buffer
	png.Encode(&b, img)
	if err := os.WriteFile(path, b.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

// previewDeck is a deck with a block of each kind of file it previews.
func previewDeck(t *testing.T) *deck {
	t.Helper()
	d := testDeck(t)
	d.blocks = nil
	write := func(name string, data []byte) {
		if err := os.WriteFile(filepath.Join(d.root, name), data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	write("readme.md", []byte("# 项目说明\n\n这是 **Markdown** 预览。\n\n- 第一条\n- 第二条\n\n```go\nfmt.Println(\"hi\")\n```\n"))
	write("plan.docx", wordtest.Docx())
	pdf, _ := os.ReadFile(pdftest.Write(t, 200, 300, 2))
	write("paper.pdf", pdf)
	writePNG(t, filepath.Join(d.root, "logo.png"), true)
	writePNG(t, filepath.Join(d.root, "solid.png"), false)
	var jb bytes.Buffer
	jpeg.Encode(&jb, image.NewRGBA(image.Rect(0, 0, 8, 8)), nil)
	write("photo.jpg", jb.Bytes())
	for i, name := range []string{"readme.md", "plan.docx", "paper.pdf", "logo.png", "solid.png", "photo.jpg"} {
		d.blocks = append(d.blocks, &block{ID: name, Kind: kindFile, Title: name, Path: name,
			X: float32(20 + (i%3)*360), Y: float32(20 + (i/3)*330), W: 340, H: 300})
	}
	return d
}

func TestFilesPreviewByKind(t *testing.T) {
	if goruntime.GOOS != "darwin" {
		t.Skip("PDFs render on macOS")
	}
	holdZoom(t)
	d := previewDeck(t)
	tt := ui.NewTester(d.view, 1100, 700)
	tt.Frame()
	tt.Frame()

	for _, want := range []string{"项目说明", "Markdown", "第二条", "季度计划", "重点内容", "。2*3 [x]", "1 / 2"} {
		if !tt.HasText(want) {
			t.Errorf("the canvas does not show %q", want)
		}
	}
	block := func(id string) *block {
		for _, b := range d.blocks {
			if b.ID == id {
				return b
			}
		}
		t.Fatalf("no block %s", id)
		return nil
	}

	pdf := block("paper.pdf")
	if pdf.rt.pdfImage == nil {
		t.Fatal("the PDF's page did not render")
	}
	if err := tt.Click("下一页"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if pdf.Page != 1 || !tt.HasText("2 / 2") || pdf.rt.pdfPage != 1 {
		t.Fatalf("after next: page %d, rendered %d", pdf.Page, pdf.rt.pdfPage)
	}
	// Zoomed in, the page renders again, sharper, once the zoom rests.
	before := pdf.rt.pdfPx
	tt.Pinch(500, 350, 2)
	tt.Frame()
	if pdf.rt.pdfPx != before {
		t.Fatalf("mid-pinch the page rendered again, %d px wide", pdf.rt.pdfPx)
	}
	d.zoomSettle = time.Time{}
	tt.Frame()
	tt.Frame()
	if pdf.rt.pdfPx <= before {
		t.Fatalf("zoomed in, the page is %d px wide, as before (%d)", pdf.rt.pdfPx, before)
	}

	if !block("logo.png").rt.imgAlpha {
		t.Error("a PNG with see-through parts shows no checkerboard")
	}
	for _, id := range []string{"solid.png", "photo.jpg"} {
		if block(id).rt.imgAlpha {
			t.Errorf("%s, opaque, shows a checkerboard", id)
		}
	}

	// Removed, a PDF block lets its document go.
	d.removeBlock(pdf)
	if pdf.rt.pdf != nil {
		t.Fatal("a removed PDF block keeps its document open")
	}
}
