//! The window's root: the home screen (open a folder, the folders opened
//! lately), or the deck of the folder open. Each deck is a folder; leaving
//! it keeps what its terminals showed, saves it and hangs the shells up.

use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext, Context, Entity, ExternalPaths, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    actions, canvas, div, point, prelude::FluentBuilder, px, svg,
};

use crate::deck::{DeckEvent, DeckView};
use crate::model::{DECK_DIR, Model};
use crate::recents::{Recents, ago, short_path};
use crate::theme::{Paper, Theme};
use crate::{icons, tape};

actions!(app, [OpenFolder, Quit, About]);

pub struct Root {
    focus: FocusHandle,
    recents: Recents,
    deck: Option<Entity<DeckView>>,
    _deck_sub: Option<Subscription>,
    /// Why the last folder did not open, on the home screen.
    err: Option<String>,
    focus_deck: bool,
}

impl Root {
    pub fn new(open: Option<PathBuf>, cx: &mut Context<Self>) -> Root {
        let mut r = Root {
            focus: cx.focus_handle(),
            recents: Recents::open(),
            deck: None,
            _deck_sub: None,
            err: None,
            focus_deck: false,
        };
        if let Some(p) = open {
            r.open_folder(&p, cx);
        }
        r
    }

    pub fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("打开".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.into_iter().next()
            {
                let _ = this.update(cx, |r, cx| r.open_folder(&p, cx));
            }
        })
        .detach();
    }

    /// Closes the deck open, if any, and opens the folder's.
    pub fn open_folder(&mut self, path: &Path, cx: &mut Context<Self>) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !path.exists() {
            self.err = Some(format!("打不开「{name}」：文件夹不存在或没有权限。"));
            self.recents.mark_missing(&path.to_string_lossy());
            cx.notify();
            return;
        }
        if !path.is_dir() {
            self.err = Some(format!("「{name}」不是文件夹。请打开一个文件夹，或把文件夹拖进窗口。"));
            cx.notify();
            return;
        }
        self.close_deck(cx);
        let root = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut m = Model::new(root.clone());
        m.file = Some(root.join(DECK_DIR).join("deck.json"));
        m.load();
        let blocks = m.blocks.len();
        let workspaces = self.recents.paths();
        let deck = cx.new(|cx| {
            let mut d = DeckView::new(m, workspaces, cx);
            d.start_mcp(cx);
            d
        });
        self._deck_sub = Some(cx.subscribe(&deck, |r: &mut Root, _, ev: &DeckEvent, cx| match ev {
            DeckEvent::Home => r.go_home(cx),
            DeckEvent::Open(p) => r.open_folder(&PathBuf::from(p), cx),
            DeckEvent::Pick => r.pick_folder(cx),
        }));
        self.deck = Some(deck);
        self.err = None;
        self.recents.touch(&root.to_string_lossy(), blocks);
        self.focus_deck = true;
        cx.notify();
    }

    pub fn go_home(&mut self, cx: &mut Context<Self>) {
        self.close_deck(cx);
        self.recents.check();
        cx.notify();
    }

    /// Leaves the deck open: keeps what its terminals show, saves it, hangs the shells up.
    pub fn close_deck(&mut self, cx: &mut Context<Self>) {
        if let Some(deck) = self.deck.take() {
            let (root, blocks) = deck.update(cx, |d, cx| {
                d.close(cx);
                (d.m.root.to_string_lossy().into_owned(), d.m.blocks.len())
            });
            self.recents.touch(&root, blocks);
            self._deck_sub = None;
        }
    }

    fn dropped(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if let Some(p) = paths.iter().find(|p| p.is_dir()) {
            self.open_folder(p, cx);
        } else if !paths.is_empty() {
            self.err = Some("拖进来的是文件。请拖入一个文件夹，它会作为 deck 打开。".into());
            cx.notify();
        }
    }

    fn home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        let paper = Paper::of(&t);
        let now = chrono::Local::now().fixed_offset();
        let dot = t.muted.opacity(0.35);
        div()
            .id("home")
            .size_full()
            .relative()
            .bg(t.background)
            .font_family(".SystemUIFont")
            .on_drop(cx.listener(|r, p: &ExternalPaths, _, cx| r.dropped(p.paths(), cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, ev: &gpui::MouseDownEvent, window, _| {
                    if f32::from(ev.position.y) < 38. {
                        window.start_window_move();
                    }
                }),
            )
            .child(
                // The canvas's dots, under it all.
                canvas(|_, _, _| (), move |b, _, window, _| {
                    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
                    let mut y = 12.;
                    while y < h {
                        let mut x = 12.;
                        while x < w {
                            window.paint_quad(
                                gpui::fill(gpui::Bounds::new(point(px(x), px(y)), gpui::size(px(2.5), px(2.5))), dot)
                                    .corner_radii(px(1.25)),
                            );
                            x += 24.;
                        }
                        y += 24.;
                    }
                })
                .absolute()
                .size_full(),
            )
            .child(
                div().id("home-scroll").absolute().size_full().overflow_y_scroll().child(
                    div().w_full().flex().flex_col().items_center().pt(px(96.)).pb(px(48.)).px(px(24.)).child(
                        div()
                            .w(px(560.))
                            .flex()
                            .flex_col()
                            .gap(px(32.))
                            .child(head(&t))
                            .child(self.recents_block(&t, &paper, now, cx)),
                    ),
                ),
            )
    }

    fn recents_block(
        &self,
        t: &Theme,
        _: &Paper,
        now: chrono::DateTime<chrono::FixedOffset>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (face, outline) = if t.dark { (gpui::black(), t.border) } else { (gpui::white(), t.border) };
        let dark = t.dark;
        let accent = t.accent;
        let mut list = div().w_full().flex().flex_col().p(px(4.)).gap(px(2.));
        if let Some(err) = &self.err {
            list = list.child(div().px(px(12.)).py(px(8.)).text_size(px(13.)).text_color(t.danger).child(err.clone()));
        }
        if self.recents.list.is_empty() {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(20.))
                    .text_size(px(13.))
                    .text_color(t.muted)
                    .child("还没有打开过文件夹。打开一个，它就是一张 deck。"),
            );
        }
        for (i, r) in self.recents.list.iter().enumerate() {
            let path = r.path.clone();
            let p2 = path.clone();
            let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path.clone());
            let missing = r.missing;
            list = list.child(
                div()
                    .id(SharedString::from(format!("recent:{i}")))
                    .group("recent")
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .px(px(12.))
                    .py(px(10.))
                    .hover(|d| d.bg(t.surface_hover))
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(30.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(7.))
                            .bg(accent.opacity(if missing { 0.06 } else { 0.12 }))
                            .child(svg().path(icons::path("folder")).size(px(16.)).text_color(accent.opacity(if missing { 0.4 } else { 1. }))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if missing { t.muted } else { t.text })
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.muted)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(short_path(&path)),
                            ),
                    )
                    .child(if missing {
                        div().text_size(px(12.)).text_color(t.danger).child("找不到这个文件夹")
                    } else {
                        div()
                            .text_size(px(12.))
                            .text_color(t.muted)
                            .whitespace_nowrap()
                            .child(format!("{} 个 block · {}", r.blocks, ago(now, r.opened_at)))
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("forget:{i}")))
                            .invisible()
                            .group_hover("recent", |d| d.visible())
                            .p(px(4.))
                            .rounded(px(5.))
                            .hover(|d| d.bg(t.danger.opacity(0.12)))
                            .child(svg().path(icons::path("x")).size(px(14.)).text_color(t.muted))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |r, _, _, cx| {
                                cx.stop_propagation();
                                r.recents.forget(&p2);
                                cx.notify();
                            })),
                    )
                    .on_click(cx.listener(move |r, _, _, cx| r.open_folder(&PathBuf::from(&path), cx))),
            );
        }
        // The block, drawn as a block on the canvas is: its tape sticking out above.
        div()
            .relative()
            .w_full()
            .child(
                canvas(|_, _, _| (), move |b, _, window, _| {
                    let w = f32::from(b.size.width);
                    let tp = tape::tape_of("home", f32::from(b.origin.x), f32::from(b.origin.y), w, 1.);
                    let tp = tape::Tape { sin: (-2f32).to_radians().sin(), cos: (-2f32).to_radians().cos(), ..tp };
                    tape::paint(window, &tp, 0., 0., tape::Look { hovered: false, held: false, selected: false, dark, accent });
                })
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .flex_col()
                    .bg(face)
                    .border_1()
                    .border_color(outline)
                    .shadow(vec![gpui::BoxShadow {
                        color: t.shade,
                        offset: point(px(0.), px(2.)),
                        blur_radius: px(6.),
                        spread_radius: px(0.),
                    }])
                    .child(
                        div()
                            .w_full()
                            .h(px(46.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .pl(px(14.))
                            .pr(px(8.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("最近的 deck"))
                            .when(!self.recents.list.is_empty(), |d| {
                                d.child(div().text_size(px(12.)).text_color(t.muted).child(self.recents.list.len().to_string()))
                            })
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("open-folder")
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.))
                                    .px(px(12.))
                                    .py(px(6.))
                                    .bg(t.accent)
                                    .hover(|d| d.opacity(0.9))
                                    .cursor_pointer()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.accent_text)
                                    .child(svg().path(icons::path("folder")).size(px(14.)).text_color(t.accent_text))
                                    .child("打开文件夹…")
                                    .on_click(cx.listener(|r, _, _, cx| r.pick_folder(cx))),
                            ),
                    )
                    .child(list),
            )
    }
}

