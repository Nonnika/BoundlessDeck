//! The window's root: the home screen (open a folder, the folders opened
//! lately), or the deck of the folder open. Each deck is a folder; leaving
//! it keeps what its terminals showed, saves it and hangs the shells up.

pub(crate) mod windows;
pub(crate) mod commands;
pub(crate) mod homescene;
pub(crate) mod recents;
pub(crate) mod welcome;
pub(crate) mod stage;
pub(crate) mod backdrop;

use std::path::{Path, PathBuf};

use gpui::{
    App, AnyView, AppContext, Context, Entity, ExternalPaths, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Subscription,
    Window,
    actions, canvas, div, point, prelude::FluentBuilder, px, svg,
};

use crate::deck::{DeckEvent, DeckView};
use crate::model::{DECK_DIR, Model};
use crate::app::recents::{Recents, ago, short_path};
use crate::ui::theme::{Paper, Theme};
use crate::ui::{icons, tape};

actions!(app, [OpenFolder, Quit, About, OpenSettings, ShowShortcuts, ShowWelcome]);

/// What changes the screen: opening a folder, going home.
type Switch = Box<dyn FnOnce(&mut Root, &mut Context<Root>)>;

pub struct Root {
    focus: FocusHandle,
    recents: Recents,
    deck: Option<Entity<DeckView>>,
    _deck_sub: Option<Subscription>,
    /// Why the last folder did not open, on the home screen.
    err: Option<String>,
    focus_deck: bool,
    pub settings: crate::mcp::config::Settings,
    /// When the app started: the home screen's turning words go by it.
    started: std::time::Instant,
    /// The frames those words take, while the home screen shows.
    ticking: bool,
    /// The turning word and the backdrop, each its own view.
    turning: Entity<Turning>,
    backdrop: Entity<Backdrop>,
    /// What changes the screen, done once the window has blurred out; None
    /// while no switch is under way (see `stage`).
    switching: Option<Switch>,
    /// When the window started blurring out, or coming clear; None at rest.
    stage_from: Option<std::time::Instant>,
    /// How blurred the window is, as the last frame left it.
    blur: f64,
    /// The welcome over the window, while it shows or goes.
    welcome: Option<Entity<crate::app::welcome::WelcomeView>>,
    /// The welcome was just asked for: it takes the keyboard next frame.
    focus_welcome: bool,
    /// The blur behind the welcome, and over its page while it turns.
    welcome_blur: crate::app::backdrop::Backdrop,
    turn_blur: crate::app::backdrop::Backdrop,
}

impl Root {
    pub fn new(open: Option<PathBuf>, cx: &mut Context<Self>) -> Root {
        let started = std::time::Instant::now();
        let mut r = Root {
            focus: cx.focus_handle(),
            recents: Recents::open(),
            deck: None,
            _deck_sub: None,
            err: None,
            focus_deck: false,
            settings: crate::mcp::config::Settings::load(),
            started,
            turning: cx.new(|_| Turning { started }),
            backdrop: cx.new(|_| Backdrop),
            ticking: false,
            switching: None,
            stage_from: None,
            blur: 0.,
            welcome: None,
            focus_welcome: false,
            welcome_blur: crate::app::backdrop::Backdrop::default(),
            turn_blur: crate::app::backdrop::Backdrop::default(),
        };
        crate::ui::theme::set_accent(r.settings.accent.as_deref());
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
                let _ = this.update(cx, |r, cx| r.begin_open(p, cx));
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
        // A deck.json that cannot be read stays as it is: the deck does not open over it.
        if let Err(e) = m.load() {
            self.err = Some(format!("「{name}」的 {DECK_DIR}/deck.json 读不了（{e}），为免覆盖它，没有打开。"));
            cx.notify();
            return;
        }
        let blocks = m.blocks.len();
        let workspaces = self.recents.paths();
        let project_mcp = self.settings.project_mcp;
        let deck = cx.new(|cx| {
            let mut d = DeckView::new(m, workspaces, cx);
            d.project_mcp = project_mcp;
            d.start_mcp(cx);
            d
        });
        self._deck_sub = Some(cx.subscribe(&deck, |r: &mut Root, _, ev: &DeckEvent, cx| match ev {
            DeckEvent::Home => r.begin_home(cx),
            DeckEvent::Open(p) => r.begin_open(PathBuf::from(p), cx),
            DeckEvent::Pick => r.pick_folder(cx),
        }));
        self.deck = Some(deck);
        self.err = None;
        self.recents.touch(&root.to_string_lossy(), blocks);
        self.focus_deck = true;
        crate::app::windows::refresh_settings(cx);
        cx.notify();
    }

