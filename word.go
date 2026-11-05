package main

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// Word documents preview as Markdown, which the canvas reads as it reads
// a .md file: a .docx's text, headings, bold and italic, lists, links,
// tables and pictures, read from its XML; a .doc is turned into a .docx
// first, by macOS's textutil.

// wordDoc is a Word document read for its preview: its Markdown, and the
// pictures it shows, by the names the Markdown gives them.
type wordDoc struct {
	markdown string
	media    map[string][]byte
}

// wordMedia is the scheme of the pictures' names in a wordDoc's Markdown.
const wordMedia = "docx-media:"

// readWord reads a .docx, or a .doc by way of textutil, for its preview.
func readWord(file string) (*wordDoc, error) {
	if strings.EqualFold(filepath.Ext(file), ".doc") {
		return readDoc(file)
	}
	data, err := os.ReadFile(file)
	if err != nil {
		return nil, err
	}
	return readDocx(data)
}

// readDoc turns a .doc into a .docx with textutil, and reads that.
func readDoc(file string) (*wordDoc, error) {
	if _, err := exec.LookPath("textutil"); err != nil {
		return nil, errors.New(".doc 预览需要 macOS 的 textutil")
	}
	dir, err := os.MkdirTemp("", "boundlessdeck-doc-")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(dir)
	out := filepath.Join(dir, "doc.docx")
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if msg, err := exec.CommandContext(ctx, "textutil", "-convert", "docx", "-output", out, file).CombinedOutput(); err != nil {
		return nil, fmt.Errorf("无法读取这个 .doc：%s", strings.TrimSpace(string(msg)))
	}
	data, err := os.ReadFile(out)
	if err != nil {
		return nil, err
	}
	return readDocx(data)
}

// maxDocxPart bounds what a part of a .docx unpacks to.
const maxDocxPart = 64 << 20

func readDocx(data []byte) (*wordDoc, error) {
	zr, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return nil, errors.New("不是可以打开的 Word 文档")
	}
	part := func(name string) ([]byte, error) {
		f, err := zr.Open(name)
		if err != nil {
			return nil, err
		}
		defer f.Close()
		b, err := io.ReadAll(io.LimitReader(f, maxDocxPart+1))
		if len(b) > maxDocxPart {
			return nil, errors.New("Word 文档太大")
		}
		return b, err
	}
	body, err := part("word/document.xml")
	if err != nil {
		return nil, errors.New("不是可以打开的 Word 文档")
	}
	// What its relationships point at: links and pictures.
	targets := map[string]string{}
	if rels, err := part("word/_rels/document.xml.rels"); err == nil {
		var r struct {
			Rels []struct {
				ID     string `xml:"Id,attr"`
				Target string `xml:"Target,attr"`
				Mode   string `xml:"TargetMode,attr"`
			} `xml:"Relationship"`
		}
		if xml.Unmarshal(rels, &r) == nil {
			for _, rel := range r.Rels {
				t := rel.Target
				if rel.Mode != "External" {
					t = path.Clean(path.Join("word", t))
				}
				targets[rel.ID] = t
			}
		}
	}
	doc := &wordDoc{media: map[string][]byte{}}
	picture := func(id string) string {
		name, ok := targets[id]
		if !ok {
			return ""
		}
		if _, ok := doc.media[name]; !ok {
			b, err := part(name)
			if err != nil {
				return ""
			}
			doc.media[name] = b
		}
		return wordMedia + name
	}
	numbered := map[string]map[int]bool{}
	if nb, err := part("word/numbering.xml"); err == nil {
		numbered = docxNumbering(nb)
	}
	md, err := docxMarkdown(body, targets, picture, func(num string, lvl int) bool { return numbered[num][lvl] })
	if err != nil {
		return nil, err
	}
	doc.markdown = md
	return doc, nil
}

// wordRun is text of one style within a paragraph.
type wordRun struct {
	text         string
	bold, italic bool
	link         string
	image        string // a picture's name, for a run that is one
	size         int    // in half points; 0 where the style says
}

// sizedHeading is the level of heading a paragraph styled by hand reads
// as, as textutil and Pages write headings: all of it bold, at 16 points
// or more. 0 for others.
func sizedHeading(runs []wordRun) int {
	size, text := 0, false
	for _, r := range runs {
		if strings.TrimSpace(r.text) == "" {
			continue
		}
		if !r.bold || r.size == 0 {
			return 0
		}
		size, text = max(size, r.size), true
	}
	switch {
	case !text:
		return 0
	case size >= 40:
		return 1
	case size >= 32:
		return 2
	}
	return 0
}

