package main

import (
	"bytes"
	"fmt"
	"image/color"
	"os"
	"path/filepath"
	goruntime "runtime"
	"testing"
)

// writeTestPDF writes a PDF of pages w×h points, each a red square 20
// points from its bottom-left corner, the second page turned 90°.
func writeTestPDF(t *testing.T, w, h float64, pages int) string {
	t.Helper()
	var b bytes.Buffer
	var offsets []int
	obj := func(s string) {
		offsets = append(offsets, b.Len())
		fmt.Fprintf(&b, "%d 0 obj\n%s\nendobj\n", len(offsets), s)
	}
	b.WriteString("%PDF-1.4\n")
	kids := ""
	for i := range pages {
		kids += fmt.Sprintf("%d 0 R ", 3+2*i)
	}
	obj("<< /Type /Catalog /Pages 2 0 R >>")
	obj(fmt.Sprintf("<< /Type /Pages /Kids [%s] /Count %d >>", kids, pages))
	for i := range pages {
		rotate := ""
		if i == 1 {
			rotate = "/Rotate 90"
		}
		obj(fmt.Sprintf("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 %g %g] %s /Contents %d 0 R >>", w, h, rotate, 4+2*i))
		content := "1 0 0 rg 20 20 40 40 re f"
		obj(fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(content), content))
	}
	xref := b.Len()
	fmt.Fprintf(&b, "xref\n0 %d\n0000000000 65535 f \n", len(offsets)+1)
	for _, o := range offsets {
		fmt.Fprintf(&b, "%010d 00000 n \n", o)
	}
	fmt.Fprintf(&b, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(offsets)+1, xref)
	path := filepath.Join(t.TempDir(), "test.pdf")
	if err := os.WriteFile(path, b.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestAPDFRendersItsPagesUpright(t *testing.T) {
	if goruntime.GOOS != "darwin" {
		t.Skip("PDFs render on macOS")
	}
	doc, err := openPDF(writeTestPDF(t, 200, 300, 2))
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
	if _, err := openPDF(path); err == nil {
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
