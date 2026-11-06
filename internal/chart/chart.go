// Package chart reads a small table, one row per category and a column
// per series, and draws it as bars, horizontal bars, a line, an area or a
// pie: the 图表 block's content.
//
// Cells part at tabs (as a spreadsheet copies them), else at commas or
// semicolons; a Markdown table reads too. The first row names the series
// when a cell after its first is no number.
package chart

import (
	"fmt"
	"math"
	"strconv"
	"strings"

	"github.com/egoist/mygo/ui"

	"boundlessdeck/internal/icons"
)

type Type string

const (
	Bar  Type = "bar"
	HBar Type = "hbar"
	Line Type = "line"
	Area Type = "area"
	Pie  Type = "pie"
)

// Types is every chart a block draws, in the order its menu lists
// them; the first is a new block's.
var Types = []struct {
	Type  Type
	Label string
	Icon  *ui.SVG
}{
	{Bar, "柱状图", icons.Chart},
	{HBar, "条形图", icons.ChartBar},
	{Line, "折线图", icons.ChartLine},
	{Area, "面积图", icons.ChartArea},
	{Pie, "饼图", icons.ChartPie},
}

// TypeNames is Types as the MCP names them.
func TypeNames() []string {
	out := make([]string, len(Types))
	for i, ct := range Types {
		out[i] = string(ct.Type)
	}
	return out
}

// Of is the chart a block's Chart field names: Bar when it names none.
func Of(name string) Type {
	for _, ct := range Types {
		if string(ct.Type) == name {
			return ct.Type
		}
	}
	return Bar
}

func Label(t Type) string {
	for _, ct := range Types {
		if ct.Type == t {
			return ct.Label
		}
	}
	return "图表"
}

func Icon(t Type) *ui.SVG {
	for _, ct := range Types {
		if ct.Type == t {
			return ct.Icon
		}
	}
	return icons.Chart
}

// Sample is the table a new 图表 block starts with.
const Sample = "月份, 收入, 支出\n1月, 120, 80\n2月, 150, 95\n3月, 98, 70\n4月, 180, 110\n5月, 165, 120"

// A chart shows at most maxSeries series, each its own hue: a ninth
// would need a hue that is not there. A pie shows maxSeries slices,
// the rest as one. At most maxCats rows are read.
const (
	maxSeries = 8
	maxCats   = 200
)

// Data is a chart's table as read.
type Data struct {
	series []string
	cats   []string
	vals   [][]float64 // [series][category]; NaN where a cell holds no number
	note   string      // what of the table was left out
}

func (cd *Data) Empty() bool {
	if len(cd.cats) == 0 || len(cd.series) == 0 {
		return true
	}
	for _, row := range cd.vals {
		for _, v := range row {
			if !math.IsNaN(v) {
				return false
			}
		}
	}
	return true
}

// Summary is the line a card in a 块堆 says of a chart of type t.
func (cd *Data) Summary(t Type) string {
	if cd.Empty() {
		return Label(t) + " · 还没有数据"
	}
	if len(cd.series) == 1 {
		return fmt.Sprintf("%s · %d 项", Label(t), len(cd.cats))
	}
	return fmt.Sprintf("%s · %d 项 × %d 组", Label(t), len(cd.cats), len(cd.series))
}

