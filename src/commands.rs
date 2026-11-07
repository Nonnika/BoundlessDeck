//! Every command with its shortcut, in one table: the menu bar, the key
//! bindings and the 键盘快捷键 window are all read from it. The canvas's
//! gestures, which no menu item has, are listed beside them.

use gpui::{Action, App, KeyBinding, Menu, MenuItem, SharedString};

use crate::app::{About, OpenFolder, OpenSettings, Quit, ShowShortcuts};
use crate::deck::*;
use crate::editor;

pub struct Entry {
    pub menu: &'static str,
    pub label: &'static str,
    /// GPUI's keystroke, as "cmd-shift-n".
    pub keys: Option<&'static str>,
    /// The key context the binding applies in.
    pub context: Option<&'static str>,
    pub action: Box<dyn Action>,
    /// A line before it in its menu.
    pub separated: bool,
}

fn e(menu: &'static str, label: &'static str, keys: Option<&'static str>, context: Option<&'static str>, action: impl Action) -> Entry {
    Entry { menu, label, keys, context, action: Box::new(action), separated: false }
}

fn sep(mut e: Entry) -> Entry {
    e.separated = true;
    e
}

const DECK: Option<&str> = Some("Deck");

pub fn entries() -> Vec<Entry> {
    vec![
        e("BoundlessDeck", "关于 BoundlessDeck", None, None, About),
        sep(e("BoundlessDeck", "设置…", Some("cmd-,"), None, OpenSettings)),
        sep(e("BoundlessDeck", "退出 BoundlessDeck", Some("cmd-q"), None, Quit)),
        e("文件", "打开文件夹…", Some("cmd-o"), None, OpenFolder),
        e("文件", "回到主页", Some("cmd-shift-h"), DECK, GoHome),
        e("新建", "新建 QuickLook", Some("cmd-n"), DECK, NewNote),
        e("新建", "新建笔记", Some("cmd-shift-n"), DECK, NewMemo),
        e("新建", "链接文件…", Some("cmd-shift-o"), DECK, NewFile),
        e("新建", "新建终端", Some("cmd-t"), DECK, NewTerminal),
        e("新建", "新建浏览器", Some("cmd-shift-b"), DECK, NewBrowser),
        e("新建", "新建图表", Some("cmd-shift-g"), DECK, NewChart),
        e("新建", "新建块堆", Some("cmd-shift-k"), DECK, NewStack),
        e("编辑", "撤销", Some("cmd-z"), DECK, Undo),
        e("编辑", "重做", Some("cmd-shift-z"), DECK, Redo),
        sep(e("编辑", "剪切", Some("cmd-x"), Some("Editor"), editor::Cut)),
        e("编辑", "拷贝", Some("cmd-c"), Some("Editor"), editor::Copy),
        e("编辑", "粘贴", Some("cmd-v"), Some("Editor"), editor::Paste),
        e("编辑", "全选", Some("cmd-a"), Some("Editor"), editor::SelectAll),
        sep(e("编辑", "删除选中的 block", Some("cmd-d"), DECK, DeleteSelected)),
        e("视图", "显示全部 block", Some("cmd-0"), DECK, FitAll),
        e("视图", "放大", Some("cmd-="), DECK, ZoomIn),
        e("视图", "缩小", Some("cmd--"), DECK, ZoomOut),
        e("帮助", "键盘快捷键", Some("cmd-/"), None, ShowShortcuts),
    ]
}

pub fn bind_keys(cx: &mut App) {
    let mapper = cx.keyboard_mapper().clone();
    let mut bindings = Vec::new();
    for en in entries() {
        let Some(keys) = en.keys else { continue };
        let ctx = en.context.map(|c| gpui::KeyBindingContextPredicate::parse(c).unwrap().into());
        if let Ok(b) = KeyBinding::load(keys, en.action, ctx, false, None, mapper.as_ref()) {
            bindings.push(b);
        }
    }
    // ⌘+ zooms in as ⌘= does.
    bindings.push(KeyBinding::new("cmd-+", ZoomIn, DECK));
    cx.bind_keys(bindings);
}

pub fn menus() -> Vec<Menu> {
    let mut out: Vec<Menu> = Vec::new();
    for en in entries() {
        if out.last().is_none_or(|m| m.name.as_ref() != en.menu) {
            out.push(Menu { name: en.menu.into(), items: Vec::new() });
        }
        let m = out.last_mut().unwrap();
        if en.separated {
            m.items.push(MenuItem::separator());
        }
        m.items.push(MenuItem::Action { name: en.label.into(), action: en.action, os_action: None });
    }
    out
}

