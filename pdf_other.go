//go:build !darwin

package main

import (
	"errors"
	"image"
)

// pdfDoc is an open PDF: only macOS renders them (pdf_darwin.go).
type pdfDoc struct{}

func openPDF(string) (*pdfDoc, error) { return nil, errors.New("PDF 预览只在 macOS 上可用") }
func (*pdfDoc) Pages() int            { return 0 }
func (*pdfDoc) PageSize(int) (float64, float64, error) {
	return 0, 0, errors.New("PDF 预览只在 macOS 上可用")
}
func (*pdfDoc) Render(int, int) (*image.RGBA, error) {
	return nil, errors.New("PDF 预览只在 macOS 上可用")
}
func (*pdfDoc) Close() {}
