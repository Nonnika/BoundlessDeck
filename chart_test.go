package main

import (
	"math"
	"strings"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

func TestParseChartReadsTablesAsTyped(t *testing.T) {
	cd := parseChart("月份, 收入, 支出\n1月, 120, 80\n2月, , ¥150\n3月，98，70%")
	if strings.Join(cd.series, "|") != "收入|支出" || strings.Join(cd.cats, "|") != "1月|2月|3月" {
		t.Fatalf("series %v, categories %v", cd.series, cd.cats)
	}
	if cd.vals[0][0] != 120 || cd.vals[1][2] != 70 || !math.IsNaN(cd.vals[0][1]) || cd.vals[1][1] != 150 {
		t.Fatalf("values %v", cd.vals)
	}

	// Tabs, as a spreadsheet copies them: thousands keep their commas.
	tabs := parseChart("城市\t人口\n北京\t2,189\n上海\t2,487")
	if tabs.series[0] != "人口" || tabs.vals[0][1] != 2487 {
		t.Fatalf("tab table read as %v %v", tabs.series, tabs.vals)
	}

	// A Markdown table.
	md := parseChart("| 项目 | 数量 |\n|---|---:|\n| 甲 | 3 |\n| 乙 | 5 |")
	if strings.Join(md.cats, "|") != "甲|乙" || md.vals[0][1] != 5 {
		t.Fatalf("Markdown table read as %v %v", md.cats, md.vals)
	}

	// No header: the series are named for what they are.
	bare := parseChart("2019, 30, 40\n2020, 35, 45")
	if len(bare.cats) != 2 || bare.cats[0] != "2019" || bare.series[1] != "系列 2" {
		t.Fatalf("headerless table read as %v %v", bare.cats, bare.series)
	}

	// One column: the rows are numbered.
	one := parseChart("销量\n5\n8")
	if strings.Join(one.cats, "|") != "1|2" || one.series[0] != "销量" || one.vals[0][1] != 8 {
		t.Fatalf("one column read as %v %v %v", one.cats, one.series, one.vals)
	}

	// Past eight series, a ninth would need a hue there is not.
	wide := parseChart("x,a,b,c,d,e,f,g,h,i\n1,1,2,3,4,5,6,7,8,9")
	if len(wide.series) != chartMaxSeries || wide.note == "" {
		t.Fatalf("nine series read as %d (%q)", len(wide.series), wide.note)
	}

	if !parseChart("").empty() || !parseChart("a, b\nx, y").empty() {
		t.Fatal("a table with no number is not empty")
	}
}

func TestChartScaleIsRound(t *testing.T) {
	lo, hi, step := chartScale(3, 187, true, 5)
	if lo != 0 || hi != 200 || step != 50 {
		t.Fatalf("0..187 scaled to %v..%v by %v", lo, hi, step)
	}
	lo, hi, step = chartScale(-12, 30, true, 4)
	if lo > -12 || hi < 30 || math.Mod(lo, step) != 0 {
		t.Fatalf("-12..30 scaled to %v..%v by %v", lo, hi, step)
	}
	if got := chartTick(20000, 40000, 10000); got != "2万" {
		t.Fatalf("20000 on a 40000 axis reads %q", got)
	}
	if got := chartTick(0.5, 1, 0.25); got != "0.50" {
		t.Fatalf("0.5 by 0.25 reads %q", got)
	}
	if got := chartValue(1234567.891); got != "1,234,567.89" {
		t.Fatalf("chartValue = %q", got)
	}
	if got := chartShare(1, 3); got != "33%" {
		t.Fatalf("a third reads %q", got)
	}
}

func TestAChartBlockDrawsAndEditsItsTable(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 1100, 700)
	tt.Frame()
	if err := tt.Click("图表"); err != nil {
		t.Fatalf("toolbar: %v", err)
	}
	tt.Frame()
	n := d.blocks[len(d.blocks)-1]
	if n.Kind != kindChart || n.Text != chartSample || chartTypeOf(n) != chartBar {
		t.Fatalf("the toolbar made %+v", n)
	}
	if !tt.HasText("柱状图") {
		t.Fatalf("the header does not say how the chart draws: %v", tt.Texts())
	}

	// Its chip asks which chart to draw.
	r, _ := tt.Find("柱状图")
	tt.ClickAt(r.X+r.W/2, r.Y+r.H/2)
	if d.chartAsked != n {
		t.Fatal("the chip did not ask for the chart kinds")
	}
	d.setChartType(n, chartPie)
	tt.Frame()
	if !tt.HasText("饼图") {
		t.Fatalf("set to a pie, the header says %v", tt.Texts())
	}

	// A double click edits its table beside the chart, which follows.
	sx, sy := d.cam.toScreen(n.X+n.W/2, n.Y+n.H/2)
	tt.ClickAt(sx, sy)
	tt.ClickAt(sx, sy)
	tt.Frame()
	if d.editing != n || d.editPart != editBody {
		t.Fatalf("a double click edits %v", d.editing)
	}
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if d.editing != nil {
		t.Fatal("escape did not end the edit")
	}
}

func TestChartsDrawEveryKindWithoutFault(t *testing.T) {
	for _, ct := range chartTypes {
		for _, src := range []string{chartSample, "", "a, -3\nb, 5\nc, 0", "x\n7", "a,b\n1,\n2,"} {
			d := testDeck(t)
			d.blocks = append(d.blocks, &block{ID: "c", Kind: kindChart, Title: "图", X: 100, Y: 260, W: 480, H: 320,
				Text: src, Chart: string(ct.typ)})
			tt := ui.NewTester(d.view, 900, 700)
			tt.Frame()
			tt.Move(340, 420) // a tooltip too
			tt.Frame()
			tt.Advance(100 * time.Millisecond)
			tt.Image()
		}
	}
}

func TestMCPCreatesAChart(t *testing.T) {
	s, d := mcpTestServer(t)
	out := toolCall(t, s, "create_block", map[string]any{
		"kind": "chart", "chart": "line", "text": "季度, 销量\nQ1, 10\nQ2, 14",
	})
	n := d.blockByID(out["id"].(string))
	if n == nil || chartTypeOf(n) != chartLine || out["chart"] != "line" || out["text"] != n.Text {
		t.Fatalf("created %+v, read %v", n, out)
	}
	if _, isErr, text := toolCallRaw(t, s, "update_block", map[string]any{"id": n.ID, "chart": "radar"}); !isErr || !strings.Contains(text, "radar") {
		t.Fatalf("an unknown chart kind: isErr=%v %q", isErr, text)
	}
	toolCall(t, s, "update_block", map[string]any{"id": n.ID, "chart": "pie"})
	if chartTypeOf(n) != chartPie {
		t.Fatalf("update_block left the chart %q", n.Chart)
	}
}