// docxMarkdown turns a .docx's document.xml into Markdown.
func docxMarkdown(body []byte, targets map[string]string, picture func(id string) string, isNumbered func(num string, lvl int) bool) (string, error) {
	dec := xml.NewDecoder(bytes.NewReader(body))
	var out strings.Builder

	// The paragraph being read.
	var (
		runs          []wordRun
		style         string
		listLevel     = -1
		bold, italic  bool
		inRun, inText bool
		link          string
		inRunStyle    bool
		inParaStyle   bool
		listNum       string
		size          int // of the run, in half points
		tableDepth    int
		row           []string
		rows          [][]string
		cell          strings.Builder
		inCell        bool
	)
	flushParagraph := func() {
		text := wordRunsMarkdown(runs)
		paraRuns := runs
		runs = nil
		defer func() { style, listLevel, listNum = "", -1, "" }()
		if inCell {
			if strings.TrimSpace(text) != "" {
				if cell.Len() > 0 {
					cell.WriteString("<br>")
				}
				cell.WriteString(text) // its | escaped, as all text
			}
			return
		}
		if strings.TrimSpace(text) == "" {
			return
		}
		level := headingLevel(style)
		if level == 0 {
			level = sizedHeading(paraRuns)
		}
		// A bullet typed as text, as textutil and Pages write them.
		if bullet, ok := strings.CutPrefix(text, "•"); ok && listLevel < 0 {
			text, listLevel = strings.TrimSpace(bullet), 0
		}
		switch {
		case level > 0:
			text = strings.TrimSuffix(strings.TrimPrefix(text, "**"), "**")
			out.WriteString(strings.Repeat("#", level) + " " + text + "\n\n")
		case listLevel >= 0:
			marker := "- "
			if isNumbered(listNum, listLevel) {
				marker = "1. "
			}
			out.WriteString(strings.Repeat("  ", listLevel) + marker + text + "\n")
		default:
			out.WriteString(text + "\n\n")
		}
	}
	flushTable := func() {
		if len(rows) == 0 {
			return
		}
		cols := 0
		for _, r := range rows {
			cols = max(cols, len(r))
		}
		line := func(r []string) {
			out.WriteString("|")
			for i := range cols {
				c := ""
				if i < len(r) {
					c = r[i]
				}
				out.WriteString(" " + c + " |")
			}
			out.WriteString("\n")
		}
		line(rows[0])
		out.WriteString("|" + strings.Repeat(" --- |", cols) + "\n")
		for _, r := range rows[1:] {
			line(r)
		}
		out.WriteString("\n")
		rows = nil
	}

	for {
		tok, err := dec.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return "", errors.New("Word 文档的内容读不懂")
		}
		switch el := tok.(type) {
		case xml.StartElement:
			switch el.Name.Local {
			case "tbl":
				tableDepth++
			case "tr":
				row = nil
			case "tc":
				inCell, cell = true, strings.Builder{}
			case "pPr":
				inParaStyle = true
			case "pStyle":
				if inParaStyle {
					style = attr(el, "val")
				}
			case "ilvl":
				if inParaStyle {
					if n, err := strconv.Atoi(attr(el, "val")); err == nil {
						listLevel = n
					}
				}
			case "numPr":
				if inParaStyle && listLevel < 0 {
					listLevel = 0
				}
			case "numId":
				if inParaStyle {
					listNum = attr(el, "val")
				}
			case "hyperlink":
				link = targets[attr(el, "id")]
			case "r":
				inRun, bold, italic, size = true, false, false, 0
			case "rPr":
				inRunStyle = true
			case "b":
				if inRunStyle {
					bold = isOn(el)
				}
			case "i":
				if inRunStyle {
					italic = isOn(el)
				}
			case "sz":
				if inRunStyle {
					size, _ = strconv.Atoi(attr(el, "val"))
				}
			case "t":
				inText = inRun
			case "tab":
				if inRun {
					runs = append(runs, wordRun{text: " ", bold: bold, italic: italic, link: link})
				}
			case "br", "cr":
				if inRun {
					runs = append(runs, wordRun{text: "\n", bold: bold, italic: italic})
				}
			case "blip":
				if name := picture(attr(el, "embed")); name != "" {
					runs = append(runs, wordRun{image: name})
				}
			}
		case xml.EndElement:
			switch el.Name.Local {
			case "p":
				flushParagraph()
			case "pPr":
				inParaStyle = false
			case "rPr":
				inRunStyle = false
			case "r":
				inRun = false
			case "t":
				inText = false
			case "hyperlink":
				link = ""
			case "tc":
				row = append(row, strings.TrimSpace(cell.String()))
				inCell = false
			case "tr":
				rows = append(rows, row)
			case "tbl":
				tableDepth--
				if tableDepth == 0 {
					flushTable()
				}
			}
		case xml.CharData:
			if inText {
				runs = append(runs, wordRun{text: string(el), bold: bold, italic: italic, link: link, size: size})
			}
		}
	}
	return strings.TrimSpace(out.String()) + "\n", nil
}

