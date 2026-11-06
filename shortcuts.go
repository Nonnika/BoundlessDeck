package main

import (
	"strings"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The 键盘快捷键 window, opened from 帮助 (⌘/): every command of the menu
// bar with its keys, read from the menu itself so the two never disagree,
// and the canvas's keys and gestures, which have no menu item.

// shortcutGroup is one section of the window: a title and its rows.
type shortcutGroup struct {
	title string
	rows  []shortcutRow
}

// shortcutRow is what a key or gesture does; keys are its key caps, as
// "⇧⌘N" or "空格", shown one after the other ("拖动" rows have none).
type shortcutRow struct {
	label string
	keys  []string
}

// gestureGroups are the keys and gestures of the canvas: they have no
// menu item, since an accelerator would take space and the arrows from
// the canvas and the inputs.
var gestureGroups = []shortcutGroup{
	{"画布", []shortcutRow{
		{"新建 QuickLook（在鼠标处）", []string{"双击空白处"}},
		{"框选", []string{"拖动空白处"}},
		{"移动画布", []string{"滚轮 / 双指滑动"}},
		{"缩放画布", []string{"⌘ + 滚轮 / 双指捏合"}},
	}},
	{"块堆", []shortcutRow{
		{"收进块堆", []string{"把 block 拖到块堆上"}},
		{"推出卡片", []string{"悬停块堆中间"}},
		{"滚动卡片", []string{"悬停上下边缘 / 滚轮"}},
		{"长预览", []string{"空格"}},
		{"切换卡片", []string{"←", "→"}},
		{"关闭长预览", []string{"空格 / Esc"}},
	}},
	{"QuickLook 与笔记", []shortcutRow{
		{"结束编辑", []string{"⌘↩ / Esc"}},
		{"QuickLook 消失时间", []string{"点击标题栏右侧的剩余时间"}},
		{"找回刚消失的 QuickLook", []string{"⌘Z"}},
	}},
	{"图表", []shortcutRow{
		{"编辑数据", []string{"双击图表 / 标题栏的表格按钮"}},
		{"切换图表类型", []string{"点击标题栏的类型"}},
		{"查看数值", []string{"悬停柱子、折线或扇区"}},
		{"结束编辑", []string{"⌘↩ / Esc"}},
	}},
}

// roleKeys are the keys of the menu items whose role gives them theirs.
var roleKeys = map[mygo.MenuRole]string{
	mygo.RoleUndo:             "⌘Z",
	mygo.RoleRedo:             "⇧⌘Z",
	mygo.RoleCut:              "⌘X",
	mygo.RoleCopy:             "⌘C",
	mygo.RolePaste:            "⌘V",
	mygo.RoleSelectAll:        "⌘A",
	mygo.RoleClose:            "⌘W",
	mygo.RoleToggleFullScreen: "⌃⌘F",
}

// keyGlyphs writes an accelerator as macOS writes it: "CmdOrCtrl+Shift+N"
// is "⇧⌘N", modifiers in the order ⌃⌥⇧⌘.
func keyGlyphs(acc string) string {
	var mods [4]bool
	key := ""
	for _, part := range strings.Split(acc, "+") {
		switch strings.ToLower(part) {
		case "ctrl", "control":
			mods[0] = true
		case "alt", "option":
			mods[1] = true
		case "shift":
			mods[2] = true
		case "cmd", "command", "cmdorctrl", "commandorcontrol":
			mods[3] = true
		case "":
			key += "+" // "CmdOrCtrl++"
		default:
			key += strings.ToUpper(part)
		}
	}
	s := ""
	for i, g := range []string{"⌃", "⌥", "⇧", "⌘"} {
		if mods[i] {
			s += g
		}
	}
	return s + key
}

// menuShortcuts are the menu bar's items that have keys, a group per
// menu, in the menu's order. The window menu, macOS's own, is left out;
// the app menu shows its 设置….
func menuShortcuts(items []*mygo.MenuItem) []shortcutGroup {
	var groups []shortcutGroup
	for _, top := range items {
		if top.Role != "" && top.Role != mygo.RoleHelp && top.Role != mygo.RoleAppMenu {
			continue
		}
		g := shortcutGroup{title: top.Label}
		for _, it := range top.Submenu {
			keys := roleKeys[it.Role]
			if it.Accelerator != "" {
				keys = keyGlyphs(it.Accelerator)
			}
			if keys != "" && it.Label != "" {
				g.rows = append(g.rows, shortcutRow{it.Label, []string{keys}})
			}
		}
		if len(g.rows) > 0 {
			groups = append(groups, g)
		}
	}
	return groups
}

// showShortcuts opens the 键盘快捷键 window, or brings it forward.
func (a *app) showShortcuts() {
	if a.keysWin != nil && !a.keysWin.IsDestroyed() {
		a.keysWin.Show()
		a.keysWin.Focus()
		return
	}
	groups := append(menuShortcuts(a.menuItems), gestureGroups...)
	var w *mygo.Window
	w = mygo.NewWindow(mygo.WindowOptions{
		Title:             "键盘快捷键",
		Width:             460,
		Height:            640,
		MinWidth:          380,
		MinHeight:         320,
		DisableMinimize:   true,
		DisableMaximize:   true,
		DisableFullScreen: true,
		BackgroundColor:   "light-dark(#f6f7f9, #0f1115)",
		StateKey:          "shortcuts",
		Content: ui.View(func(c *ui.Context) {
			shortcutsView(c, groups, func() { w.Close() })
		}),
	})
	a.keysWin = w
	w.OnClosed(func() {
		if a.keysWin == w {
			a.keysWin = nil
		}
	})
}

// shortcutsView is the content of the 键盘快捷键 window; Esc closes it.
func shortcutsView(c *ui.Context, groups []shortcutGroup, close func()) {
	t := c.Theme()
	if c.Shortcut(0, ui.KeyEscape) && close != nil {
		close()
	}
	ui.Scroll(c).Fill().Background(t.Background).Children(func() {
		ui.Column(c).FillWidth().Padding(20, 24, 28, 24).Gap(22).Children(func() {
			for _, g := range groups {
				ui.Column(c).FillWidth().Gap(2).Children(func() {
					ui.Text(c, g.title).FontSize(12).FontWeight(600).TextColor(t.TextMuted).
						Padding(0, 0, 6, 0)
					for i, r := range g.rows {
						shortcutLine(c, t, r, i > 0)
					}
				})
			}
		})
	})
}

func shortcutLine(c *ui.Context, t *ui.Theme, r shortcutRow, rule bool) {
	row := ui.Row(c).FillWidth().AlignItems(ui.Center).Gap(12).Padding(7, 0, 7, 0)
	if rule {
		row = row.BorderWidth(1, 0, 0, 0).BorderColor(t.Border.Alpha(0.6))
	}
	row.Children(func() {
		ui.Text(c, r.label).FontSize(13).TextColor(t.Text).Grow(1)
		ui.Row(c).Gap(4).Children(func() {
			for _, k := range r.keys {
				ui.Text(c, k).FontSize(12).FontWeight(500).TextColor(t.Text).SingleLine().
					Padding(2, 7, 2, 7).Background(t.Surface).Border(1, t.Border).Radius(5)
			}
		})
	})
}
