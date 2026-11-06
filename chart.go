package main

import (
	"fmt"
	"math"
	"strconv"
	"strings"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The 图表 block (kindChart, "chart"): a chart of a small table the block
// keeps in its Text, one row per category, a column per series:
//
//	月份, 收入, 支出
//	1月, 120, 80
//	2月, 150, 95
//
// Cells part at tabs (as a spreadsheet copies them), else at commas or
// semicolons; a Markdown table reads too. The first row names the series
// when a cell after its first is no number. Its Chart field says how it
// draws: bars, horizontal bars, a line, an area or a pie. The header's
// chip picks the kind; the table button or a double click on the chart
// edits the table beside the chart, which follows as it is typed.

type chartType string

const (
	chartBar  chartType = "bar"
	chartHBar chartType = "hbar"
	chartLine chartType = "line"
	chartArea chartType = "area"
	chartPie  chartType = "pie"
)

// chartTypes is every chart a block draws, in the order its menu lists
// them; the first is a new block's.
var chartTypes = []struct {
	typ   chartType
	label string
	icon  *ui.SVG
}{
	{chartBar, "柱状图", iconChart},
	{chartHBar, "条形图", iconChartBar},
	{chartLine, "折线图", iconChartLine},
	{chartArea, "面积图", iconChartArea},
	{chartPie, "饼图", iconChartPie},
}

// chartTypeNames is chartTypes as the MCP names them.
func chartTypeNames() []string {
	out := make([]string, len(chartTypes))
	for i, ct := range chartTypes {
		out[i] = string(ct.typ)
	}
	return out
}

// chartTypeOf is how n draws: bars when its Chart names no chart.
func chartTypeOf(n *block) chartType {
	for _, ct := range chartTypes {
		if string(ct.typ) == n.Chart {
			return ct.typ
		}
	}
	return chartBar
}

func chartLabel(t chartType) string {
	for _, ct := range chartTypes {
		if ct.typ == t {
			return ct.label
		}
	}
	return "图表"
}

func chartIcon(t chartType) *ui.SVG {
	for _, ct := range chartTypes {
		if ct.typ == t {
			return ct.icon
		}
	}
	return iconChart
}

// chartSample is the table a new 图表 block starts with.
const chartSample = "月份, 收入, 支出\n1月, 120, 80\n2月, 150, 95\n3月, 98, 70\n4月, 180, 110\n5月, 165, 120"

// A chart shows at most chartMaxSeries series, each its own hue: a ninth
// would need a hue that is not there. A pie shows chartMaxSeries slices,
// the rest as one. At most chartMaxCats rows are read.
const (
	chartMaxSeries = 8
	chartMaxCats   = 200
)

// chartData is a chart's table as read.
type chartData struct {
	series []string
	cats   []string
	vals   [][]float64 // [series][category]; NaN where a cell holds no number
	note   string      // what of the table was left out
}

func (cd *chartData) empty() bool {
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

// parseChart reads a chart's table.
func parseChart(src string) *chartData {
	cd := &chartData{}
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
			if chartRule(cells) {
				continue // a Markdown table's |---|---|
			}
		case tabs:
			cells = strings.Split(line, "\t")
		default:
			// An empty cell stays one: a,,5 has a gap in its middle.
			cells = strings.Split(chartSeps.Replace(line), ",")
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
		if _, ok := chartNumber(cell); !ok && cell != "" {
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
	if nser > chartMaxSeries {
		cd.note = fmt.Sprintf("只画前 %d 列", chartMaxSeries)
		nser = chartMaxSeries
	}
	if len(data) > chartMaxCats {
		cd.note = fmt.Sprintf("只画前 %d 行", chartMaxCats)
		data = data[:chartMaxCats]
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
				if f, ok := chartNumber(cells[si]); ok {
					v = f
				}
			}
			cd.vals[si] = append(cd.vals[si], v)
		}
	}
	return cd
}

// chartSeps makes every separator a cell may end at a comma.
var chartSeps = strings.NewReplacer("，", ",", ";", ",", "；", ",")

// chartRule says whether cells are a Markdown table's rule: dashes and
// colons alone.
func chartRule(cells []string) bool {
	for _, c := range cells {
		if strings.Trim(strings.TrimSpace(c), "-: ") != "" {
			return false
		}
	}
	return true
}

// chartNumber reads a cell as a number: 1,200 and 1200 alike, a currency
// sign before it or % after it left out.
func chartNumber(s string) (float64, bool) {
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

// chartDataOf is n's table, read again only when its text changed.
func (d *deck) chartDataOf(n *block) *chartData {
	rt := d.ensureRuntime(n)
	if rt.chart == nil || rt.chartSrc != n.Text {
		rt.chart, rt.chartSrc = parseChart(n.Text), n.Text
	}
	return rt.chart
}

// chartSummary is the line a card in a 块堆 says of a chart.
func chartSummary(n *block, cd *chartData) string {
	if cd.empty() {
		return chartLabel(chartTypeOf(n)) + " · 还没有数据"
	}
	if len(cd.series) == 1 {
		return fmt.Sprintf("%s · %d 项", chartLabel(chartTypeOf(n)), len(cd.cats))
	}
	return fmt.Sprintf("%s · %d 项 × %d 组", chartLabel(chartTypeOf(n)), len(cd.cats), len(cd.series))
}

// The categorical hues, in their fixed order, for the light and the dark
// theme: the same hues, stepped for each surface. Series i always wears
// hue i.
var (
	chartLight = []ui.Color{ui.Hex("#2a78d6"), ui.Hex("#eb6834"), ui.Hex("#1baf7a"), ui.Hex("#eda100"),
		ui.Hex("#e87ba4"), ui.Hex("#008300"), ui.Hex("#6250d6"), ui.Hex("#e34948")}
	chartDark = []ui.Color{ui.Hex("#3987e5"), ui.Hex("#d95926"), ui.Hex("#199e70"), ui.Hex("#c98500"),
		ui.Hex("#d55181"), ui.Hex("#008300"), ui.Hex("#9085e9"), ui.Hex("#e66767")}
)

func chartColor(i int, dark bool) ui.Color {
	if dark {
		return chartDark[i%len(chartDark)]
	}
	return chartLight[i%len(chartLight)]
}

// chartScale is the axis a chart's values lie on: from lo to hi in steps,
// round numbers; with zero, 0 is on it.
func chartScale(lo, hi float64, zero bool, ticks int) (float64, float64, float64) {
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

// chartTick writes an axis value: in 万 or 亿 on an axis that reaches
// them, as many decimals as its step needs.
func chartTick(v, top, step float64) string {
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

// chartValue writes a value as a tooltip shows it: in full, its thousands
// grouped, at most two decimals.
func chartValue(v float64) string {
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

// chartView is a chart as one frame draws it.
type chartView struct {
	typ  chartType
	data *chartData
	s    float32 // its scale: 1 at 100%
	t    *ui.Theme
	// hover is the pointer, in the painter's coordinates, when hovering.
	hover    ui.Point
	hovering bool
	// compact is a chart small on a 块堆's card: marks alone.
	compact bool
}

func (v chartView) span(text string, size float32, color ui.Color) ui.Span {
	return ui.Span{Text: text, Size: size * v.s, Color: color}
}

func (v chartView) hairline() float32 { return max(1*v.s, 0.5) }

// draw paints the chart in r.
func (v chartView) draw(p *ui.Painter, r ui.Rect) {
	s, t := v.s, v.t
	if v.data == nil || v.data.empty() {
		if !v.compact {
			msg := v.span("还没有数据 · 双击或点标题栏的表格按钮填写", 12, t.TextMuted)
			w, h := p.MeasureText(r.W-24*s, msg)
			p.RichText(r.X+(r.W-w)/2, r.Y+(r.H-h)/2, r.W-24*s, msg)
		}
		return
	}
	pad := 12 * s
	if v.compact {
		pad = 4 * s
	}
	in := ui.Rect{X: r.X + pad, Y: r.Y + pad, W: r.W - 2*pad, H: r.H - 2*pad}
	if in.W <= 8*s || in.H <= 8*s {
		return
	}
	p.Clip(r, 0, func() {
		if v.typ == chartPie {
			v.drawPie(p, in, r)
			return
		}
		top := in.Y
		if len(v.data.series) > 1 && !v.compact {
			items := make([]chartKey, len(v.data.series))
			for i, name := range v.data.series {
				items[i] = chartKey{chartColor(i, t.Dark), name}
			}
			h := v.drawLegend(p, ui.Rect{X: in.X, Y: in.Y, W: in.W, H: in.H / 3}, items)
			top += h + 8*s
		}
		plot := ui.Rect{X: in.X, Y: top, W: in.W, H: in.Y + in.H - top}
		if v.data.note != "" && !v.compact {
			note := v.span(v.data.note, 10, t.TextMuted)
			nw, nh := p.MeasureText(0, note)
			p.RichText(in.X+in.W-nw, in.Y+in.H-nh, 0, note)
			plot.H -= nh + 4*s
		}
		if v.typ == chartHBar {
			v.drawHBars(p, plot, r)
			return
		}
		v.drawColumns(p, plot, r)
	})
}

// chartKey is one entry of a legend.
type chartKey struct {
	color ui.Color
	name  string
}

// drawLegend lays the keys out in rows from r's top left, and says how
// tall they came out.
func (v chartView) drawLegend(p *ui.Painter, r ui.Rect, items []chartKey) float32 {
	return v.legend(p, r, items, true)
}

// legend lays the keys out, drawing them when draw says so, and says how
// tall they come out.
func (v chartView) legend(p *ui.Painter, r ui.Rect, items []chartKey, draw bool) float32 {
	s := v.s
	sw, gap, lineH := 8*s, 14*s, 16*s
	x, y := r.X, r.Y
	for _, it := range items {
		label := v.span(it.name, 11, v.t.TextMuted)
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
func (v chartView) valueRange() (float64, float64) {
	lo, hi := math.Inf(1), math.Inf(-1)
	for _, row := range v.data.vals {
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
func (v chartView) drawColumns(p *ui.Painter, plot, bounds ui.Rect) {
	s, t, cd := v.s, v.t, v.data
	lo, hi := v.valueRange()
	ticks := max(min(int(plot.H/(44*s)), 6), 2)
	vmin, vmax, step := chartScale(lo, hi, v.typ != chartLine, ticks)

	labelH := float32(0)
	axisW := float32(0)
	if !v.compact {
		labelH = 18 * s
		for x := vmin; x <= vmax+step/2; x += step {
			w, _ := p.MeasureText(0, v.span(chartTick(x, vmax, step), 10.5, t.TextMuted))
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
		if !v.compact {
			label := v.span(chartTick(x, vmax, step), 10.5, t.TextMuted)
			w, h := p.MeasureText(0, label)
			p.RichText(area.X-8*s-w, y-h/2, 0, label)
		}
	}
	base := yOf(math.Max(math.Min(0, vmax), vmin))
	p.Line(area.X, base, area.X+area.W, base, v.hairline(), t.Border)

	n := len(cd.cats)
	band := area.W / float32(n)
	hi0 := -1
	if v.hovering && v.hover.X >= area.X && v.hover.X < area.X+area.W && v.hover.Y >= plot.Y && v.hover.Y < plot.Y+plot.H {
		hi0 = min(int((v.hover.X-area.X)/band), n-1)
		p.Fill(ui.Rect{X: area.X + band*float32(hi0), Y: area.Y, W: band, H: area.H}, t.Text.Alpha(0.045), 3*s)
	}

	// The categories under it, as many as fit side by side.
	if !v.compact {
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
	switch v.typ {
	case chartBar:
		gap := 2 * s
		group := band * 0.72
		w := min(24*s, (group-gap*float32(nser-1))/float32(nser))
		w = max(w, 1)
		group = w*float32(nser) + gap*float32(nser-1)
		for si, row := range cd.vals {
			color := chartColor(si, t.Dark)
			for i, x := range row {
				if math.IsNaN(x) {
					continue
				}
				bx := area.X + band*float32(i) + (band-group)/2 + float32(si)*(w+gap)
				v.bar(p, ui.Rect{X: bx, Y: min(yOf(x), base), W: w, H: abs(yOf(x) - base)}, x >= 0, false, color)
			}
		}
	case chartLine, chartArea:
		for si, row := range cd.vals {
			color := chartColor(si, t.Dark)
			var run [][2]float32
			flush := func() {
				switch {
				case len(run) == 1:
					p.FillPath(new(ui.Path).Circle(run[0][0], run[0][1], 3*s), color)
				case len(run) > 1:
					if v.typ == chartArea {
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
					p.FillPath(new(ui.Path).Circle(cx, cy, 4*s), chartColor(si, t.Dark))
				}
			}
		}
	}
	if hi0 >= 0 && !v.compact {
		v.tooltipAt(p, bounds, hi0)
	}
}

// drawHBars draws bars lying down: the categories down the left, the
// values along the bottom.
func (v chartView) drawHBars(p *ui.Painter, plot, bounds ui.Rect) {
	s, t, cd := v.s, v.t, v.data
	lo, hi := v.valueRange()
	ticks := max(min(int(plot.W/(72*s)), 6), 2)
	vmin, vmax, step := chartScale(lo, hi, true, ticks)

	labelW, axisH := float32(0), float32(0)
	if !v.compact {
		for _, c := range cd.cats {
			w, _ := p.MeasureText(0, v.span(c, 10.5, t.TextMuted))
			labelW = max(labelW, w)
		}
		labelW = min(labelW, plot.W*0.35) + 8*s
		axisH = 18 * s
	}
	// Room at the right for half the last value under it.
	right := 8 * s
	if !v.compact {
		w, _ := p.MeasureText(0, v.span(chartTick(vmax, vmax, step), 10.5, t.TextMuted))
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
		if !v.compact {
			label := v.span(chartTick(x, vmax, step), 10.5, t.TextMuted)
			w, _ := p.MeasureText(0, label)
			p.RichText(gx-w/2, area.Y+area.H+5*s, 0, label)
		}
	}
	base := xOf(math.Max(math.Min(0, vmax), vmin))
	p.Line(base, area.Y, base, area.Y+area.H, v.hairline(), t.Border)

	n := len(cd.cats)
	band := area.H / float32(n)
	hi0 := -1
	if v.hovering && v.hover.Y >= area.Y && v.hover.Y < area.Y+area.H && v.hover.X >= plot.X && v.hover.X < plot.X+plot.W {
		hi0 = min(int((v.hover.Y-area.Y)/band), n-1)
		p.Fill(ui.Rect{X: area.X, Y: area.Y + band*float32(hi0), W: area.W, H: band}, t.Text.Alpha(0.045), 3*s)
	}
	nser := len(cd.series)
	gap := 2 * s
	group := band * 0.72
	h := max(min(24*s, (group-gap*float32(nser-1))/float32(nser)), 1)
	group = h*float32(nser) + gap*float32(nser-1)
	if !v.compact {
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
		color := chartColor(si, t.Dark)
		for i, x := range row {
			if math.IsNaN(x) {
				continue
			}
			by := area.Y + band*float32(i) + (band-group)/2 + float32(si)*(h+gap)
			v.bar(p, ui.Rect{X: min(xOf(x), base), Y: by, W: abs(xOf(x) - base), H: h}, x >= 0, true, color)
		}
	}
	if hi0 >= 0 && !v.compact {
		v.tooltipAt(p, bounds, hi0)
	}
}

// bar fills a bar: its data end rounded, square where it stands on the
// baseline. up says the value is positive; flat that the bar lies down.
func (v chartView) bar(p *ui.Painter, r ui.Rect, up, flat bool, color ui.Color) {
	if r.W <= 0 || r.H <= 0 {
		return
	}
	var k float32
	if flat {
		k = min(4*v.s, r.H/2, r.W)
	} else {
		k = min(4*v.s, r.W/2, r.H)
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
func (v chartView) tooltipAt(p *ui.Painter, bounds ui.Rect, i int) {
	cd := v.data
	rows := make([]chartTipRow, 0, len(cd.series))
	for si, name := range cd.series {
		rows = append(rows, chartTipRow{chartColor(si, v.t.Dark), name, chartValue(cd.vals[si][i])})
	}
	v.tooltip(p, bounds, cd.cats[i], rows)
}

// chartTipRow is one line of a tooltip: a key, a name and its value.
type chartTipRow struct {
	color       ui.Color
	name, value string
}

// tooltip draws a card of rows under a title by the pointer, kept inside
// bounds.
func (v chartView) tooltip(p *ui.Painter, bounds ui.Rect, title string, rows []chartTipRow) {
	s, t := v.s, v.t
	pad, lineH, sw := 8*s, 17*s, 8*s
	head := ui.Span{Text: title, Size: 11.5 * v.s, Weight: 600, Color: t.Text}
	w, _ := p.MeasureText(0, head)
	nameW, valueW := float32(0), float32(0)
	for _, r := range rows {
		nw, _ := p.MeasureText(0, v.span(r.name, 11, t.TextMuted))
		vw, _ := p.MeasureText(0, ui.Span{Text: r.value, Size: 11 * s, Weight: 600, Color: t.Text})
		nameW, valueW = max(nameW, nw), max(valueW, vw)
	}
	w = max(w, sw+6*s+nameW+16*s+valueW)
	box := ui.Rect{W: w + 2*pad, H: lineH*float32(len(rows)+1) + 2*pad - 2*s}
	box.X, box.Y = v.hover.X+14*s, v.hover.Y+14*s
	if box.X+box.W > bounds.X+bounds.W-4*s {
		box.X = v.hover.X - 14*s - box.W
	}
	if box.Y+box.H > bounds.Y+bounds.H-4*s {
		box.Y = v.hover.Y - 14*s - box.H
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
// chartMaxSeries of them, the smallest fold into 其他.
func (v chartView) pieSlices() ([]pieSlice, float64) {
	cd := v.data
	var out []pieSlice
	var total float64
	for i, x := range cd.vals[0] {
		if math.IsNaN(x) || x <= 0 {
			continue
		}
		out = append(out, pieSlice{name: cd.cats[i], value: x})
		total += x
	}
	if len(out) > chartMaxSeries {
		rest := pieSlice{name: "其他"}
		for _, sl := range out[chartMaxSeries-1:] {
			rest.value += sl.value
		}
		out = append(out[:chartMaxSeries-1], rest)
	}
	for i := range out {
		out[i].color = chartColor(i, v.t.Dark)
		if out[i].name == "其他" && i == chartMaxSeries-1 {
			out[i].color = v.t.TextMuted.Alpha(0.6)
		}
	}
	return out, total
}

// drawPie draws the first series as a pie, its legend beside it when
// there is room, else under it.
func (v chartView) drawPie(p *ui.Painter, in, bounds ui.Rect) {
	s, t := v.s, v.t
	slices, total := v.pieSlices()
	if total <= 0 {
		return
	}
	disc := in
	var keys []chartKey
	if !v.compact {
		keys = make([]chartKey, len(slices))
		for i, sl := range slices {
			keys[i] = chartKey{sl.color, fmt.Sprintf("%s %s", sl.name, chartShare(sl.value, total))}
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
				v.drawLegend(p, ui.Rect{X: lr.X, Y: lr.Y + lineH*float32(i), W: lr.W, H: lineH}, []chartKey{k})
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
	if v.hovering {
		dx, dy := v.hover.X-cx, v.hover.Y-cy
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
	if hovered >= 0 && !v.compact {
		sl := slices[hovered]
		v.tooltip(p, bounds, sl.name, []chartTipRow{
			{sl.color, v.data.series[0], chartValue(sl.value)},
			{sl.color, "占比", chartShare(sl.value, total)},
		})
	}
}

// chartShare is part of total in percent.
func chartShare(part, total float64) string {
	pct := part / total * 100
	if pct >= 10 || pct == math.Trunc(pct) {
		return strconv.FormatFloat(math.Round(pct), 'f', 0, 64) + "%"
	}
	return strconv.FormatFloat(pct, 'f', 1, 64) + "%"
}

// chartBody shows a 图表 block's chart, and while its table is edited,
// the table beside it (under it in a narrow block).
func (d *deck) chartBody(c *ui.Context, n *block) {
	t := c.Theme()
	s := d.lay.S
	cd := d.chartDataOf(n)
	typ := chartTypeOf(n)
	chart := func(el ui.Element) {
		hx, hy, over := el.PointerPosition()
		el.Draw(func(p *ui.Painter, r ui.Rect) {
			chartView{typ: typ, data: cd, s: s, t: t,
				hover: ui.Point{X: r.X + hx, Y: r.Y + hy}, hovering: over}.draw(p, r)
		})
	}
	if d.editing != n || d.editPart != editBody {
		chart(ui.Box(c.Key("chart:" + n.ID)).Grow(1).FillWidth())
		return
	}
	wide := n.W >= 560
	w, h := n.W*s, n.H*s-d.titleHeight(n)
	var box ui.Element // one of the two: building an element puts it in the tree
	if wide {
		box = ui.Row(c).Size(w, h)
	} else {
		box = ui.Column(c).Size(w, h)
	}
	box.Children(func() {
		editor := ui.Box(c).Padding(8*s, 10*s).Background(t.SurfaceHover)
		if wide {
			editor = editor.Size(max(n.W*0.38, 200)*s, h)
		} else {
			editor = editor.Size(w, h*0.42)
		}
		editor.Children(func() {
			ui.TextArea(c.Key("chart-data:"+n.ID), &n.Text).Bind(&d.editInput).AutoFocus().Fill().
				FontSize(12*s).TextColor(t.Text).
				Placeholder("每行一项：名称, 数值, 数值…\n第一行可写列名 · Esc 或 ⌘↵ 完成").
				Background(ui.Transparent).Border(0, ui.Transparent).
				OnTextKey(d.endKeys)
			d.watchEditFocus(c)
		})
		if wide {
			chart(ui.Box(c.Key("chart:"+n.ID)).Size(w-max(n.W*0.38, 200)*s, h))
		} else {
			chart(ui.Box(c.Key("chart:"+n.ID)).Size(w, h*0.58))
		}
	})
}

// chartControls are a 图表 block's header controls: the chip that picks
// how it draws, and the button that edits its table.
func (d *deck) chartControls(c *ui.Context, n *block) {
	t := c.Theme()
	s := d.lay.S
	typ := chartTypeOf(n)
	press := func(fn func()) func(ui.InputEvent) bool {
		return func(ev ui.InputEvent) bool {
			if ev.Kind != ui.InputPointerDown || ev.Button != 0 {
				return false
			}
			fn()
			return true
		}
	}
	chip := ui.Row(c.Key("chart-type:"+n.ID)).AlignItems(ui.Center).Gap(4*s).Padding(2*s, 7*s).Radius(7 * s).
		Tooltip("图表类型 · 点击切换").Label("图表类型：" + chartLabel(typ)).Cursor(ui.CursorPointer)
	if chip.Hovered() {
		chip = chip.Background(t.SurfaceHover)
	}
	chip.HandleInput(press(func() { d.chartMenu(n) })).Children(func() {
		ui.Icon(c, chartIcon(typ)).FontSize(12 * s).TextColor(t.TextMuted)
		ui.Text(c, chartLabel(typ)).FontSize(11 * s).SingleLine().TextColor(t.TextMuted)
	})
	editing := d.editing == n && d.editPart == editBody
	tip := "编辑数据（或双击图表）"
	if editing {
		tip = "完成编辑（Esc）"
	}
	btn := ui.Box(c.Key("chart-edit:"+n.ID)).Padding(3*s, 5*s).Radius(7 * s).
		Tooltip(tip).Label(tip).Cursor(ui.CursorPointer)
	if editing {
		btn = btn.Background(t.Accent.Alpha(0.14))
	} else if btn.Hovered() {
		btn = btn.Background(t.SurfaceHover)
	}
	color := t.TextMuted
	if editing {
		color = t.Accent
	}
	btn.HandleInput(press(func() {
		if editing {
			d.stopEditing()
			return
		}
		d.sel = map[string]bool{n.ID: true}
		d.startEditing(n, editBody)
	})).Children(func() {
		ui.Icon(c, iconTable).FontSize(12 * s).TextColor(color)
	})
}

// chartMenu pops up the charts n can draw as, its own checked.
func (d *deck) chartMenu(n *block) {
	d.chartAsked = n // tests have no window to pop a menu up in
	if d.win == nil {
		return
	}
	cur := chartTypeOf(n)
	items := []*mygo.MenuItem{{Label: "图表类型", Disabled: true}}
	for _, ct := range chartTypes {
		typ := ct.typ
		items = append(items, &mygo.MenuItem{
			Label: ct.label, Type: mygo.MenuItemCheckbox, Checked: typ == cur,
			Click: func(*mygo.MenuItem, *mygo.Window) {
				d.win.Update(func() { d.setChartType(n, typ) })
			},
		})
	}
	mygo.NewMenu(items).Popup(d.win)
}

// setChartType makes n draw as typ.
func (d *deck) setChartType(n *block, typ chartType) {
	if n.Chart == string(typ) {
		return
	}
	n.Chart = string(typ)
	d.requestSave()
}