// Parse reads a chart's table.
func Parse(src string) *Data {
	cd := &Data{}
	tabs := strings.Contains(src, "\t")
	var rows [][]string
	for _, line := range strings.Split(src, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		var cells []string
		switch {
		case strings.HasPrefix(line, "|"):
			cells = strings.Split(strings.Trim(line, "|"), "|")
			if isRule(cells) {
				continue // a Markdown table's |---|---|
			}
		case tabs:
			cells = strings.Split(line, "\t")
		default:
			// An empty cell stays one: a,,5 has a gap in its middle.
			cells = strings.Split(seps.Replace(line), ",")
		}
		for i := range cells {
			cells[i] = strings.TrimSpace(cells[i])
		}
		if len(cells) > 0 {
			rows = append(rows, cells)
		}
	}
	if len(rows) == 0 {
		return cd
	}
	// The first row names the columns when a cell after its first, or its
	// only one, is no number.
	head, header := rows[0], false
	from := 1
	if len(head) == 1 {
		from = 0
	}
	for _, cell := range head[from:] {
		if _, ok := cellNumber(cell); !ok && cell != "" {
			header = true
		}
	}
	data := rows
	if header {
		data = rows[1:]
	}
	cols := 0
	for _, r := range data {
		cols = max(cols, len(r))
	}
	// Two columns or more: the first names the rows. One: they are numbered.
	labeled := cols >= 2
	nser := 1
	if labeled {
		nser = cols - 1
	}
	if nser > maxSeries {
		cd.note = fmt.Sprintf("只画前 %d 列", maxSeries)
		nser = maxSeries
	}
	if len(data) > maxCats {
		cd.note = fmt.Sprintf("只画前 %d 行", maxCats)
		data = data[:maxCats]
	}
	for i := range nser {
		name := ""
		switch {
		case header && labeled && i+1 < len(head):
			name = head[i+1]
		case header && !labeled:
			name = head[0]
		}
		if name == "" {
			name = "数值"
			if nser > 1 {
				name = fmt.Sprintf("系列 %d", i+1)
			}
		}
		cd.series = append(cd.series, name)
	}
	cd.vals = make([][]float64, nser)
	for i, r := range data {
		cat, cells := strconv.Itoa(i+1), r
		if labeled {
			cat, cells = r[0], r[1:]
		}
		cd.cats = append(cd.cats, cat)
		for si := range nser {
			v := math.NaN()
			if si < len(cells) {
				if f, ok := cellNumber(cells[si]); ok {
					v = f
				}
			}
			cd.vals[si] = append(cd.vals[si], v)
		}
	}
	return cd
}

// seps makes every separator a cell may end at a comma.
var seps = strings.NewReplacer("，", ",", ";", ",", "；", ",")

// isRule says whether cells are a Markdown table's rule: dashes and
// colons alone.
func isRule(cells []string) bool {
	for _, c := range cells {
		if strings.Trim(strings.TrimSpace(c), "-: ") != "" {
			return false
		}
	}
	return true
}

// cellNumber reads a cell as a number: 1,200 and 1200 alike, a currency
// sign before it or % after it left out.
func cellNumber(s string) (float64, bool) {
	s = strings.TrimSpace(s)
	s = strings.TrimLeft(s, "¥￥$€£")
	s = strings.TrimSuffix(s, "%")
	s = strings.ReplaceAll(s, ",", "")
	s = strings.ReplaceAll(s, "_", "")
	if s == "" {
		return 0, false
	}
	f, err := strconv.ParseFloat(s, 64)
	if err != nil || math.IsInf(f, 0) || math.IsNaN(f) {
		return 0, false
	}
	return f, true
}

// The categorical hues, in their fixed order, for the light and the dark
// theme: the same hues, stepped for each surface. Series i always wears
// hue i.
var (
	lightHues = []ui.Color{ui.Hex("#2a78d6"), ui.Hex("#eb6834"), ui.Hex("#1baf7a"), ui.Hex("#eda100"),
		ui.Hex("#e87ba4"), ui.Hex("#008300"), ui.Hex("#6250d6"), ui.Hex("#e34948")}
	darkHues = []ui.Color{ui.Hex("#3987e5"), ui.Hex("#d95926"), ui.Hex("#199e70"), ui.Hex("#c98500"),
		ui.Hex("#d55181"), ui.Hex("#008300"), ui.Hex("#9085e9"), ui.Hex("#e66767")}
)

func hue(i int, dark bool) ui.Color {
	if dark {
		return darkHues[i%len(darkHues)]
	}
	return lightHues[i%len(lightHues)]
}

// niceScale is the axis a chart's values lie on: from lo to hi in steps,
// round numbers; with zero, 0 is on it.
func niceScale(lo, hi float64, zero bool, ticks int) (float64, float64, float64) {
	if zero {
		lo, hi = math.Min(lo, 0), math.Max(hi, 0)
	}
	if hi == lo {
		pad := math.Abs(hi) * 0.5
		if pad == 0 {
			pad = 1
		}
		if zero && lo == 0 {
			hi += pad
		} else {
			lo, hi = lo-pad, hi+pad
		}
	}
	raw := (hi - lo) / float64(max(ticks, 1))
	mag := math.Pow(10, math.Floor(math.Log10(raw)))
	var step float64
	switch f := raw / mag; {
	case f <= 1:
		step = mag
	case f <= 2:
		step = 2 * mag
	case f <= 2.5:
		step = 2.5 * mag
	case f <= 5:
		step = 5 * mag
	default:
		step = 10 * mag
	}
	return math.Floor(lo/step+1e-9) * step, math.Ceil(hi/step-1e-9) * step, step
}

