package word

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"boundlessdeck/internal/word/wordtest"
)

func TestADocxReadsAsMarkdown(t *testing.T) {
	doc, err := ReadDocx(wordtest.Docx())
	if err != nil {
		t.Fatal(err)
	}
	md := doc.Markdown
	for _, want := range []string{
		"# 季度计划\n",
		"## 目标\n",
		"这是 **重点内容**，和 *斜体*。2\\*3 &#91;x&#93;",
		"- 第一项\n  - 子项\n",
		"1. 步骤一\n",
		"[官网](https://example.com)",
		"| 名称 | 数量 |\n| --- | --- |\n| a\\|b | 3 |",
		"![](" + MediaScheme + "word/media/image1.png)",
	} {
		if !strings.Contains(md, want) {
			t.Errorf("markdown lacks %q:\n%s", want, md)
		}
	}
	if string(doc.Media["word/media/image1.png"]) != "PNGDATA" {
		t.Error("the picture's bytes were not kept")
	}
}

func TestNotAWordDocument(t *testing.T) {
	if _, err := ReadDocx([]byte("plain text")); err == nil {
		t.Fatal("read a file that is no .docx")
	}
}

// A .doc and a .docx that macOS itself writes read as Markdown.
func TestWordFilesMacOSWrites(t *testing.T) {
	if _, err := exec.LookPath("textutil"); err != nil {
		t.Skip("no textutil")
	}
	dir := t.TempDir()
	html := filepath.Join(dir, "in.html")
	os.WriteFile(html, []byte(`<html><head><meta charset="utf-8"></head><body><h1>会议纪要</h1><p>今天讨论了 <b>预算</b>。</p><ul><li>第一点</li><li>第二点</li></ul></body></html>`), 0o644)
	for _, format := range []string{"doc", "docx"} {
		out := filepath.Join(dir, "out."+format)
		if msg, err := exec.Command("textutil", "-convert", format, "-output", out, html).CombinedOutput(); err != nil {
			t.Fatalf("textutil: %s", msg)
		}
		doc, err := Read(out)
		if err != nil {
			t.Fatalf(".%s: %v", format, err)
		}
		for _, want := range []string{"# 会议纪要\n", "**预算**", "- 第一点\n", "- 第二点\n"} {
			if !strings.Contains(doc.Markdown, want) {
				t.Errorf(".%s lacks %q:\n%s", format, want, doc.Markdown)
			}
		}
	}
}
