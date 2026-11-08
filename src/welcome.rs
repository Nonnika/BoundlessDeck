//! 欢迎使用: a small window of a few pages telling what the app does — a
//! folder is a deck, the kinds of block, moving about the canvas, agents. It
//! opens by itself the first time the app starts (`Settings::welcomed`), and
//! again from 帮助 › 欢迎使用. Shortcuts are read from the commands table.

use gpui::{
    App, AppContext, Context, FocusHandle, Focusable, FontWeight, Global, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, WindowHandle,
    canvas, div, prelude::FluentBuilder, px, svg,
};

use crate::theme::Theme;
use crate::{commands, icons, windows};

/// What a row shows on its right.
enum Keys {
    /// The shortcut of the menu command with this label.
    Menu(&'static str),
    /// A gesture, in words.
    Gesture(&'static str),
    None,
}

struct Row {
    icon: &'static str,
    what: &'static str,
    about: &'static str,
    keys: Keys,
}

enum Hero {
    Robot,
    Icon(&'static str),
}

struct Page {
    hero: Hero,
    title: &'static str,
    lead: &'static str,
    rows: &'static [Row],
}

const fn row(icon: &'static str, what: &'static str, about: &'static str, keys: Keys) -> Row {
    Row { icon, what, about, keys }
}

const PAGES: &[Page] = &[
    Page {
        hero: Hero::Robot,
        title: "欢迎使用 BoundlessDeck",
        lead: "一张没有边界的画布，把笔记、文件、终端和网页摊开放在一起。\n一个文件夹就是一张 deck，画布就保存在这个文件夹里。",
        rows: &[
            row("folder", "打开文件夹", "选一个文件夹，或把它拖进窗口，它就成了一张 deck", Keys::Menu("打开文件夹…")),
            row("save", "自动保存", "画布存在文件夹的 .boundlessdeck/deck.json 里，不用手动保存", Keys::None),
            row("back", "回到主页", "主页列出最近打开的 deck；把一项拖到窗口底部可以移除或删除", Keys::Menu("回到主页")),
        ],
    },
    Page {
        hero: Hero::Icon("stack"),
        title: "画布上的 block",
        lead: "画布上的每一样东西都是一个 block。在 deck 里用快捷键或「新建」菜单添加。",
        rows: &[
            row("note", "QuickLook", "随手记下的便签，到时间会自动消失", Keys::Menu("新建 QuickLook")),
            row("memo", "笔记", "Markdown 笔记，保存为文件夹里的文件", Keys::Menu("新建笔记")),
            row("file", "文件", "预览文本、Markdown、图片、PDF、Word 和视频", Keys::Menu("链接文件…")),
            row("terminal", "终端", "完整的 shell，支持输入法", Keys::Menu("新建终端")),
            row("globe", "浏览器", "内嵌网页，可以开多个标签页", Keys::Menu("新建浏览器")),
            row("chart", "图表", "柱状图、折线图、面积图和饼图，双击编辑数据", Keys::Menu("新建图表")),
            row("stack", "块堆", "把 block 拖进去收成一叠，悬停时翻看", Keys::Menu("新建块堆")),
        ],
    },
    Page {
        hero: Hero::Icon("move"),
        title: "在画布上移动",
        lead: "画布可以向任何方向延伸。缩放时文字重新排版，放多大都清晰。",
        rows: &[
            row("move", "平移", "双指滑动、滚动滚轮，或按住右键拖动", Keys::Gesture("双指滑动")),
            row("zoom_in", "缩放", "双指捏合，或按住 ⌘ 滚动滚轮", Keys::Gesture("双指捏合")),
            row("select", "框选", "在空白处拖动，选中多个 block 一起移动", Keys::Gesture("拖动空白处")),
            row("plus", "随手记一笔", "双击空白处，在鼠标位置新建 QuickLook", Keys::Gesture("双击空白处")),
            row("fit", "显示全部", "把所有 block 收进视野", Keys::Menu("显示全部 block")),
            row("undo", "撤销", "移动、删除、排版都能撤销，消失的 QuickLook 也能找回", Keys::Menu("撤销")),
        ],
    },
    Page {
        hero: Hero::Icon("bot"),
        title: "和 agent 一起用",
        lead: "打开 deck 时，BoundlessDeck 会启动一个 MCP 端点。agent 可以读写画布上的 block、排版、移动视野，它做的改动同样可以撤销。",
        rows: &[
            row("plug", "一键安装", "在设置里为 Claude Code、Cursor、VS Code、Codex 等装好全局配置", Keys::Menu("设置…")),
            row("terminal", "在终端里启动 agent", "agent 知道自己运行在哪个终端 block 里，可以把结果放在旁边", Keys::None),
            row("keyboard", "键盘快捷键", "所有快捷键与手势都列在这里，随时可查", Keys::Menu("键盘快捷键")),
        ],
    },
];

/// The shortcut of the menu command labelled so, as macOS writes it.
fn menu_keys(label: &str) -> Option<String> {
    commands::entries().into_iter().find(|e| e.label == label).and_then(|e| e.keys).map(commands::glyphs)
}

struct Open(WindowHandle<WelcomeView>);
impl Global for Open {}

pub struct WelcomeView {
    focus: FocusHandle,
    page: usize,
    /// Each page's rows' keycaps, read from the commands table once.
    keys: Vec<Vec<Option<SharedString>>>,
}

impl Focusable for WelcomeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Whether the welcome has not been shown yet: the first time the app starts.
pub fn due(cx: &App) -> bool {
    windows::root(cx).is_some_and(|r| !r.read(cx).settings.welcomed)
}

/// Opens the welcome window on its first page, or brings it forward. Once
/// shown, it does not open by itself again.
pub fn show(cx: &mut App) {
    if let Some(r) = windows::root(cx) {
        r.update(cx, |r, _| {
            if !r.settings.welcomed {
                r.settings.welcomed = true;
                let _ = r.settings.save();
            }
        });
    }
    if let Some(w) = cx.try_global::<Open>().map(|o| o.0)
        && w.update(cx, |v, window, cx| {
            v.go(0, cx);
            window.activate_window();
        })
        .is_ok()
    {
        return;
    }
    let mut opts = windows::options("欢迎使用 BoundlessDeck", 600., 620., cx);
    opts.is_resizable = false;
    if let Ok(w) = cx.open_window(opts, |window, cx| {
        let keys = PAGES
            .iter()
            .map(|p| {
                p.rows
                    .iter()
                    .map(|r| match r.keys {
                        Keys::Menu(label) => menu_keys(label).map(SharedString::from),
                        Keys::Gesture(g) => Some(g.into()),
                        Keys::None => None,
                    })
                    .collect()
            })
            .collect();
        let v = cx.new(|cx| WelcomeView { focus: cx.focus_handle(), page: 0, keys });
        window.focus(&v.focus_handle(cx));
        v
    }) {
        cx.set_global(Open(w));
    }
}

impl WelcomeView {
    fn go(&mut self, page: usize, cx: &mut Context<Self>) {
        self.page = page.min(PAGES.len() - 1);
        cx.notify();
    }

    fn last(&self) -> bool {
        self.page + 1 == PAGES.len()
    }

    /// Next page, or on the last one, done.
    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.last() {
            window.remove_window();
        } else {
            self.go(self.page + 1, cx);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => window.remove_window(),
            "right" | "enter" => self.next(window, cx),
            "left" if self.page > 0 => self.go(self.page - 1, cx),
            _ => {}
        }
    }
}

fn hero(h: &Hero, t: &Theme) -> gpui::AnyElement {
    match h {
        Hero::Robot => {
            let t = *t;
            canvas(|_, _, _| (), move |b, _, window, _| crate::homescene::paint_robot(window, b, &t))
                .size(px(64.))
                .into_any_element()
        }
        Hero::Icon(name) => div()
            .size(px(56.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(14.))
            .bg(t.accent.opacity(0.14))
            .child(svg().path(icons::path(name)).size(px(28.)).text_color(t.accent))
            .into_any_element(),
    }
}

fn keycap(text: SharedString, t: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(7.))
        .py(px(2.))
        .rounded(px(5.))
        .border_1()
        .border_color(t.border)
        .bg(t.background)
        .text_size(px(12.))
        .text_color(t.muted)
        .whitespace_nowrap()
        .child(text)
}

fn button(id: &'static str, label: &'static str, primary: bool, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(16.))
        .py(px(7.))
        .rounded(px(7.))
        .text_size(px(13.))
        .cursor_pointer()
        .when(primary, |d| d.bg(t.accent).text_color(t.accent_text).font_weight(FontWeight::SEMIBOLD).hover(|d| d.opacity(0.9)))
        .when(!primary, |d| d.border_1().border_color(t.border).text_color(t.text).hover(|d| d.bg(t.surface_hover)))
        .child(label)
}

impl Render for WelcomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        let page = &PAGES[self.page];
        let keys = &self.keys[self.page];
        let rows = div().flex().flex_col().rounded(px(10.)).border_1().border_color(t.border).bg(t.surface).overflow_hidden().children(
            page.rows.iter().enumerate().map(|(i, r)| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(9.))
                    .when(i > 0, |d| d.border_t_1().border_color(t.border.opacity(0.6)))
                    .child(svg().path(icons::path(r.icon)).flex_none().size(px(18.)).text_color(t.accent))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(r.what))
                            .child(div().text_size(px(12.)).text_color(t.muted).child(r.about)),
                    )
                    .children(keys[i].clone().map(|k| keycap(k, &t)))
            }),
        );
        let dots = div().flex().flex_row().gap(px(4.)).children((0..PAGES.len()).map(|i| {
            let on = i == self.page;
            div()
                .id(("welcome-dot", i))
                .p(px(4.))
                .cursor_pointer()
                .child(div().h(px(6.)).w(px(if on { 18. } else { 6. })).rounded_full().bg(if on { t.accent } else { t.border }))
                .on_click(cx.listener(move |v, _, _, cx| v.go(i, cx)))
        }));
        let back = if self.page == 0 {
            button("welcome-skip", "跳过", false, &t).on_click(cx.listener(|_, _, window, _| window.remove_window()))
        } else {
            button("welcome-back", "上一步", false, &t).on_click(cx.listener(|v, _, _, cx| v.go(v.page - 1, cx)))
        };
        let last = self.last();
        div()
            .id("welcome")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background)
            .font_family(".SystemUIFont")
            .child(
                div()
                    .id("welcome-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .px(px(32.))
                            .pt(px(28.))
                            .pb(px(20.))
                            .child(hero(&page.hero, &t))
                            .child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).text_color(t.text).child(page.title))
                            .child(div().text_size(px(13.)).line_height(px(20.)).text_color(t.muted).child(page.lead))
                            .child(div().mt(px(6.)).child(rows)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(32.))
                    .py(px(16.))
                    .border_t_1()
                    .border_color(t.border.opacity(0.6))
                    .child(dots)
                    .child(div().flex_1())
                    .child(back)
                    .when(last, |d| {
                        d.child(button("welcome-settings", "打开设置", false, &t).on_click(|_, _, cx| windows::show_settings(cx)))
                    })
                    .child(
                        button("welcome-next", if last { "开始使用" } else { "继续" }, true, &t)
                            .on_click(cx.listener(|v, _, window, cx| v.next(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_named_is_in_the_commands_table() {
        for p in PAGES {
            for r in p.rows {
                if let Keys::Menu(label) = r.keys {
                    assert!(menu_keys(label).is_some(), "「{label}」不在命令表里或没有快捷键");
                }
            }
        }
    }

    #[test]
    fn every_icon_named_is_an_asset() {
        use gpui::AssetSource;
        for p in PAGES {
            let hero = match p.hero {
                Hero::Icon(n) => Some(n),
                Hero::Robot => None,
            };
            for name in p.rows.iter().map(|r| r.icon).chain(hero) {
                assert!(icons::Assets.load(&icons::path(name)).unwrap().is_some(), "没有图标 {name}");
            }
        }
    }
}
