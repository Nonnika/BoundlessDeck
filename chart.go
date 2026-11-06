package main

import (
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"

	"boundlessdeck/internal/chart"
	"boundlessdeck/internal/icons"
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

// chartTypeOf is how n draws: bars when its Chart names no chart.
func chartTypeOf(n *block) chart.Type { return chart.Of(n.Chart) }

// chartDataOf is n's table, read again only when its text changed.
func (d *deck) chartDataOf(n *block) *chart.Data {
	rt := d.ensureRuntime(n)
	if rt.chart == nil || rt.chartSrc != n.Text {
		rt.chart, rt.chartSrc = chart.Parse(n.Text), n.Text
	}
	return rt.chart
}

// chartSummary is the line a card in a 块堆 says of a chart.
func chartSummary(n *block, cd *chart.Data) string { return cd.Summary(chartTypeOf(n)) }

// chartBody shows a 图表 block's chart, and while its table is edited,
// the table beside it (under it in a narrow block).
func (d *deck) chartBody(c *ui.Context, n *block) {
	t := c.Theme()
	s := d.lay.S
	cd := d.chartDataOf(n)
	typ := chartTypeOf(n)
	drawChart := func(el ui.Element) {
		hx, hy, over := el.PointerPosition()
		el.Draw(func(p *ui.Painter, r ui.Rect) {
			chart.View{Type: typ, Data: cd, S: s, T: t,
				Hover: ui.Point{X: r.X + hx, Y: r.Y + hy}, Hovering: over}.Draw(p, r)
		})
	}
	if d.editing != n || d.editPart != editBody {
		drawChart(ui.Box(c.Key("chart:" + n.ID)).Grow(1).FillWidth())
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
			drawChart(ui.Box(c.Key("chart:"+n.ID)).Size(w-max(n.W*0.38, 200)*s, h))
		} else {
			drawChart(ui.Box(c.Key("chart:"+n.ID)).Size(w, h*0.58))
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
		Tooltip("图表类型 · 点击切换").Label("图表类型：" + chart.Label(typ)).Cursor(ui.CursorPointer)
	if chip.Hovered() {
		chip = chip.Background(t.SurfaceHover)
	}
	chip.HandleInput(press(func() { d.chartMenu(n) })).Children(func() {
		ui.Icon(c, chart.Icon(typ)).FontSize(12 * s).TextColor(t.TextMuted)
		ui.Text(c, chart.Label(typ)).FontSize(11 * s).SingleLine().TextColor(t.TextMuted)
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
		ui.Icon(c, icons.Table).FontSize(12 * s).TextColor(color)
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
	for _, ct := range chart.Types {
		typ := ct.Type
		items = append(items, &mygo.MenuItem{
			Label: ct.Label, Type: mygo.MenuItemCheckbox, Checked: typ == cur,
			Click: func(*mygo.MenuItem, *mygo.Window) {
				d.win.Update(func() { d.setChartType(n, typ) })
			},
		})
	}
	mygo.NewMenu(items).Popup(d.win)
}

// setChartType makes n draw as typ.
func (d *deck) setChartType(n *block, typ chart.Type) {
	if n.Chart == string(typ) {
		return
	}
	n.Chart = string(typ)
	d.requestSave()
}