// tickText writes an axis value: in 万 or 亿 on an axis that reaches
// them, as many decimals as its step needs.
func tickText(v, top, step float64) string {
	unit, div := "", 1.0
	switch {
	case math.Abs(top) >= 1e8:
		unit, div = "亿", 1e8
	case math.Abs(top) >= 1e4:
		unit, div = "万", 1e4
	}
	// As many decimals as the step has: 0.25 needs two.
	dec := 0
	for st := step / div; dec < 4 && math.Abs(st-math.Round(st)) > 1e-9*math.Max(1, st); dec++ {
		st *= 10
	}
	if v == 0 {
		return "0"
	}
	return strconv.FormatFloat(v/div, 'f', dec, 64) + unit
}

// valueText writes a value as a tooltip shows it: in full, its thousands
// grouped, at most two decimals.
func valueText(v float64) string {
	if math.IsNaN(v) {
		return "—"
	}
	s := strconv.FormatFloat(math.Round(v*100)/100, 'f', -1, 64)
	sign := ""
	if strings.HasPrefix(s, "-") {
		sign, s = "-", s[1:]
	}
	whole, frac, _ := strings.Cut(s, ".")
	var b strings.Builder
	for i, r := range whole {
		if i > 0 && (len(whole)-i)%3 == 0 {
			b.WriteByte(',')
		}
		b.WriteRune(r)
	}
	if frac != "" {
		return sign + b.String() + "." + frac
	}
	return sign + b.String()
}

// View is a chart as one frame draws it.
type View struct {
	Type Type
	Data *Data
	S    float32 // its scale: 1 at 100%
	T    *ui.Theme
	// Hover is the pointer, in the painter's coordinates, when Hovering.
	Hover    ui.Point
	Hovering bool
	// Compact is a chart small on a 块堆's card: marks alone.
	Compact bool
}

func (v View) span(text string, size float32, color ui.Color) ui.Span {
	return ui.Span{Text: text, Size: size * v.S, Color: color}
}

func (v View) hairline() float32 { return max(1*v.S, 0.5) }

// Draw paints the chart in r.
func (v View) Draw(p *ui.Painter, r ui.Rect) {
	s, t := v.S, v.T
	if v.Data == nil || v.Data.Empty() {
		if !v.Compact {
			msg := v.span("还没有数据 · 双击或点标题栏的表格按钮填写", 12, t.TextMuted)
			w, h := p.MeasureText(r.W-24*s, msg)
			p.RichText(r.X+(r.W-w)/2, r.Y+(r.H-h)/2, r.W-24*s, msg)
		}
		return
	}
	pad := 12 * s
	if v.Compact {
		pad = 4 * s
	}
	in := ui.Rect{X: r.X + pad, Y: r.Y + pad, W: r.W - 2*pad, H: r.H - 2*pad}
	if in.W <= 8*s || in.H <= 8*s {
		return
	}
	p.Clip(r, 0, func() {
		if v.Type == Pie {
			v.drawPie(p, in, r)
			return
		}
		top := in.Y
		if len(v.Data.series) > 1 && !v.Compact {
			items := make([]legendKey, len(v.Data.series))
			for i, name := range v.Data.series {
				items[i] = legendKey{hue(i, t.Dark), name}
			}
			h := v.drawLegend(p, ui.Rect{X: in.X, Y: in.Y, W: in.W, H: in.H / 3}, items)
			top += h + 8*s
		}
		plot := ui.Rect{X: in.X, Y: top, W: in.W, H: in.Y + in.H - top}
		if v.Data.note != "" && !v.Compact {
			note := v.span(v.Data.note, 10, t.TextMuted)
			nw, nh := p.MeasureText(0, note)
			p.RichText(in.X+in.W-nw, in.Y+in.H-nh, 0, note)
			plot.H -= nh + 4*s
		}
		if v.Type == HBar {
			v.drawHBars(p, plot, r)
			return
		}
		v.drawColumns(p, plot, r)
	})
}