    /// Draws with the accent of this id (`theme::ACCENTS`) from now on, in every window.
    pub fn set_accent(&mut self, id: &str, cx: &mut Context<Self>) {
        let default = crate::ui::theme::ACCENTS[0].id;
        self.settings.accent = (id != default).then(|| id.to_string());
        let _ = self.settings.save();
        crate::ui::theme::set_accent(Some(id));
        cx.refresh_windows();
    }

    /// Shows the welcome over the window on its first page, or brings it back there.
    pub fn show_welcome(&mut self, cx: &mut Context<Self>) {
        match &self.welcome {
            Some(w) => w.update(cx, |v, cx| v.reopen(cx)),
            None => self.welcome = Some(cx.new(crate::app::welcome::WelcomeView::new)),
        }
        self.focus_welcome = true;
        cx.notify();
    }

    /// The welcome over the window, faded by how far in it is, with the window
    /// blurred behind it; none once it has gone, and the keyboard goes back.
    fn welcome_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use crate::app::welcome;
        let now = std::time::Instant::now();
        if self.welcome.as_ref().is_some_and(|w| w.read(cx).gone(now)) {
            self.welcome = None;
            match &self.deck {
                Some(_) => self.focus_deck = true,
                None => window.focus(&self.focus),
            }
        }
        let covered = self.welcome.is_some();
        if let Some(d) = &self.deck
            && d.read(cx).covered != covered
        {
            d.update(cx, |d, cx| {
                d.covered = covered;
                cx.notify();
            });
        }
        let Some(w) = self.welcome.clone() else {
            self.welcome_blur.around(window, 0., crate::app::backdrop::Hole::default());
            self.turn_blur.over(window, 0., &[]);
            return None;
        };
        if std::mem::take(&mut self.focus_welcome) {
            window.focus(&w.focus_handle(cx));
        }
        let f = w.read(cx).presence(now);
        if f < 1. {
            window.request_animation_frame();
        }
        let vs = window.viewport_size();
        let (x, y, pw, ph) = welcome::panel_rect(f32::from(vs.width), f32::from(vs.height));
        let y = y + 12. * (1. - f);
        self.welcome_blur.around(window, 14. * f as f64, crate::app::backdrop::Hole { x, y, w: pw, h: ph, r: welcome::RADIUS });
        // Turning a page, the page blurs and comes clear.
        let page = welcome::WelcomeView::page_rect((x, y, pw, ph));
        self.turn_blur.over(window, w.read(cx).turn_blur(now), &[page]);
        let t = crate::ui::theme::Theme::of(window);
        let scrim = gpui::black().opacity(if t.dark { 0.36 } else { 0.14 } * f);
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(scrim)
                .child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(pw))
                        .h(px(ph))
                        .opacity(f)
                        .rounded(px(welcome::RADIUS))
                        .shadow(vec![gpui::BoxShadow {
                            color: t.shade,
                            offset: gpui::point(px(0.), px(12.)),
                            blur_radius: px(40.),
                            spread_radius: px(0.),
                        }])
                        .child(w),
                )
                .into_any_element(),
        )
    }

    /// The folder of the deck open, if one is.
    pub fn deck_root(&self, cx: &App) -> Option<PathBuf> {
        self.deck.as_ref().map(|d| d.read(cx).m.root.clone())
    }

    /// Turns the project-level MCP configs on or off: on, the deck open
    /// writes them now; off, the next decks opened write none.
    pub fn set_project_mcp(&mut self, on: bool, cx: &mut Context<Self>) -> Result<(), String> {
        self.settings.project_mcp = on;
        let mut err = self.settings.save().err();
        if let Some(d) = &self.deck {
            let r = d.update(cx, |d, cx| d.set_project_mcp(on, cx));
            err = err.or(r.err());
        }
        err.map_or(Ok(()), Err)
    }

    /// Opens the folder as the window shows it, blurring out first. A folder
    /// that will not open says why at once.
    pub fn begin_open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_dir() {
            self.open_folder(&path, cx);
            return;
        }
        self.err = None;
        self.switch_to(Box::new(move |r, cx| r.open_folder(&path, cx)), cx);
    }

    /// Goes home as the window shows it, blurring out first.
    pub fn begin_home(&mut self, cx: &mut Context<Self>) {
        self.switch_to(Box::new(|r, cx| r.go_home(cx)), cx);
    }

    /// Does go, which changes the screen, once the window has blurred out. A
    /// switch under way keeps on, and this one is dropped; one coming clear
    /// blurs out again from where it is.
    fn switch_to(&mut self, go: Switch, cx: &mut Context<Self>) {
        if self.switching.is_some() {
            return;
        }
        let into = crate::app::stage::uneased(self.blur / crate::app::stage::BLUR) * crate::app::stage::OUT_MS;
        self.stage_from = Some(std::time::Instant::now() - std::time::Duration::from_secs_f64(into / 1000.));
        self.switching = Some(go);
        cx.notify();
    }

    /// Steps the blur between screens for this frame: blurring out, then
    /// switching at its blurriest, then coming clear.
    fn step_stage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::app::stage::{BLUR, CLEAR_MS, OUT_MS, ease};
        let Some(from) = self.stage_from else { return };
        let ms = from.elapsed().as_secs_f64() * 1000.;
        let r = if self.switching.is_some() {
            if ms >= OUT_MS {
                if let Some(go) = self.switching.take() {
                    go(self, cx);
                }
                self.stage_from = Some(std::time::Instant::now());
                BLUR
            } else {
                BLUR * ease(ms / OUT_MS)
            }
        } else if ms >= CLEAR_MS {
            self.stage_from = None;
            0.
        } else {
            BLUR * (1. - ease(ms / CLEAR_MS))
        };
        if (r - self.blur).abs() > 0.01 {
            crate::app::stage::set_blur(window, r);
            self.blur = r;
        }
        if self.stage_from.is_some() {
            window.request_animation_frame();
        }
    }

    pub fn go_home(&mut self, cx: &mut Context<Self>) {
        self.close_deck(cx);
        self.recents.check();
        crate::app::windows::refresh_settings(cx);
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

    /// Redraws the home screen's turning word when it moves: each frame of a
    /// slide, then not again until the next one starts.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                let wait = this
                    .update(cx, |r, cx| {
                        if r.deck.is_some() {
                            r.ticking = false;
                            return None;
                        }
                        r.turning.update(cx, |_, cx| cx.notify());
                        let ms = r.started.elapsed().as_millis() as u64;
                        let period = crate::app::homescene::ENDLESS_HOLD_MS + crate::app::homescene::ENDLESS_SLIDE_MS;
                        let into = ms % period;
                        Some(if into < crate::app::homescene::ENDLESS_HOLD_MS { crate::app::homescene::ENDLESS_HOLD_MS - into } else { 16 })
                    })
                    .ok()
                    .flatten();
                let Some(ms) = wait else { break };
                cx.background_executor().timer(std::time::Duration::from_millis(ms)).await;
            }
        })
        .detach();
    }

    /// A deck dragged onto "删除文件夹": asks, then moves its folder to the
    /// Trash and forgets it. One no longer there is only forgotten.
    fn trash_deck(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let dir = PathBuf::from(path);
        if !dir.exists() {
            self.recents.forget(path);
            cx.notify();
            return;
        }
        if let Some(why) = crate::app::recents::trash_refusal(&dir, dirs::home_dir().as_deref()) {
            self.err = Some(why);
            cx.notify();
            return;
        }
        let name = crate::model::file_name(path);
        let answer = window.prompt(
            gpui::PromptLevel::Critical,
            &format!("把「{name}」移到废纸篓？"),
            Some(&format!("整个文件夹 {} 连同里面的所有文件都会移到废纸篓，之后可以从废纸篓放回。", short_path(path))),
            &["移到废纸篓", "取消"],
            cx,
        );
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |r, cx| {
                match crate::app::recents::move_to_trash(&dir) {
                    Ok(()) => {
                        r.recents.forget(&path);
                        r.err = None;
                    }
                    Err(e) => r.err = Some(format!("没能移到废纸篓：{e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// While a deck is dragged: two places at the bottom to drop it, each a
    /// glow spreading from its corner (see `paint_corner_glow`): left takes
    /// the folder to the Trash, right takes the deck off the list. The page
    /// fades into the background from both corners. The glow under the
    /// pointer brightens.
    fn drop_zones(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (bg, danger, muted) = (t.background, t.danger, t.muted);
        let vs = window.viewport_size();
        let (w, h) = (f32::from(vs.width), f32::from(vs.height));
        let m = window.mouse_position();
        let (mx, my) = (f32::from(m.x), f32::from(m.y));
        // Which side the pointer is over: Some(true) left, Some(false) right.
        let over = (my > h - DROP_ZONE_H).then_some(mx < w / 2.);
        let zone = |id: &'static str, title: &'static str, note: &'static str, tint: gpui::Hsla, left: bool| {
            let lit = over == Some(left);
            div()
                .id(id)
                .absolute()
                .bottom_0()
                .w(gpui::relative(0.5))
                .h(px(DROP_ZONE_H))
                .when(left, |d| d.left_0().items_start())
                .when(!left, |d| d.right_0().items_end())
                .flex()
                .flex_col()
                .justify_end()
                .gap(px(2.))
                .px(px(32.))
                .pb(px(28.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tint)
                        .opacity(if lit { 1. } else { 0.7 })
                        .child(title),
                )
                .child(div().text_size(px(11.)).text_color(muted).opacity(if lit { 0.9 } else { 0.6 }).child(note))
        };
        div()
            .absolute()
            .inset_0()
            .child(
                canvas(|_, _, _| (), move |b, _, window, _| {
                    let (x0, y0) = (f32::from(b.origin.x), f32::from(b.origin.y));
                    let (bw, bh) = (f32::from(b.size.width), f32::from(b.size.height));
                    let (left, right) = (point(px(x0), px(y0 + bh)), point(px(x0 + bw), px(y0 + bh)));
                    // The page fading into the background, from both corners.
                    let fade = bw * 0.62;
                    paint_corner_glow(window, left, fade, bg, 0.96);
                    paint_corner_glow(window, right, fade, bg, 0.96);
                    // Each place's own colour, brighter under the pointer.
                    let glow = bw * 0.42;
                    paint_corner_glow(window, left, glow, danger, if over == Some(true) { 0.30 } else { 0.10 });
                    paint_corner_glow(window, right, glow, muted, if over == Some(false) { 0.30 } else { 0.10 });
                })
                .absolute()
                .size_full(),
            )
            .child(zone("drop-trash", "删除文件夹", "移到废纸篓", danger, true).on_drop(cx.listener(
                |r, d: &DraggedDeck, window, cx| r.trash_deck(&d.path, window, cx),
            )))
            .child(zone("drop-forget", "从列表移除", "文件夹保留", muted, false).on_drop(cx.listener(
                |r, d: &DraggedDeck, _, cx| {
                    r.recents.forget(&d.path);
                    cx.notify();
                },
            )))
    }

    fn dropped(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if let Some(p) = paths.iter().find(|p| p.is_dir()) {
            self.begin_open(p.clone(), cx);
        } else if !paths.is_empty() {
            self.err = Some("拖进来的是文件。请拖入一个文件夹，它会作为 deck 打开。".into());
            cx.notify();
        }
    }

    fn home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        let paper = Paper::of(&t);
        let now = chrono::Local::now().fixed_offset();
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
                // The 3D canvas, running off into the distance.
                AnyView::from(self.backdrop.clone()).cached(StyleRefinement::default().absolute().size_full()),
            )
            .child(
                div().id("home-scroll").absolute().size_full().overflow_y_scroll().child(
                    div().w_full().flex().flex_col().items_center().pt(px(96.)).pb(px(48.)).px(px(24.)).child(
                        div()
                            .w(px(560.))
                            .flex()
                            .flex_col()
                            .gap(px(32.))
                            .child(self.turning.clone())
                            .child(self.recents_block(&t, &paper, now, cx)),
                    ),
                ),
            )
            .when(cx.has_active_drag(), |d| d.child(self.drop_zones(&t, window, cx)))
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
            let (t2, paper2) = (*t, Paper::of(t));
            let path = r.path.clone();
            let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path.clone());
            let missing = r.missing;
            let meta = if missing {
                "找不到这个文件夹".to_string()
            } else {
                format!("{} 个 block · {}", r.blocks, ago(now, r.opened_at))
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("recent:{i}")))
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
                            .child(
                                canvas(|_, _, _| (), move |b, _, window, _| {
                                    crate::app::homescene::paint_deck_mark(window, b, &t2, &paper2, missing)
                                })
                                .size_full(),
                            ),
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
                                    .child(name.clone()),
                            )
                            // The path, and on its line (so on its baseline) the count and when.
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(12.))
                                    .text_size(px(12.))
                                    .text_color(t.muted)
                                    .whitespace_nowrap()
                                    .child(
                                        div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(short_path(&path)),
                                    )
                                    .child(div().flex_none().when(missing, |d| d.text_color(t.danger)).child(meta.clone())),
                            ),
                    )
                    .on_drag(
                        DraggedDeck { path: path.clone(), name: name.clone(), meta: meta.clone(), missing },
                        |d, _, _, cx| cx.new(|_| d.clone()),
                    )
                    .on_click(cx.listener(move |r, _, _, cx| r.begin_open(PathBuf::from(&path), cx))),
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

