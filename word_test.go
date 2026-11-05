package main

import (
	"archive/zip"
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

const testDocument = `<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="w" xmlns:r="r" xmlns:a="a">
<w:body>
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>季度计划</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>目标</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">这是 </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>重点</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>内容</w:t></w:r><w:r><w:t xml:space="preserve">，和 </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>斜体</w:t></w:r><w:r><w:rPr><w:b w:val="0"/></w:rPr><w:t>。2*3 [x]</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>第一项</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>子项</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>步骤一</w:t></w:r></w:p>
<w:p><w:hyperlink r:id="rLink"><w:r><w:t>官网</w:t></w:r></w:hyperlink></w:p>
<w:tbl>
<w:tr><w:tc><w:p><w:r><w:t>名称</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>数量</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>a|b</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>3</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
<w:p><w:r><w:drawing><a:blip r:embed="rPic"/></w:drawing></w:r></w:p>
</w:body>
</w:document>`

const testRels = `<?xml version="1.0" encoding="UTF-8"?>
<Relationships>
<Relationship Id="rLink" Target="https://example.com" TargetMode="External"/>
<Relationship Id="rPic" Target="media/image1.png"/>
</Relationships>`

const testNumbering = `<?xml version="1.0" encoding="UTF-8"?>
<w:numbering xmlns:w="w">
<w:abstractNum w:abstractNumId="10"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
<w:abstractNum w:abstractNumId="20"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="10"/></w:num>
<w:num w:numId="2"><w:abstractNumId w:val="20"/></w:num>
</w:numbering>`

func testDocx(t *testing.T) []byte {
	t.Helper()
	var b bytes.Buffer
	z := zip.NewWriter(&b)
	for name, body := range map[string]string{
		"word/document.xml":            testDocument,
		"word/_rels/document.xml.rels": testRels,
		"word/numbering.xml":           testNumbering,
		"word/media/image1.png":        "PNGDATA",
	} {
		w, _ := z.Create(name)
		w.Write([]byte(body))
	}
	z.Close()
	return b.Bytes()
}

func TestADocxReadsAsMarkdown(t *testing.T) {
	doc, err := readDocx(testDocx(t))
	if err != nil {
		t.Fatal(err)
	}
	md := doc.markdown
	for _, want := range []string{
		"# 季度计划\n",
		"## 目标\n",
		"这是 **重点内容**，和 *斜体*。2\\*3 &#91;x&#93;",
		"- 第一项\n  - 子项\n",
		"1. 步骤一\n",
		"[官网](https://example.com)",
		"| 名称 | 数量 |\n| --- | --- |\n| a\\|b | 3 |",
		"![](" + wordMedia + "word/media/image1.png)",
	} {
		if !strings.Contains(md, want) {
			t.Errorf("markdown lacks %q:\n%s", want, md)
		}
	}
	if string(doc.media["word/media/image1.png"]) != "PNGDATA" {
		t.Error("the picture's bytes were not kept")
	}
}

func TestNotAWordDocument(t *testing.T) {
	if _, err := readDocx([]byte("plain text")); err == nil {
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
		doc, err := readWord(out)
		if err != nil {
			t.Fatalf(".%s: %v", format, err)
		}
		for _, want := range []string{"# 会议纪要\n", "**预算**", "- 第一点\n", "- 第二点\n"} {
			if !strings.Contains(doc.markdown, want) {
				t.Errorf(".%s lacks %q:\n%s", format, want, doc.markdown)
			}
		}
	}
}