// legendKey is one entry of a legend.
type legendKey struct {
	color ui.Color
	name  string
}

// drawLegend lays the keys out in rows from r's top left, and says how
// tall they came out.
func (v View) drawLegend(p *ui.Painter, r ui.Rect, items []legendKey) float32 {
	return v.legend(p, r, items, true)
}

// legend lays the keys out, drawing them when draw says so, and says how
// tall they come out.
func (v View) legend(p *ui.Painter, r ui.Rect, items []legendKey, draw bool) float32 {
	s := v.S
	sw, gap, lineH := 8*s, 14*s, 16*s
	x, y := r.X, r.Y
	for _, it := range items {
		label := v.span(it.name, 11, v.T.TextMuted)
		w, _ := p.MeasureText(0, label)
		w = min(w, r.W-sw-6*s)
		if x > r.X && x+sw+6*s+w > r.X+r.W {
			x, y = r.X, y+lineH
			if y+lineH > r.Y+r.H {
				break
			}
		}
		if draw {
			p.Fill(ui.Rect{X: x, Y: y + (lineH-sw)/2, W: sw, H: sw}, it.color, 2*s)
			p.Clip(ui.Rect{X: x + sw + 6*s, Y: y, W: w, H: lineH}, 0, func() {
				p.RichText(x+sw+6*s, y+1.5*s, 0, label)
			})
		}
		x += sw + 6*s + w + gap
	}
	return y + lineH - r.Y
}

// valueRange is the least and the most of the chart's values.
func (v View) valueRange() (float64, float64) {
	lo, hi := math.Inf(1), math.Inf(-1)
	for _, row := range v.Data.vals {
		for _, x := range row {
			if !math.IsNaN(x) {
				lo, hi = math.Min(lo, x), math.Max(hi, x)
			}
		}
	}
	return lo, hi
}

