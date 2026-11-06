package pdf

import (
	"image/color"
	"os"
	"path/filepath"
	goruntime "runtime"
	"testing"

	"boundlessdeck/internal/pdf/pdftest"
)

func TestAPDFRendersItsPagesUpright(t *testing.T) {
	if goruntime.GOOS != "darwin" {
		t.Skip("PDFs render on macOS")
	}
	doc, err := Open(pdftest.Write(t, 200, 300, 2))
	if err != nil {
		t.Fatal(err)
	}
	defer doc.Close()
	if doc.Pages() != 2 {
		t.Fatalf("%d pages, want 2", doc.Pages())
	}
	if w, h, _ := doc.PageSize(0); w != 200 || h != 300 {
		t.Fatalf("page 1 is %v×%v, want 200×300", w, h)
	}
	if w, h, _ := doc.PageSize(1); w != 300 || h != 200 {
		t.Fatalf("page 2, turned, is %v×%v, want 300×200", w, h)
	}

	img, err := doc.Render(0, 400) // twice its size
	if err != nil {
		t.Fatal(err)
	}
	if b := img.Bounds(); b.Dx() != 400 || b.Dy() != 600 {
		t.Fatalf("rendered %v, want 400×600", b)
	}
	red := color.RGBA{255, 0, 0, 255}
	white := color.RGBA{255, 255, 255, 255}
	// The square is 40..120 from the left, and from the bottom: 480..560
	// from the top.
	if c := img.RGBAAt(80, 520); !near(c, red) {
		t.Fatalf("the square shows %v, want red", c)
	}
	if c := img.RGBAAt(300, 100); c != white {
		t.Fatalf("the page shows %v, want white", c)
	}

	// Turned 90° clockwise, the bottom-left corner goes to the top left.
	img, err = doc.Render(1, 300)
	if err != nil {
		t.Fatal(err)
	}
	if b := img.Bounds(); b.Dx() != 300 || b.Dy() != 200 {
		t.Fatalf("turned page rendered %v, want 300×200", b)
	}
	if c := img.RGBAAt(40, 40); !near(c, red) {
		t.Fatalf("turned, the square shows %v at the top left, want red", c)
	}
	if c := img.RGBAAt(40, 160); c != white {
		t.Fatalf("turned, the bottom left shows %v, want white", c)
	}

	if _, err := doc.Render(2, 100); err == nil {
		t.Fatal("rendered a page it does not have")
	}
}

func TestNotAPDFDoesNotOpen(t *testing.T) {
	if goruntime.GOOS != "darwin" {
		t.Skip("PDFs render on macOS")
	}
	path := filepath.Join(t.TempDir(), "x.pdf")
	os.WriteFile(path, []byte("not a pdf"), 0o644)
	if _, err := Open(path); err == nil {
		t.Fatal("opened a file that is no PDF")
	}
}

func near(a, b color.RGBA) bool {
	d := func(x, y uint8) int {
		if x > y {
			return int(x - y)
		}
		return int(y - x)
	}
	return d(a.R, b.R) < 8 && d(a.G, b.G) < 8 && d(a.B, b.B) < 8 && d(a.A, b.A) < 8
}
