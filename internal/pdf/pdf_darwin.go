//go:build darwin

// Package pdf renders the pages of PDF files: by CoreGraphics, through
// purego (no cgo), on macOS; elsewhere Open reports it cannot.
package pdf

import (
	"errors"
	"image"
	"math"
	"sync"
	"unsafe"

	"github.com/ebitengine/purego"
)

// PDFs render with CoreGraphics, as Preview and Quick Look draw them,
// called through purego: no cgo, nothing to install.

// cgRect is a CGRect: its origin and size, in PDF points.
type cgRect struct{ X, Y, W, H float64 }

var cg struct {
	once sync.Once
	err  error

	urlFromPath   func(alloc uintptr, buf *byte, n int, dir bool) uintptr
	release       func(ref uintptr)
	docWithURL    func(url uintptr) uintptr
	docPages      func(doc uintptr) uintptr
	docPage       func(doc uintptr, n uintptr) uintptr
	docRelease    func(doc uintptr)
	pageBox       func(page uintptr, box int32) cgRect
	pageRotation  func(page uintptr) int32
	rgb           func() uintptr
	spaceRelease  func(space uintptr)
	bitmapContext func(data uintptr, w, h, bpc, bpr uintptr, space uintptr, info uint32) uintptr
	bitmapData    func(ctx uintptr) unsafe.Pointer
	ctxRelease    func(ctx uintptr)
	fillColor     func(ctx uintptr, r, g, b, a float64)
	fillRect      func(ctx uintptr, r cgRect)
	translate     func(ctx uintptr, x, y float64)
	scale         func(ctx uintptr, x, y float64)
	rotate        func(ctx uintptr, angle float64)
	drawPage      func(ctx uintptr, page uintptr)
}

func loadCG() error {
	cg.once.Do(func() {
		coreFoundation, err := purego.Dlopen("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation", purego.RTLD_NOW|purego.RTLD_GLOBAL)
		if err != nil {
			cg.err = err
			return
		}
		coreGraphics, err := purego.Dlopen("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics", purego.RTLD_NOW|purego.RTLD_GLOBAL)
		if err != nil {
			cg.err = err
			return
		}
		purego.RegisterLibFunc(&cg.urlFromPath, coreFoundation, "CFURLCreateFromFileSystemRepresentation")
		purego.RegisterLibFunc(&cg.release, coreFoundation, "CFRelease")
		purego.RegisterLibFunc(&cg.docWithURL, coreGraphics, "CGPDFDocumentCreateWithURL")
		purego.RegisterLibFunc(&cg.docPages, coreGraphics, "CGPDFDocumentGetNumberOfPages")
		purego.RegisterLibFunc(&cg.docPage, coreGraphics, "CGPDFDocumentGetPage")
		purego.RegisterLibFunc(&cg.docRelease, coreGraphics, "CGPDFDocumentRelease")
		purego.RegisterLibFunc(&cg.pageBox, coreGraphics, "CGPDFPageGetBoxRect")
		purego.RegisterLibFunc(&cg.pageRotation, coreGraphics, "CGPDFPageGetRotationAngle")
		purego.RegisterLibFunc(&cg.rgb, coreGraphics, "CGColorSpaceCreateDeviceRGB")
		purego.RegisterLibFunc(&cg.spaceRelease, coreGraphics, "CGColorSpaceRelease")
		purego.RegisterLibFunc(&cg.bitmapContext, coreGraphics, "CGBitmapContextCreate")
		purego.RegisterLibFunc(&cg.bitmapData, coreGraphics, "CGBitmapContextGetData")
		purego.RegisterLibFunc(&cg.ctxRelease, coreGraphics, "CGContextRelease")
		purego.RegisterLibFunc(&cg.fillColor, coreGraphics, "CGContextSetRGBFillColor")
		purego.RegisterLibFunc(&cg.fillRect, coreGraphics, "CGContextFillRect")
		purego.RegisterLibFunc(&cg.translate, coreGraphics, "CGContextTranslateCTM")
		purego.RegisterLibFunc(&cg.scale, coreGraphics, "CGContextScaleCTM")
		purego.RegisterLibFunc(&cg.rotate, coreGraphics, "CGContextRotateCTM")
		purego.RegisterLibFunc(&cg.drawPage, coreGraphics, "CGContextDrawPDFPage")
	})
	return cg.err
}

// Doc is an open PDF. Its methods are safe from any goroutine, one at
// a time each: CoreGraphics does not draw a document's pages on two
// threads at once.
type Doc struct {
	mu    sync.Mutex
	ref   uintptr
	pages int
}