// drawColumns draws bars standing up, a line or an area: the categories
// along the bottom, the values up the left.
func (v View) drawColumns(p *ui.Painter, plot, bounds ui.Rect) {
	s, t, cd := v.S, v.T, v.Data
	lo, hi := v.valueRange()
	ticks := max(min(int(plot.H/(44*s)), 6), 2)
	vmin, vmax, step := niceScale(lo, hi, v.Type != Line, ticks)

	labelH := float32(0)
	axisW := float32(0)
	if !v.Compact {
		labelH = 18 * s
		for x := vmin; x <= vmax+step/2; x += step {
			w, _ := p.MeasureText(0, v.span(tickText(x, vmax, step), 10.5, t.TextMuted))
			axisW = max(axisW, w)
		}
		axisW += 8 * s
	}
	area := ui.Rect{X: plot.X + axisW, Y: plot.Y + 4*s, W: plot.W - axisW, H: plot.H - labelH - 4*s}
	if area.W <= 4*s || area.H <= 4*s {
		return
	}
	yOf := func(x float64) float32 {
		return area.Y + area.H - float32((x-vmin)/(vmax-vmin))*area.H
	}

	// Gridlines, hairline and recessive, and the axis's values.
	grid := t.Border.Alpha(0.55)
	for x := vmin; x <= vmax+step/2; x += step {
		y := yOf(x)
		p.Line(area.X, y, area.X+area.W, y, v.hairline(), grid)
		if !v.Compact {
			label := v.span(tickText(x, vmax, step), 10.5, t.TextMuted)
			w, h := p.MeasureText(0, label)
			p.RichText(area.X-8*s-w, y-h/2, 0, label)
		}
	}
	base := yOf(math.Max(math.Min(0, vmax), vmin))
	p.Line(area.X, base, area.X+area.W, base, v.hairline(), t.Border)

	n := len(cd.cats)
	band := area.W / float32(n)
	hi0 := -1
	if v.Hovering && v.Hover.X >= area.X && v.Hover.X < area.X+area.W && v.Hover.Y >= plot.Y && v.Hover.Y < plot.Y+plot.H {
		hi0 = min(int((v.Hover.X-area.X)/band), n-1)
		p.Fill(ui.Rect{X: area.X + band*float32(hi0), Y: area.Y, W: band, H: area.H}, t.Text.Alpha(0.045), 3*s)
	}

	// The categories under it, as many as fit side by side.
	if !v.Compact {
		widest := float32(0)
		for _, c := range cd.cats {
			w, _ := p.MeasureText(0, v.span(c, 10.5, t.TextMuted))
			widest = max(widest, w)
		}
		every := max(int(math.Ceil(float64((widest+8*s)/band))), 1)
		for i, c := range cd.cats {
			if i%every != 0 {
				continue
			}
			label := v.span(c, 10.5, t.TextMuted)
			w, _ := p.MeasureText(0, label)
			cx := area.X + band*(float32(i)+0.5)
			p.RichText(cx-w/2, area.Y+area.H+5*s, 0, label)
		}
	}

	nser := len(cd.series)
	switch v.Type {
	case Bar:
		gap := 2 * s
		group := band * 0.72
		w := min(24*s, (group-gap*float32(nser-1))/float32(nser))
		w = max(w, 1)
		group = w*float32(nser) + gap*float32(nser-1)
		for si, row := range cd.vals {
			color := hue(si, t.Dark)
			for i, x := range row {
				if math.IsNaN(x) {
					continue
				}
				bx := area.X + band*float32(i) + (band-group)/2 + float32(si)*(w+gap)
				v.bar(p, ui.Rect{X: bx, Y: min(yOf(x), base), W: w, H: abs(yOf(x) - base)}, x >= 0, false, color)
			}
		}
	case Line, Area:
		for si, row := range cd.vals {
			color := hue(si, t.Dark)
			var run [][2]float32
			flush := func() {
				switch {
				case len(run) == 1:
					p.FillPath(new(ui.Path).Circle(run[0][0], run[0][1], 3*s), color)
				case len(run) > 1:
					if v.Type == Area {
						path := new(ui.Path).MoveTo(run[0][0], base)
						for _, pt := range run {
							path.LineTo(pt[0], pt[1])
						}
						path.LineTo(run[len(run)-1][0], base).Close()
						p.FillPath(path, color.Alpha(0.12))
					}
					p.StrokePolyline(run, 2*s, color)
				}
				run = run[:0]
			}
			for i, x := range row {
				if math.IsNaN(x) {
					flush() // no value: the line breaks
					continue
				}
				run = append(run, [2]float32{area.X + band*(float32(i)+0.5), yOf(x)})
			}
			flush()
		}
		if hi0 >= 0 {
			// The values hovered: a dot on each line, ringed in the surface.
			for si, row := range cd.vals {
				if x := row[hi0]; !math.IsNaN(x) {
					cx, cy := area.X+band*(float32(hi0)+0.5), yOf(x)
					p.FillPath(new(ui.Path).Circle(cx, cy, 6*s), t.Surface)
					p.FillPath(new(ui.Path).Circle(cx, cy, 4*s), hue(si, t.Dark))
				}
			}
		}
	}
	if hi0 >= 0 && !v.Compact {
		v.tooltipAt(p, bounds, hi0)
	}
}