/// How tall the drop places at the bottom are.
const DROP_ZONE_H: f32 = 240.;

/// A glow of color spreading from a corner: strongest there (`peak`), gone at
/// `radius`. GPUI draws linear gradients only, so it is many circles about the
/// corner, each faint, the nearer the corner the more of them lying over it.
fn paint_corner_glow(window: &mut Window, at: gpui::Point<gpui::Pixels>, radius: f32, color: gpui::Hsla, peak: f32) {
    const LAYERS: usize = 48;
    // Each layer's alpha, so that all of them together reach `peak` at the corner.
    let a = 1. - (1. - peak.clamp(0., 0.999)).powf(1. / LAYERS as f32);
    for i in 1..=LAYERS {
        // Eased radii: layers crowd near the corner, so the edge fades out softly.
        let r = radius * (i as f32 / LAYERS as f32).powf(1.6);
        let b = gpui::Bounds::new(point(at.x - px(r), at.y - px(r)), gpui::size(px(2. * r), px(2. * r)));
        window.paint_quad(gpui::fill(b, color.opacity(a)).corner_radii(px(r)));
    }
}

/// A recent deck being dragged toward the drop places, and how it looks under the pointer.
#[derive(Clone)]
struct DraggedDeck {
    path: String,
    name: String,
    /// The row's second line: the count and when, or the missing note.
    meta: String,
    missing: bool,
}