var errNotPDF = errors.New("不是可以打开的 PDF")

// Open opens the PDF at path.
func Open(path string) (*Doc, error) {
	if err := loadCG(); err != nil {
		return nil, err
	}
	b := []byte(path)
	url := cg.urlFromPath(0, &b[0], len(b), false)
	if url == 0 {
		return nil, errNotPDF
	}
	defer cg.release(url)
	ref := cg.docWithURL(url)
	if ref == 0 {
		return nil, errNotPDF
	}
	n := int(cg.docPages(ref))
	if n == 0 {
		cg.docRelease(ref)
		return nil, errors.New("PDF 没有页面")
	}
	return &Doc{ref: ref, pages: n}, nil
}

// Pages is how many pages it has.
func (d *Doc) Pages() int { return d.pages }

// box returns page i's crop box and how far it turns, clockwise, as a
// reader shows it.
func (d *Doc) box(i int) (page uintptr, r cgRect, turn int, err error) {
	if d.ref == 0 {
		return 0, cgRect{}, 0, errors.New("PDF 已关闭")
	}
	if i < 0 || i >= d.pages {
		return 0, cgRect{}, 0, errors.New("没有这一页")
	}
	page = cg.docPage(d.ref, uintptr(i+1))
	if page == 0 {
		return 0, cgRect{}, 0, errors.New("这一页打不开")
	}
	const cropBox = 1
	r = cg.pageBox(page, cropBox)
	turn = ((int(cg.pageRotation(page)) % 360) + 360) % 360
	return page, r, turn, nil
}

// PageSize returns page i's size as it shows, in points.
func (d *Doc) PageSize(i int) (w, h float64, err error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	_, r, turn, err := d.box(i)
	if err != nil {
		return 0, 0, err
	}
	if turn == 90 || turn == 270 {
		return r.H, r.W, nil
	}
	return r.W, r.H, nil
}

// maxPagePixels bounds a rendered page, at about 4096×4096.
const maxPagePixels = 1 << 24

// Render draws page i width pixels wide, on white, upright.
func (d *Doc) Render(i, width int) (*image.RGBA, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	page, r, turn, err := d.box(i)
	if err != nil {
		return nil, err
	}
	pw, ph := r.W, r.H
	if turn == 90 || turn == 270 {
		pw, ph = ph, pw
	}
	if pw <= 0 || ph <= 0 {
		return nil, errors.New("这一页是空的")
	}
	width = max(width, 16)
	height := int(math.Ceil(float64(width) * ph / pw))
	if width*height > maxPagePixels {
		k := math.Sqrt(float64(maxPagePixels) / float64(width*height))
		width, height = int(float64(width)*k), int(float64(height)*k)
	}
	s := float64(width) / pw

	space := cg.rgb()
	defer cg.spaceRelease(space)
	// RGBA, premultiplied, the bytes in that order: as image.RGBA keeps
	// them.
	const premultipliedLast, byteOrder32Big = 1, 4 << 12
	ctx := cg.bitmapContext(0, uintptr(width), uintptr(height), 8, uintptr(width*4), space, premultipliedLast|byteOrder32Big)
	if ctx == 0 {
		return nil, errors.New("无法绘制这一页")
	}
	defer cg.ctxRelease(ctx)

	cg.fillColor(ctx, 1, 1, 1, 1)
	cg.fillRect(ctx, cgRect{0, 0, float64(width), float64(height)})
	cg.scale(ctx, s, s)
	// Turned as the page asks, clockwise, into the box (0, 0)–(pw, ph).
	switch turn {
	case 90:
		cg.translate(ctx, 0, ph)
		cg.rotate(ctx, -math.Pi/2)
	case 180:
		cg.translate(ctx, pw, ph)
		cg.rotate(ctx, math.Pi)
	case 270:
		cg.translate(ctx, pw, 0)
		cg.rotate(ctx, math.Pi/2)
	}
	cg.translate(ctx, -r.X, -r.Y)
	cg.drawPage(ctx, page)

	data := cg.bitmapData(ctx)
	if data == nil {
		return nil, errors.New("无法绘制这一页")
	}
	img := image.NewRGBA(image.Rect(0, 0, width, height))
	copy(img.Pix, unsafe.Slice((*byte)(data), width*height*4))
	return img, nil
}

// Close lets the document go.
func (d *Doc) Close() {
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.ref != 0 {
		cg.docRelease(d.ref)
		d.ref = 0
	}
}
