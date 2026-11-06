package main

import (
	"strings"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"

	"boundlessdeck/internal/chart"
)

func TestAChartBlockDrawsAndEditsItsTable(t *testing.T) {
	d := testDeck(t)
	tt := ui.NewTester(d.view, 1100, 700)
	tt.Frame()
	if err := tt.Click("图表"); err != nil {
		t.Fatalf("toolbar: %v", err)
	}
	tt.Frame()
	n := d.blocks[len(d.blocks)-1]
	if n.Kind != kindChart || n.Text != chart.Sample || chartTypeOf(n) != chart.Bar {
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
	d.setChartType(n, chart.Pie)
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
	for _, ct := range chart.Types {
		for _, src := range []string{chart.Sample, "", "a, -3\nb, 5\nc, 0", "x\n7", "a,b\n1,\n2,"} {
			d := testDeck(t)
			d.blocks = append(d.blocks, &block{ID: "c", Kind: kindChart, Title: "图", X: 100, Y: 260, W: 480, H: 320,
				Text: src, Chart: string(ct.Type)})
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
	if n == nil || chartTypeOf(n) != chart.Line || out["chart"] != "line" || out["text"] != n.Text {
		t.Fatalf("created %+v, read %v", n, out)
	}
	if _, isErr, text := toolCallRaw(t, s, "update_block", map[string]any{"id": n.ID, "chart": "radar"}); !isErr || !strings.Contains(text, "radar") {
		t.Fatalf("an unknown chart kind: isErr=%v %q", isErr, text)
	}
	toolCall(t, s, "update_block", map[string]any{"id": n.ID, "chart": "pie"})
	if chartTypeOf(n) != chart.Pie {
		t.Fatalf("update_block left the chart %q", n.Chart)
	}
}