impl Render for DraggedDeck {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        let paper = Paper::of(&t);
        let missing = self.missing;
        let (face, outline) = if t.dark { (gpui::black(), t.border) } else { (gpui::white(), t.border) };
        // GPUI lays a drag ghost out at min-content (it is prepainted with
        // AvailableSpace::min_size()), so the width must be given or the row
        // collapses to a sliver of wrapped words. 550 is a list row's width.
        div()
            .w(px(550.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .py(px(10.))
            .bg(face)
            .border_1()
            .border_color(outline)
            .opacity(0.95)
            .shadow(vec![gpui::BoxShadow {
                color: t.shade,
                offset: point(px(0.), px(6.)),
                blur_radius: px(16.),
                spread_radius: px(0.),
            }])
            .child(
                div().size(px(30.)).flex().items_center().justify_center().child(
                    canvas(|_, _, _| (), move |b, _, window, _| {
                        crate::app::homescene::paint_deck_mark(window, b, &t, &paper, missing)
                    })
                    .size_full(),
                ),
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
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(self.name.clone()),
                    )
                    // The path, and on its line (so on its baseline) the count and when.
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(12.))
                            .text_size(px(12.))
                            .text_color(t.muted)
                            .whitespace_nowrap()
                            .child(
                                div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(short_path(&self.path)),
                            )
                            .child(div().flex_none().when(missing, |d| d.text_color(t.danger)).child(self.meta.clone())),
                    ),
            )
    }
}