/// The title: "Boundless" rushing on, its trail fading to the left, then "Deck".
fn head(t: &Theme) -> impl IntoElement {
    let word = |c: gpui::Hsla| {
        div().text_size(px(38.)).font_weight(FontWeight::SEMIBOLD).whitespace_nowrap().text_color(c).child("Boundless")
    };
    let mut boundless = div().relative().child(word(t.text.opacity(0.)));
    const TRAIL: i32 = 18;
    for i in (1..=TRAIL).rev() {
        let f = i as f32 / TRAIL as f32;
        boundless = boundless
            .child(word(t.text.opacity(0.09 * (1. - f) * (1. - f))).absolute().left(px(-f * 90.)).top_0());
    }
    boundless = boundless.child(word(t.text).absolute().left_0().top_0());
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div().flex().flex_col().child(boundless).child(
                div()
                    .text_size(px(96.))
                    .line_height(px(86.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(t.text)
                    .child("Deck"),
            ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .text_size(px(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().text_color(t.accent).child("创意"))
                .child(div().text_color(t.text).child("无止境")),
        )
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match &self.deck {
            Some(d) => format!("{} — BoundlessDeck", crate::model::file_name(&d.read(cx).m.root.to_string_lossy())),
            None => "BoundlessDeck".into(),
        };
        window.set_window_title(&title);
        let body = match self.deck.clone() {
            Some(d) => {
                if self.focus_deck {
                    self.focus_deck = false;
                    window.focus(&d.focus_handle(cx));
                }
                d.into_any_element()
            }
            None => self.home(window, cx).into_any_element(),
        };
        div()
            .size_full()
            .key_context("Root")
            .track_focus(&self.focus)
            .on_action(cx.listener(|r, _: &OpenFolder, _, cx| r.pick_folder(cx)))
            .child(body)
    }
}

use gpui::Focusable as _;

pub fn bind_keys(cx: &mut App) {
    use crate::deck::*;
    use gpui::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenFolder, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-0", FitAll, Some("Deck")),
        KeyBinding::new("cmd-=", ZoomIn, Some("Deck")),
        KeyBinding::new("cmd-+", ZoomIn, Some("Deck")),
        KeyBinding::new("cmd--", ZoomOut, Some("Deck")),
        KeyBinding::new("cmd-z", Undo, Some("Deck")),
        KeyBinding::new("cmd-shift-z", Redo, Some("Deck")),
        KeyBinding::new("cmd-d", DeleteSelected, Some("Deck")),
        KeyBinding::new("cmd-n", NewNote, Some("Deck")),
        KeyBinding::new("cmd-shift-n", NewMemo, Some("Deck")),
        KeyBinding::new("cmd-shift-o", NewFile, Some("Deck")),
        KeyBinding::new("cmd-t", NewTerminal, Some("Deck")),
        KeyBinding::new("cmd-shift-b", NewBrowser, Some("Deck")),
        KeyBinding::new("cmd-shift-g", NewChart, Some("Deck")),
        KeyBinding::new("cmd-shift-k", NewStack, Some("Deck")),
        KeyBinding::new("cmd-shift-h", GoHome, Some("Deck")),
    ]);
}

