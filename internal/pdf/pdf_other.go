//go:build !darwin

package pdf

import (
	"errors"
	"image"
)

// Doc is an open PDF: only macOS renders them (pdf_darwin.go).
type Doc struct{}

func Open(string) (*Doc, error) { return nil, errors.New("PDF 预览只在 macOS 上可用") }
func (*Doc) Pages() int         { return 0 }
func (*Doc) PageSize(int) (float64, float64, error) {
	return 0, 0, errors.New("PDF 预览只在 macOS 上可用")
}
func (*Doc) Render(int, int) (*image.RGBA, error) {
	return nil, errors.New("PDF 预览只在 macOS 上可用")
}
func (*Doc) Close() {}