/// The home screen's 3D backdrop. It is a cached view: the turning word's
/// frames leave it alone, so its plane is painted only when the window changes.
struct Backdrop;

impl Render for Backdrop {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        let paper = Paper::of(&t);
        canvas(|_, _, _| (), move |b, _, window, _| crate::app::homescene::paint(window, b, &t, &paper)).absolute().size_full()
    }
}

/// The turning word of the home title, as its own view: its frames repaint
/// only it, not the window's root.
struct Turning {
    started: std::time::Instant,
}

impl Render for Turning {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        head(Theme::of(window), self.started.elapsed().as_millis() as u64)
    }
}

/// The title: "Boundless" rushing on, its trail fading to the left, then "Deck".
fn head(t: Theme, ms: u64) -> impl IntoElement {
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
    let t2 = t;
    // "创意无止境": its first word takes turns with the others, each sliding
    // up out of the way of the next.
    use crate::app::homescene::{ENDLESS_WORDS, endless_at};
    let (w, slide) = endless_at(ms);
    let e = 1. - (1. - slide).powi(3);
    let line_h = 30.;
    // Mid-slide the word smears: faint copies trail below it, strongest at the
    // middle of the slide and gone at rest, so the change reads as motion blur.
    const GHOSTS: usize = 4;
    const SMEAR_PX: f32 = 10.;
    let smear = (slide * std::f32::consts::PI).sin();
    let ghosts = if smear > 0. { GHOSTS } else { 0 };
    let word_at = |text: &'static str, dy: f32, a: f32| {
        let mut group = div().absolute().left_0().top(px(dy)).w(px(44.)).h(px(line_h));
        for j in 1..=ghosts {
            let f = j as f32 / GHOSTS as f32;
            group = group.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(f * SMEAR_PX * smear))
                    .h(px(line_h))
                    .whitespace_nowrap()
                    .text_color(t.accent.opacity(a * 0.18 * (1. - f) * smear))
                    .child(text),
            );
        }
        group.child(
            div().absolute().left_0().top_0().h(px(line_h)).whitespace_nowrap().text_color(t.accent.opacity(a)).child(text),
        )
    };
    let mut turning = div().relative().w(px(44.)).h(px(line_h)).overflow_hidden();
    turning = if slide == 0. {
        turning.child(word_at(ENDLESS_WORDS[w], 0., 1.))
    } else {
        turning
            .child(word_at(ENDLESS_WORDS[w], -e * line_h, 1. - e))
            .child(word_at(ENDLESS_WORDS[(w + 1) % ENDLESS_WORDS.len()], (1. - e) * line_h, e))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap(px(20.))
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
                    canvas(|_, _, _| (), move |b, _, window, _| crate::app::homescene::paint_robot(window, b, &t2))
                        .size(px(88.))
                        .mb(px(4.)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .text_size(px(22.))
                .line_height(px(line_h))
                .font_weight(FontWeight::SEMIBOLD)
                .child(turning)
                .child(div().text_color(t.text).child("无止境")),
        )
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Before the body: at the blurriest, the screen changes here.
        self.step_stage(window, cx);
        let title = match &self.deck {
            Some(d) => format!("{} — BoundlessDeck", crate::model::file_name(&d.read(cx).m.root.to_string_lossy())),
            None => "BoundlessDeck".into(),
        };
        window.set_window_title(&title);
        let welcome = self.welcome_overlay(window, cx);
        let body = match self.deck.clone() {
            Some(d) => {
                if self.focus_deck {
                    self.focus_deck = false;
                    window.focus(&d.focus_handle(cx));
                }
                d.into_any_element()
            }
            None => {
                self.tick(cx);
                self.home(window, cx).into_any_element()
            }
        };
        div()
            .size_full()
            .key_context("Root")
            .track_focus(&self.focus)
            .on_action(cx.listener(|r, _: &OpenFolder, _, cx| r.pick_folder(cx)))
            .relative()
            .child(body)
            .children(welcome)
            // Blurring out to another screen, nothing takes input.
            .when(self.switching.is_some(), |d| d.child(div().absolute().inset_0().occlude()))
    }
}

use gpui::Focusable as _;