pub fn menus() -> Vec<gpui::Menu> {
    use crate::deck::*;
    use gpui::{Menu, MenuItem};
    vec![
        Menu {
            name: "BoundlessDeck".into(),
            items: vec![MenuItem::action("关于 BoundlessDeck", About), MenuItem::separator(), MenuItem::action("退出 BoundlessDeck", Quit)],
        },
        Menu {
            name: "文件".into(),
            items: vec![MenuItem::action("打开文件夹…", OpenFolder), MenuItem::action("回到主页", GoHome)],
        },
        Menu {
            name: "新建".into(),
            items: vec![
                MenuItem::action("新建 QuickLook", NewNote),
                MenuItem::action("新建笔记", NewMemo),
                MenuItem::action("链接文件…", NewFile),
                MenuItem::action("新建终端", NewTerminal),
                MenuItem::action("新建图表", NewChart),
                MenuItem::action("新建块堆", NewStack),
            ],
        },
        Menu {
            name: "编辑".into(),
            items: vec![
                MenuItem::action("撤销", Undo),
                MenuItem::action("重做", Redo),
                MenuItem::separator(),
                MenuItem::action("剪切", crate::editor::Cut),
                MenuItem::action("拷贝", crate::editor::Copy),
                MenuItem::action("粘贴", crate::editor::Paste),
                MenuItem::action("全选", crate::editor::SelectAll),
                MenuItem::separator(),
                MenuItem::action("删除选中的 block", DeleteSelected),
            ],
        },
        Menu {
            name: "视图".into(),
            items: vec![
                MenuItem::action("显示全部 block", FitAll),
                MenuItem::action("放大", ZoomIn),
                MenuItem::action("缩小", ZoomOut),
            ],
        },
    ]
}
