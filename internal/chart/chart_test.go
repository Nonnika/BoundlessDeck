package chart

import (
	"math"
	"strings"
	"testing"
)

func TestParseChartReadsTablesAsTyped(t *testing.T) {
	cd := Parse("月份, 收入, 支出\n1月, 120, 80\n2月, , ¥150\n3月，98，70%")
	if strings.Join(cd.series, "|") != "收入|支出" || strings.Join(cd.cats, "|") != "1月|2月|3月" {
		t.Fatalf("series %v, categories %v", cd.series, cd.cats)
	}
	if cd.vals[0][0] != 120 || cd.vals[1][2] != 70 || !math.IsNaN(cd.vals[0][1]) || cd.vals[1][1] != 150 {
		t.Fatalf("values %v", cd.vals)
	}

	// Tabs, as a spreadsheet copies them: thousands keep their commas.
	tabs := Parse("城市\t人口\n北京\t2,189\n上海\t2,487")
	if tabs.series[0] != "人口" || tabs.vals[0][1] != 2487 {
		t.Fatalf("tab table read as %v %v", tabs.series, tabs.vals)
	}

	// A Markdown table.
	md := Parse("| 项目 | 数量 |\n|---|---:|\n| 甲 | 3 |\n| 乙 | 5 |")
	if strings.Join(md.cats, "|") != "甲|乙" || md.vals[0][1] != 5 {
		t.Fatalf("Markdown table read as %v %v", md.cats, md.vals)
	}

	// No header: the series are named for what they are.
	bare := Parse("2019, 30, 40\n2020, 35, 45")
	if len(bare.cats) != 2 || bare.cats[0] != "2019" || bare.series[1] != "系列 2" {
		t.Fatalf("headerless table read as %v %v", bare.cats, bare.series)
	}

	// One column: the rows are numbered.
	one := Parse("销量\n5\n8")
	if strings.Join(one.cats, "|") != "1|2" || one.series[0] != "销量" || one.vals[0][1] != 8 {
		t.Fatalf("one column read as %v %v %v", one.cats, one.series, one.vals)
	}

	// Past eight series, a ninth would need a hue there is not.
	wide := Parse("x,a,b,c,d,e,f,g,h,i\n1,1,2,3,4,5,6,7,8,9")
	if len(wide.series) != maxSeries || wide.note == "" {
		t.Fatalf("nine series read as %d (%q)", len(wide.series), wide.note)
	}

	if !Parse("").Empty() || !Parse("a, b\nx, y").Empty() {
		t.Fatal("a table with no number is not empty")
	}
}

func TestChartScaleIsRound(t *testing.T) {
	lo, hi, step := niceScale(3, 187, true, 5)
	if lo != 0 || hi != 200 || step != 50 {
		t.Fatalf("0..187 scaled to %v..%v by %v", lo, hi, step)
	}
	lo, hi, step = niceScale(-12, 30, true, 4)
	if lo > -12 || hi < 30 || math.Mod(lo, step) != 0 {
		t.Fatalf("-12..30 scaled to %v..%v by %v", lo, hi, step)
	}
	if got := tickText(20000, 40000, 10000); got != "2万" {
		t.Fatalf("20000 on a 40000 axis reads %q", got)
	}
	if got := tickText(0.5, 1, 0.25); got != "0.50" {
		t.Fatalf("0.5 by 0.25 reads %q", got)
	}
	if got := valueText(1234567.891); got != "1,234,567.89" {
		t.Fatalf("valueText = %q", got)
	}
	if got := shareText(1, 3); got != "33%" {
		t.Fatalf("a third reads %q", got)
	}
}