// drawHBars draws bars lying down: the categories down the left, the
// values along the bottom.
func (v View) drawHBars(p *ui.Painter, plot, bounds ui.Rect) {
	s, t, cd := v.S, v.T, v.Data
	lo, hi := v.valueRange()
	ticks := max(min(int(plot.W/(72*s)), 6), 2)
	vmin, vmax, step := niceScale(lo, hi, true, ticks)

	labelW, axisH := float32(0), float32(0)
	if !v.Compact {
		for _, c := range cd.cats {
			w, _ := p.MeasureText(0, v.span(c, 10.5, t.TextMuted))
			labelW = max(labelW, w)
		}
		labelW = min(labelW, plot.W*0.35) + 8*s
		axisH = 18 * s
	}
	// Room at the right for half the last value under it.
	right := 8 * s
	if !v.Compact {
		w, _ := p.MeasureText(0, v.span(tickText(vmax, vmax, step), 10.5, t.TextMuted))
		right = max(right, w/2+2*s)
	}
	area := ui.Rect{X: plot.X + labelW, Y: plot.Y, W: plot.W - labelW - right, H: plot.H - axisH}
	if area.W <= 4*s || area.H <= 4*s {
		return
	}
	xOf := func(x float64) float32 {
		return area.X + float32((x-vmin)/(vmax-vmin))*area.W
	}
	grid := t.Border.Alpha(0.55)
	for x := vmin; x <= vmax+step/2; x += step {
		gx := xOf(x)
		p.Line(gx, area.Y, gx, area.Y+area.H, v.hairline(), grid)
		if !v.Compact {
			label := v.span(tickText(x, vmax, step), 10.5, t.TextMuted)
			w, _ := p.MeasureText(0, label)
			p.RichText(gx-w/2, area.Y+area.H+5*s, 0, label)
		}
	}
	base := xOf(math.Max(math.Min(0, vmax), vmin))
	p.Line(base, area.Y, base, area.Y+area.H, v.hairline(), t.Border)

	n := len(cd.cats)
	band := area.H / float32(n)
	hi0 := -1
	if v.Hovering && v.Hover.Y >= area.Y && v.Hover.Y < area.Y+area.H && v.Hover.X >= plot.X && v.Hover.X < plot.X+plot.W {
		hi0 = min(int((v.Hover.Y-area.Y)/band), n-1)
		p.Fill(ui.Rect{X: area.X, Y: area.Y + band*float32(hi0), W: area.W, H: band}, t.Text.Alpha(0.045), 3*s)
	}
	nser := len(cd.series)
	gap := 2 * s
	group := band * 0.72
	h := max(min(24*s, (group-gap*float32(nser-1))/float32(nser)), 1)
	group = h*float32(nser) + gap*float32(nser-1)
	if !v.Compact {
		every := max(int(math.Ceil(float64(16*s/band))), 1)
		for i, c := range cd.cats {
			if i%every != 0 {
				continue
			}
			label := v.span(c, 10.5, t.TextMuted)
			w, lh := p.MeasureText(0, label)
			cy := area.Y + band*(float32(i)+0.5)
			p.Clip(ui.Rect{X: plot.X, Y: cy - lh, W: labelW - 8*s, H: 2 * lh}, 0, func() {
				p.RichText(max(area.X-8*s-w, plot.X), cy-lh/2, 0, label)
			})
		}
	}
	for si, row := range cd.vals {
		color := hue(si, t.Dark)
		for i, x := range row {
			if math.IsNaN(x) {
				continue
			}
			by := area.Y + band*float32(i) + (band-group)/2 + float32(si)*(h+gap)
			v.bar(p, ui.Rect{X: min(xOf(x), base), Y: by, W: abs(xOf(x) - base), H: h}, x >= 0, true, color)
		}
	}
	if hi0 >= 0 && !v.Compact {
		v.tooltipAt(p, bounds, hi0)
	}
}

// bar fills a bar: its data end rounded, square where it stands on the
// baseline. up says the value is positive; flat that the bar lies down.
func (v View) bar(p *ui.Painter, r ui.Rect, up, flat bool, color ui.Color) {
	if r.W <= 0 || r.H <= 0 {
		return
	}
	var k float32
	if flat {
		k = min(4*v.S, r.H/2, r.W)
	} else {
		k = min(4*v.S, r.W/2, r.H)
	}
	const c = 0.5523 // a quarter circle's control points
	x0, y0, x1, y1 := r.X, r.Y, r.X+r.W, r.Y+r.H
	path := new(ui.Path)
	switch {
	case !flat && up: // rounded at the top
		path.MoveTo(x0, y1).LineTo(x0, y0+k).CubeTo(x0, y0+k-k*c, x0+k-k*c, y0, x0+k, y0).
			LineTo(x1-k, y0).CubeTo(x1-k+k*c, y0, x1, y0+k-k*c, x1, y0+k).LineTo(x1, y1)
	case !flat: // below zero: rounded at the bottom
		path.MoveTo(x0, y0).LineTo(x0, y1-k).CubeTo(x0, y1-k+k*c, x0+k-k*c, y1, x0+k, y1).
			LineTo(x1-k, y1).CubeTo(x1-k+k*c, y1, x1, y1-k+k*c, x1, y1-k).LineTo(x1, y0)
	case up: // lying down, rounded at the right
		path.MoveTo(x0, y0).LineTo(x1-k, y0).CubeTo(x1-k+k*c, y0, x1, y0+k-k*c, x1, y0+k).
			LineTo(x1, y1-k).CubeTo(x1, y1-k+k*c, x1-k+k*c, y1, x1-k, y1).LineTo(x0, y1)
	default: // lying down below zero: rounded at the left
		path.MoveTo(x1, y0).LineTo(x0+k, y0).CubeTo(x0+k-k*c, y0, x0, y0+k-k*c, x0, y0+k).
			LineTo(x0, y1-k).CubeTo(x0, y1-k+k*c, x0+k-k*c, y1, x0+k, y1).LineTo(x1, y1)
	}
	p.FillPath(path.Close(), color)
}