// wordRunsMarkdown writes a paragraph's runs as Markdown, the neighbors of
// one style together, so that "**a****b**" reads "**ab**".
func wordRunsMarkdown(runs []wordRun) string {
	var b strings.Builder
	for i := 0; i < len(runs); {
		r := runs[i]
		if r.image != "" {
			b.WriteString("![](" + r.image + ")")
			i++
			continue
		}
		j, text := i, ""
		for j < len(runs) && runs[j].image == "" && runs[j].bold == r.bold && runs[j].italic == r.italic && runs[j].link == r.link {
			text += runs[j].text
			j++
		}
		i = j
		// Line breaks within a paragraph, as Markdown's hard breaks.
		lines := strings.Split(text, "\n")
		for k, line := range lines {
			if k > 0 {
				b.WriteString("  \n")
			}
			b.WriteString(styled(escapeMarkdown(line), r))
		}
	}
	return strings.TrimSpace(b.String())
}

// styled marks text bold, italic or linked as r is, keeping the spaces
// around it outside the marks, where Markdown needs them.
func styled(text string, r wordRun) string {
	core := strings.TrimSpace(text)
	if core == "" {
		return text
	}
	lead := text[:strings.Index(text, core)]
	trail := text[len(lead)+len(core):]
	switch {
	case r.bold && r.italic:
		core = "***" + core + "***"
	case r.bold:
		core = "**" + core + "**"
	case r.italic:
		core = "*" + core + "*"
	}
	if r.link != "" && !strings.HasPrefix(r.link, "word/") {
		core = "[" + core + "](" + r.link + ")"
	}
	return lead + core + trail
}

// escapeMarkdown keeps text from reading as Markdown syntax. Brackets and
// dollars go as character references: escaped with \, \[x\] and $x$
// would read as math.
func escapeMarkdown(s string) string {
	var b strings.Builder
	for i, r := range s {
		switch r {
		case '[':
			b.WriteString("&#91;")
			continue
		case ']':
			b.WriteString("&#93;")
			continue
		case '$':
			b.WriteString("&#36;")
			continue
		case '\\', '`', '*', '_', '<', '>', '|':
			b.WriteByte('\\')
		case '#', '-', '+':
			if strings.TrimSpace(s[:i]) == "" {
				b.WriteByte('\\')
			}
		}
		b.WriteRune(r)
	}
	return b.String()
}

// docxNumbering reads numbering.xml: which levels of which lists count,
// 1. 2. 3., rather than showing bullets.
func docxNumbering(data []byte) map[string]map[int]bool {
	var n struct {
		Abstract []struct {
			ID     string `xml:"abstractNumId,attr"`
			Levels []struct {
				Level  int `xml:"ilvl,attr"`
				Format struct {
					Val string `xml:"val,attr"`
				} `xml:"numFmt"`
			} `xml:"lvl"`
		} `xml:"abstractNum"`
		Nums []struct {
			ID       string `xml:"numId,attr"`
			Abstract struct {
				Val string `xml:"val,attr"`
			} `xml:"abstractNumId"`
		} `xml:"num"`
	}
	out := map[string]map[int]bool{}
	if xml.Unmarshal(data, &n) != nil {
		return out
	}
	abstract := map[string]map[int]bool{}
	for _, a := range n.Abstract {
		levels := map[int]bool{}
		for _, l := range a.Levels {
			levels[l.Level] = l.Format.Val != "" && l.Format.Val != "bullet" && l.Format.Val != "none"
		}
		abstract[a.ID] = levels
	}
	for _, num := range n.Nums {
		out[num.ID] = abstract[num.Abstract.Val]
	}
	return out
}

// headingLevel is the level of a heading paragraph style, 0 for others.
func headingLevel(style string) int {
	s := strings.ToLower(style)
	switch {
	case s == "title":
		return 1
	case s == "subtitle":
		return 2
	case strings.HasPrefix(s, "heading"):
		if n, err := strconv.Atoi(strings.TrimPrefix(s, "heading")); err == nil && n >= 1 && n <= 6 {
			return n
		}
	}
	return 0
}

func attr(el xml.StartElement, name string) string {
	for _, a := range el.Attr {
		if a.Name.Local == name {
			return a.Value
		}
	}
	return ""
}

// isOn reads a toggle of a run's style, as <w:b/> or <w:b w:val="0"/>.
func isOn(el xml.StartElement) bool {
	switch attr(el, "val") {
	case "0", "false", "off", "none":
		return false
	}
	return true
}
