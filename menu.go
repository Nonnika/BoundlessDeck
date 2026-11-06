package main

import (
	"github.com/egoist/mygo"
)

// The menu bar: every command of the deck with its shortcut, so that the
// keys can be found there. Its items work on the deck open; those that
// need one are disabled on the home screen (syncMenu). On macOS the menu
// takes its keys before the window, so its Click is what runs for them;
// the window's own shortcuts (canvas.go, home.go) are the same commands,
// for tests and for platforms without a menu bar.

// newKeys are the shortcuts that add blocks, as the menu and the
// toolbar's tips write them. A block added with one lands under the mouse.
var newKeys = []struct {
	kind              blockKind
	label, acc, glyph string
}{
	{kindNote, "新建 QuickLook", "CmdOrCtrl+N", "⌘N"},
	{kindMemo, "新建笔记", "CmdOrCtrl+Shift+N", "⇧⌘N"},
	{kindFile, "链接文件…", "CmdOrCtrl+Shift+O", "⇧⌘O"},
	{kindTerminal, "新建终端", "CmdOrCtrl+T", "⌘T"},
	{kindBrowser, "新建浏览器", "CmdOrCtrl+Shift+B", "⇧⌘B"},
	{kindStack, "新建块堆", "CmdOrCtrl+Shift+K", "⇧⌘K"},
}

// newKeyOf is the shortcut glyphs that add a block of kind, "" for none.
func newKeyOf(kind blockKind) string {
	for _, k := range newKeys {
		if k.kind == kind {
			return k.glyph
		}
	}
	return ""
}

// deckItems are the IDs of the items that need a deck open.
func deckItems() []string {
	ids := []string{"home", "delete", "fit", "zoom-in", "zoom-out"}
	for _, k := range newKeys {
		ids = append(ids, "new-"+string(k.kind))
	}
	return ids
}

// buildMenu makes the menu bar of the app.
func (a *app) buildMenu() *mygo.Menu {
	// onDeck runs fn on the deck open, as the window's frames do.
	onDeck := func(fn func(d *deck)) func(*mygo.MenuItem, *mygo.Window) {
		return func(*mygo.MenuItem, *mygo.Window) {
			if a.win == nil {
				return
			}
			a.win.Update(func() {
				if a.deck != nil {
					fn(a.deck)
				}
			})
		}
	}

	file := []*mygo.MenuItem{
		{Label: "打开文件夹…", Accelerator: "CmdOrCtrl+O", Click: func(*mygo.MenuItem, *mygo.Window) {
			go a.pickFolder()
		}},
		{ID: "home", Label: "回到主页", Accelerator: "CmdOrCtrl+Shift+H", Click: onDeck(func(d *deck) {
			if d.onHome != nil {
				d.onHome()
			}
		})},
		mygo.Separator(),
		{Role: mygo.RoleClose, Label: "关闭窗口"},
	}

	add := make([]*mygo.MenuItem, 0, len(newKeys))
	for _, k := range newKeys {
		kind := k.kind
		add = append(add, &mygo.MenuItem{
			ID: "new-" + string(kind), Label: k.label, Accelerator: k.acc,
			Click: onDeck(func(d *deck) { d.addAtPointer(kind) }),
		})
	}

	edit := []*mygo.MenuItem{
		{Role: mygo.RoleUndo, Label: "撤销"},
		{Role: mygo.RoleRedo, Label: "重做"},
		mygo.Separator(),
		{Role: mygo.RoleCut, Label: "剪切"},
		{Role: mygo.RoleCopy, Label: "拷贝"},
		{Role: mygo.RolePaste, Label: "粘贴"},
		{Role: mygo.RoleSelectAll, Label: "全选"},
		mygo.Separator(),
		{ID: "delete", Label: "删除选中的 block", Accelerator: "CmdOrCtrl+D", Click: onDeck(func(d *deck) {
			if d.editing == nil {
				d.deleteSelected()
			}
		})},
	}

	view := []*mygo.MenuItem{
		{ID: "fit", Label: "显示全部 block", Accelerator: "CmdOrCtrl+0", Click: onDeck(func(d *deck) {
			d.fitAll(d.viewW, d.viewH)
		})},
		{ID: "zoom-in", Label: "放大", Accelerator: "CmdOrCtrl+=", Click: onDeck(func(d *deck) {
			d.zoomStep(d.viewW/2, d.viewH/2, 1)
		})},
		{ID: "zoom-out", Label: "缩小", Accelerator: "CmdOrCtrl+-", Click: onDeck(func(d *deck) {
			d.zoomStep(d.viewW/2, d.viewH/2, -1)
		})},
		mygo.Separator(),
		{Role: mygo.RoleToggleFullScreen, Label: "进入全屏"},
	}

	// What has no menu item of its own: keys and gestures on the canvas.
	// These items only say so; they hold no accelerator, which would take
	// the keys (space, arrows) from the canvas and the inputs.
	help := []*mygo.MenuItem{}
	for _, line := range []string{
		"双击空白处：新建 QuickLook（在鼠标处）",
		"拖动空白处：框选",
		"滚轮 / 双指滑动：移动画布",
		"⌘ + 滚轮 / 双指捏合：缩放画布",
		"拖 block 的胶带或标题栏到块堆上：收进块堆",
		"点击块堆后悬停中间：推出卡片；悬停上下边缘：滚动",
		"空格：块堆长预览（再按空格或 Esc 关闭）",
		"← / →：长预览里切换卡片",
		"⌘↩ / Esc：结束编辑 QuickLook",
	} {
		help = append(help, &mygo.MenuItem{Label: line, Disabled: true})
	}

	a.menu = mygo.NewMenu([]*mygo.MenuItem{
		{Role: mygo.RoleAppMenu},
		{Label: "文件", Submenu: file},
		{Label: "新建", Submenu: add},
		{Label: "编辑", Submenu: edit},
		{Label: "视图", Submenu: view},
		{Role: mygo.RoleWindowMenu, Label: "窗口"},
		{Label: "快捷键", Submenu: help},
	})
	return a.menu
}

// syncMenu enables the items that need a deck while one is open.
func (a *app) syncMenu() {
	m := a.menu
	if m == nil {
		return
	}
	for _, id := range deckItems() {
		if it := m.ItemByID(id); it != nil {
			it.SetEnabled(a.deck != nil)
		}
	}
}