// tooltipAt shows the values of category i by the pointer.
func (v View) tooltipAt(p *ui.Painter, bounds ui.Rect, i int) {
	cd := v.Data
	rows := make([]tipRow, 0, len(cd.series))
	for si, name := range cd.series {
		rows = append(rows, tipRow{hue(si, v.T.Dark), name, valueText(cd.vals[si][i])})
	}
	v.tooltip(p, bounds, cd.cats[i], rows)
}

// tipRow is one line of a tooltip: a key, a name and its value.
type tipRow struct {
	color       ui.Color
	name, value string
}

// tooltip draws a card of rows under a title by the pointer, kept inside
// bounds.
func (v View) tooltip(p *ui.Painter, bounds ui.Rect, title string, rows []tipRow) {
	s, t := v.S, v.T
	pad, lineH, sw := 8*s, 17*s, 8*s
	head := ui.Span{Text: title, Size: 11.5 * v.S, Weight: 600, Color: t.Text}
	w, _ := p.MeasureText(0, head)
	nameW, valueW := float32(0), float32(0)
	for _, r := range rows {
		nw, _ := p.MeasureText(0, v.span(r.name, 11, t.TextMuted))
		vw, _ := p.MeasureText(0, ui.Span{Text: r.value, Size: 11 * s, Weight: 600, Color: t.Text})
		nameW, valueW = max(nameW, nw), max(valueW, vw)
	}
	w = max(w, sw+6*s+nameW+16*s+valueW)
	box := ui.Rect{W: w + 2*pad, H: lineH*float32(len(rows)+1) + 2*pad - 2*s}
	box.X, box.Y = v.Hover.X+14*s, v.Hover.Y+14*s
	if box.X+box.W > bounds.X+bounds.W-4*s {
		box.X = v.Hover.X - 14*s - box.W
	}
	if box.Y+box.H > bounds.Y+bounds.H-4*s {
		box.Y = v.Hover.Y - 14*s - box.H
	}
	box.X = max(box.X, bounds.X+4*s)
	box.Y = max(box.Y, bounds.Y+4*s)
	shade := ui.RGBA(0, 0, 0, 0.14)
	if t.Dark {
		shade = ui.RGBA(0, 0, 0, 0.5)
	}
	p.Shadow(box, 6*s, 0, 2*s, 10*s, 0, shade)
	p.Fill(box, t.Surface, 6*s)
	p.Stroke(box, t.Border, 6*s, v.hairline())
	p.RichText(box.X+pad, box.Y+pad, 0, head)
	for i, r := range rows {
		y := box.Y + pad + lineH*float32(i+1)
		p.Fill(ui.Rect{X: box.X + pad, Y: y + (lineH-sw)/2 - 2*s, W: sw, H: sw}, r.color, 2*s)
		p.RichText(box.X+pad+sw+6*s, y, 0, v.span(r.name, 11, t.TextMuted))
		value := ui.Span{Text: r.value, Size: 11 * s, Weight: 600, Color: t.Text}
		vw, _ := p.MeasureText(0, value)
		p.RichText(box.X+box.W-pad-vw, y, 0, value)
	}
}

// pieSlice is one slice of a pie: a category, or the rest as one.
type pieSlice struct {
	name  string
	value float64
	color ui.Color
}

// pieSlices are the first series' positive values as slices: past
// maxSeries of them, the smallest fold into 其他.
func (v View) pieSlices() ([]pieSlice, float64) {
	cd := v.Data
	var out []pieSlice
	var total float64
	for i, x := range cd.vals[0] {
		if math.IsNaN(x) || x <= 0 {
			continue
		}
		out = append(out, pieSlice{name: cd.cats[i], value: x})
		total += x
	}
	if len(out) > maxSeries {
		rest := pieSlice{name: "其他"}
		for _, sl := range out[maxSeries-1:] {
			rest.value += sl.value
		}
		out = append(out[:maxSeries-1], rest)
	}
	for i := range out {
		out[i].color = hue(i, v.T.Dark)
		if out[i].name == "其他" && i == maxSeries-1 {
			out[i].color = v.T.TextMuted.Alpha(0.6)
		}
	}
	return out, total
}

