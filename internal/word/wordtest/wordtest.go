// Package wordtest holds a Word document for tests that read one.
package wordtest

import (
	"archive/zip"
	"bytes"
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

// Docx is a small .docx with headings, emphasis, lists, a link, a table
// and a picture (word/media/image1.png, whose bytes are "PNGDATA").
func Docx() []byte {
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