/// A keystroke as macOS writes it: "cmd-shift-n" is ⇧⌘N, modifiers ⌃⌥⇧⌘.
pub fn glyphs(keys: &str) -> String {
    let (mut ctrl, mut alt, mut shift, mut cmd) = (false, false, false, false);
    let mut key = String::new();
    let parts: Vec<&str> = if keys.ends_with("--") {
        let mut p: Vec<&str> = keys.trim_end_matches("--").split('-').collect();
        p.push("-");
        p
    } else {
        keys.split('-').collect()
    };
    for p in parts {
        match p {
            "ctrl" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            "cmd" => cmd = true,
            "enter" => key = "↩".into(),
            "escape" => key = "Esc".into(),
            "space" => key = "空格".into(),
            other => key = other.to_uppercase(),
        }
    }
    let mut s = String::new();
    for (on, g) in [(ctrl, "⌃"), (alt, "⌥"), (shift, "⇧"), (cmd, "⌘")] {
        if on {
            s.push_str(g);
        }
    }
    s + &key
}

/// One group of the shortcuts window: its rows of what and how.
pub type Group = (SharedString, Vec<(SharedString, Vec<SharedString>)>);

/// The menu's commands with keys, then the canvas's gestures.
pub fn shortcut_groups() -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for en in entries() {
        let Some(keys) = en.keys else { continue };
        if out.last().is_none_or(|g| g.0.as_ref() != en.menu) {
            out.push((en.menu.into(), Vec::new()));
        }
        out.last_mut().unwrap().1.push((en.label.into(), vec![glyphs(keys).into()]));
    }
    let gestures: [(&str, &[(&str, &[&str])]); 5] = [
        ("画布", &[
            ("新建 QuickLook（在鼠标处）", &["双击空白处"]),
            ("框选", &["拖动空白处"]),
            ("移动画布", &["滚轮 / 双指滑动 / 右键拖动"]),
            ("缩放画布", &["⌘ + 滚轮 / 双指捏合"]),
        ]),
        ("块堆", &[
            ("收进块堆", &["把 block 拖到块堆上"]),
            ("推出卡片", &["悬停块堆中间"]),
            ("滚动卡片", &["悬停上下边缘 / 滚轮"]),
            ("长预览", &["空格"]),
            ("切换卡片", &["←", "→"]),
            ("关闭长预览", &["空格 / Esc"]),
            ("取出卡片", &["拖动卡片标题到块堆外"]),
        ]),
        ("QuickLook 与笔记", &[
            ("结束编辑", &["⌘↩ / Esc"]),
            ("QuickLook 消失时间", &["点击标题栏右侧的剩余时间"]),
            ("找回刚消失的 QuickLook", &["⌘Z"]),
        ]),
        ("图表", &[
            ("编辑数据", &["双击图表 / 标题栏的表格按钮"]),
            ("切换图表类型", &["点击标题栏的类型"]),
            ("查看数值", &["悬停柱子、折线或扇区"]),
        ]),
        ("浏览器", &[
            ("输入网址或搜索", &["点击地址栏，↩ 打开"]),
            ("后退 / 前进 / 刷新", &["地址栏左侧的按钮"]),
        ]),
    ];
    for (group, rows) in gestures {
        out.push((
            group.into(),
            rows.iter().map(|(what, how)| ((*what).into(), how.iter().map(|h| (*h).into()).collect())).collect(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_read_as_macos_writes_them() {
        assert_eq!(glyphs("cmd-shift-n"), "⇧⌘N");
        assert_eq!(glyphs("cmd--"), "⌘-");
        assert_eq!(glyphs("cmd-,"), "⌘,");
        assert_eq!(glyphs("ctrl-alt-cmd-x"), "⌃⌥⌘X");
    }

    #[test]
    fn every_new_kind_has_a_shortcut_and_none_repeats() {
        let es = entries();
        let mut seen = std::collections::HashSet::new();
        for e in &es {
            if let Some(k) = e.keys {
                assert!(seen.insert((k, e.context)), "{k} bound twice");
            }
        }
        assert_eq!(es.iter().filter(|e| e.menu == "新建").count(), 7);
    }
}