// drawPie draws the first series as a pie, its legend beside it when
// there is room, else under it.
func (v View) drawPie(p *ui.Painter, in, bounds ui.Rect) {
	s, t := v.S, v.T
	slices, total := v.pieSlices()
	if total <= 0 {
		return
	}
	disc := in
	var keys []legendKey
	if !v.Compact {
		keys = make([]legendKey, len(slices))
		for i, sl := range slices {
			keys[i] = legendKey{sl.color, fmt.Sprintf("%s %s", sl.name, shareText(sl.value, total))}
		}
		if in.W >= in.H*1.3 {
			// Beside it, as tall as it is.
			legendW := min(in.W*0.4, 180*s)
			disc.W -= legendW + 12*s
			lr := ui.Rect{X: in.X + in.W - legendW, W: legendW}
			lineH := 16 * s
			lr.H = lineH * float32(len(keys))
			lr.Y = in.Y + max((in.H-lr.H)/2, 0)
			for i, k := range keys {
				v.drawLegend(p, ui.Rect{X: lr.X, Y: lr.Y + lineH*float32(i), W: lr.W, H: lineH}, []legendKey{k})
			}
		} else {
			// Under it: measured first, then laid at the foot.
			lr := ui.Rect{X: in.X, W: in.W, H: in.H / 3}
			h := v.legend(p, lr, keys, false)
			lr.Y = in.Y + in.H - h
			v.legend(p, lr, keys, true)
			disc.H -= h + 10*s
		}
	}
	radius := min(disc.W, disc.H)/2 - 2*s
	if radius <= 4*s {
		return
	}
	cx, cy := disc.X+disc.W/2, disc.Y+disc.H/2

	// The slice hovered.
	hovered := -1
	if v.Hovering {
		dx, dy := v.Hover.X-cx, v.Hover.Y-cy
		if dx*dx+dy*dy <= radius*radius {
			a := math.Atan2(float64(dy), float64(dx)) + math.Pi/2
			if a < 0 {
				a += 2 * math.Pi
			}
			at := 0.0
			for i, sl := range slices {
				share := sl.value / total * 2 * math.Pi
				if a >= at && a < at+share {
					hovered = i
				}
				at += share
			}
		}
	}

	at := -math.Pi / 2
	for i, sl := range slices {
		share := sl.value / total * 2 * math.Pi
		x, y := cx, cy
		if i == hovered {
			// It comes out a little.
			mid := at + share/2
			x += float32(math.Cos(mid)) * 4 * s
			y += float32(math.Sin(mid)) * 4 * s
		}
		path := new(ui.Path)
		if len(slices) > 1 {
			path.MoveTo(x, y)
		}
		steps := max(int(share/(math.Pi/90)), 2)
		for k := 0; k <= steps; k++ {
			a := at + share*float64(k)/float64(steps)
			px, py := x+radius*float32(math.Cos(a)), y+radius*float32(math.Sin(a))
			if k == 0 && len(slices) == 1 {
				path.MoveTo(px, py)
			} else {
				path.LineTo(px, py)
			}
		}
		path.Close()
		p.FillPath(path, sl.color)
		if len(slices) > 1 {
			// The surface parts it from the next: a gap, not a stroke.
			p.StrokePath(path.Join(ui.RoundJoin), 2*s, t.Surface)
		}
		at += share
	}
	if hovered >= 0 && !v.Compact {
		sl := slices[hovered]
		v.tooltip(p, bounds, sl.name, []tipRow{
			{sl.color, v.Data.series[0], valueText(sl.value)},
			{sl.color, "占比", shareText(sl.value, total)},
		})
	}
}

// shareText is part of total in percent.
func shareText(part, total float64) string {
	pct := part / total * 100
	if pct >= 10 || pct == math.Trunc(pct) {
		return strconv.FormatFloat(math.Round(pct), 'f', 0, 64) + "%"
	}
	return strconv.FormatFloat(pct, 'f', 1, 64) + "%"
}

func abs(v float32) float32 {
	if v < 0 {
		return -v
	}
	return v
}
