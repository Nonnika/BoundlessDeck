//! 欢迎使用: a panel drawn in the main window, over the rest blurred
//! (`backdrop`), of a few pages telling what the app does. Each page is an
//! abstract animation of the thing, a title, one line and the keys for it;
//! nothing scrolls. It opens by itself the first time the app starts
//! (`Settings::welcomed`), and again from 帮助 › 欢迎使用. Shortcuts are read
//! from the commands table. `Root` lays it out (`panel_rect`) and fades it.

use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, PathBuilder, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Window, canvas,
    div, fill, point, prelude::FluentBuilder, px, size,
};

use crate::theme::{Paper, Theme};
use crate::{commands, windows};

/// What a page's keys show: a menu command's shortcut, or a gesture in words.
#[derive(Clone, Copy)]
enum Key {
    Menu(&'static str),
    Gesture(&'static str),
}

#[derive(Clone, Copy)]
enum Anim {
    Folder,
    Blocks,
    Styled,
    Navigate,
    Focus,
    Agent,
}

struct Page {
    anim: Anim,
    title: &'static str,
    line: &'static str,
    /// Each: the keys, and what they do in a word or two.
    keys: &'static [(Key, &'static str)],
}

const PAGES: &[Page] = &[
    Page {
        anim: Anim::Folder,
        title: "一个文件夹，一张画布",
        line: "打开一个文件夹，画布就保存在里面，不用手动保存。",
        keys: &[(Key::Menu("打开文件夹…"), "打开文件夹")],
    },
    Page {
        anim: Anim::Blocks,
        title: "什么都是 block",
        line: "笔记、文件、终端、网页和图表，摊开摆在一起。",
        keys: &[(Key::Menu("新建笔记"), "笔记"), (Key::Menu("新建终端"), "终端"), (Key::Menu("新建浏览器"), "网页")],
    },
    Page {
        anim: Anim::Styled,
        title: "文字与图形",
        line: "文字按 Markdown 排版，拖动数值调大小，从屏幕上取色。",
        keys: &[(Key::Menu("新建文字"), "文字"), (Key::Menu("新建图形"), "图形"), (Key::Gesture("左右拖动数值"), "调大小")],
    },
    Page {
        anim: Anim::Navigate,
        title: "画布没有边界",
        line: "平移、缩放、框选，放多大文字都清晰。",
        keys: &[(Key::Gesture("双指滑动"), "平移"), (Key::Gesture("双指捏合"), "缩放"), (Key::Menu("显示全部 block"), "看全部")],
    },
    Page {
        anim: Anim::Focus,
        title: "焦点与放映",
        line: "把视野存成焦点，连成一条线，像幻灯片一样一站站讲下去。",
        keys: &[(Key::Menu("把当前视野存为焦点"), "存焦点"), (Key::Menu("放映焦点"), "放映"), (Key::Gesture("空格"), "下一个")],
    },
    Page {
        anim: Anim::Agent,
        title: "和 agent 一起用",
        line: "agent 通过 MCP 整理画布、报告进度，改动都能撤销。",
        keys: &[(Key::Menu("设置…"), "一键安装"), (Key::Menu("撤销"), "撤销")],
    },
];

/// The panel's width, and its height besides the animation.
const PANEL_W: f32 = 520.;
const TEXT_H: f32 = 196.;
const ANIM_H: (f32, f32) = (130., 260.);
pub const RADIUS: f32 = 16.;
/// How long the panel takes to come and to go.
const IN_TIME: Duration = Duration::from_millis(280);
const OUT_TIME: Duration = Duration::from_millis(200);
/// One round of a page's animation, then it starts over.
const LOOP: f32 = 5.2;
/// The moment shown, still, when the system asks for less motion.
const STILL: f32 = 3.6;
/// The footer's height: the rest of the panel is the page.
const FOOTER_H: f32 = 72.;
/// How long a turn of the page takes, how far the pages move, and how
/// blurred they get halfway.
const SWITCH_TIME: Duration = Duration::from_millis(420);
const SWITCH_SHIFT: f32 = 72.;
const SWITCH_BLUR: f64 = 7.;
/// Room left under a page's keys. The blur over a turning page takes in a
/// little past its edge; this keeps the footer's buttons out of it, or their
/// color would glow at the page's foot.
const PAGE_FOOT: f32 = 14.;

/// The shortcut of the menu command labelled so, as macOS writes it.
fn menu_keys(label: &str) -> Option<String> {
    commands::entries().into_iter().find(|e| e.label == label).and_then(|e| e.keys).map(commands::glyphs)
}

/// Whether the welcome has not been shown yet: the first time the app starts.
pub fn due(cx: &App) -> bool {
    windows::root(cx).is_some_and(|r| !r.read(cx).settings.welcomed)
}

/// Shows the welcome over the main window, on its first page. Once shown, it
/// does not open by itself again.
pub fn show(cx: &mut App) {
    if let Some(r) = windows::root(cx) {
        r.update(cx, |r, cx| {
            if !r.settings.welcomed {
                r.settings.welcomed = true;
                let _ = r.settings.save();
            }
            r.show_welcome(cx);
        });
    }
    // Asked for from 设置, the main window is behind: it comes forward.
    if let Some(w) = cx.windows().into_iter().find_map(|w| w.downcast::<crate::app::Root>()) {
        let _ = w.update(cx, |_, window, _| window.activate_window());
    }
}

/// Where the panel lies in a window of this size: x, y, w, h.
pub fn panel_rect(vw: f32, vh: f32) -> (f32, f32, f32, f32) {
    let w = PANEL_W.min(vw - 32.);
    let h = (TEXT_H + ANIM_H.1).min((vh - 32.).max(TEXT_H + ANIM_H.0));
    ((vw - w) / 2., (vh - h) / 2., w, h)
}

pub struct WelcomeView {
    focus: FocusHandle,
    page: usize,
    /// When the page showing began: its animation runs from there.
    page_at: Instant,
    opened: Instant,
    closing: Option<Instant>,
    /// Turning from another page: it, when its animation began, when the
    /// turn began, and which way (1 on to the next, -1 back).
    switch: Option<(usize, Instant, Instant, f32)>,
    /// Each page's keycaps, read from the commands table once.
    keys: Vec<Vec<Option<SharedString>>>,
}

impl Focusable for WelcomeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl WelcomeView {
    pub fn new(cx: &mut Context<Self>) -> WelcomeView {
        let keys = PAGES
            .iter()
            .map(|p| {
                p.keys
                    .iter()
                    .map(|(k, _)| match k {
                        Key::Menu(label) => menu_keys(label).map(SharedString::from),
                        Key::Gesture(g) => Some((*g).into()),
                    })
                    .collect()
            })
            .collect();
        let now = Instant::now();
        WelcomeView { focus: cx.focus_handle(), page: 0, page_at: now, opened: now, closing: None, switch: None, keys }
    }

    /// How far in the panel is: 0 gone, 1 fully shown.
    pub fn presence(&self, now: Instant) -> f32 {
        if crate::stage::reduce_motion() {
            return if self.closing.is_some() { 0. } else { 1. };
        }
        let f = |since: Instant, len: Duration| now.saturating_duration_since(since).as_secs_f32() / len.as_secs_f32();
        let shown = crate::stage::ease(f(self.opened, IN_TIME).into()) as f32;
        match self.closing {
            Some(at) => shown * (1. - crate::stage::ease(f(at, OUT_TIME).into()) as f32),
            None => shown,
        }
    }

    /// Whether it has gone, and can be dropped.
    pub fn gone(&self, now: Instant) -> bool {
        self.closing.is_some() && self.presence(now) <= 0.
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.closing.is_none() {
            self.closing = Some(Instant::now());
            cx.notify();
        }
    }

    /// Back to the first page, and in again if it was going.
    pub fn reopen(&mut self, cx: &mut Context<Self>) {
        if self.closing.take().is_some() {
            self.opened = Instant::now();
        }
        self.go(0, cx);
    }

    fn go(&mut self, page: usize, cx: &mut Context<Self>) {
        let page = page.min(PAGES.len() - 1);
        if page != self.page {
            let now = Instant::now();
            if !crate::stage::reduce_motion() {
                let dir = if page > self.page { 1. } else { -1. };
                self.switch = Some((self.page, self.page_at, now, dir));
            }
            self.page = page;
            self.page_at = now;
        }
        cx.notify();
    }

    /// How far a turn of the page has come, 0 to 1, eased; none at rest.
    fn turn(&self, now: Instant) -> Option<f32> {
        let (_, _, at, _) = self.switch?;
        let f = now.saturating_duration_since(at).as_secs_f32() / SWITCH_TIME.as_secs_f32();
        (f < 1.).then(|| crate::stage::ease(f.into()) as f32)
    }

    /// How blurred the page is now: most halfway through a turn.
    pub fn turn_blur(&self, now: Instant) -> f64 {
        self.turn(now).map_or(0., |e| SWITCH_BLUR * (std::f64::consts::PI * e as f64).sin())
    }

    /// Where the page is blurred while it turns, in a panel at x, y, w, h:
    /// inside the border, and stopping short of the footer.
    pub fn page_rect(panel: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        (panel.0 + 6., panel.1 + 6., panel.2 - 12., panel.3 - FOOTER_H - 6. - PAGE_FOOT / 2.)
    }

    /// Page `ix`: its animation `time` seconds in, its title, line and keys.
    fn page_body(&self, ix: usize, time: f32, t: &Theme) -> gpui::Div {
        let page = &PAGES[ix];
        let anim = page.anim;
        let th = *t;
        let stage = canvas(|_, _, _| (), move |b, _, window, _| paint(anim, b, time, &th, window)).size_full();
        let keys = div().flex().flex_row().flex_wrap().justify_center().gap(px(14.)).children(
            page.keys.iter().zip(&self.keys[ix]).filter_map(|((_, what), cap)| {
                let cap = cap.clone()?;
                Some(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .child(keycap(cap, t))
                        .child(div().text_size(px(12.)).text_color(t.muted).child(*what)),
                )
            }),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().m(px(10.)).rounded(px(RADIUS - 6.)).bg(t.background).overflow_hidden().child(stage))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .px(px(28.))
                    .pt(px(10.))
                    .pb(px(PAGE_FOOT))
                    .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).text_color(t.text).child(page.title))
                    .child(div().text_size(px(13.)).text_color(t.muted).child(page.line))
                    .child(div().mt(px(6.)).child(keys)),
            )
    }

    fn last(&self) -> bool {
        self.page + 1 == PAGES.len()
    }

    /// Next page, or on the last one, done.
    fn next(&mut self, cx: &mut Context<Self>) {
        if self.last() {
            self.close(cx);
        } else {
            self.go(self.page + 1, cx);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => self.close(cx),
            "right" | "enter" => self.next(cx),
            "left" if self.page > 0 => self.go(self.page - 1, cx),
            _ => {}
        }
        cx.stop_propagation();
    }
}

/// A page laid over the others, `dx` to the side and faded to `alpha`.
fn page_layer(body: gpui::Div, dx: f32, alpha: f32) -> gpui::Div {
    div().absolute().top_0().bottom_0().left(px(dx)).w_full().opacity(alpha).child(body)
}

fn keycap(text: SharedString, t: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(5.))
        .border_1()
        .border_color(t.border)
        .bg(t.background)
        .text_size(px(11.))
        .text_color(t.text)
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
        let now = Instant::now();
        let still = crate::stage::reduce_motion();
        if !still {
            window.request_animation_frame();
        }
        let time_of = |at: Instant| if still { STILL } else { now.saturating_duration_since(at).as_secs_f32() % LOOP };
        // Turning the page: the old one goes off to the side, the new one
        // comes in from the other, crossing faded; `Root` blurs them meanwhile.
        let turn = self.turn(now);
        if turn.is_none() {
            self.switch = None;
        }
        let mut pages = div().flex_1().min_h_0().relative().overflow_hidden();
        match (turn, self.switch) {
            (Some(e), Some((from, from_at, _, dir))) => {
                pages = pages
                    .child(page_layer(self.page_body(from, time_of(from_at), &t), -dir * SWITCH_SHIFT * e, 1. - e))
                    .child(page_layer(self.page_body(self.page, time_of(self.page_at), &t), dir * SWITCH_SHIFT * (1. - e), e));
            }
            _ => pages = pages.child(page_layer(self.page_body(self.page, time_of(self.page_at), &t), 0., 1.)),
        }
        let dots = div().flex().flex_row().gap(px(2.)).children((0..PAGES.len()).map(|i| {
            let on = i == self.page;
            div()
                .id(("welcome-dot", i))
                .p(px(4.))
                .cursor_pointer()
                .child(div().h(px(6.)).w(px(if on { 18. } else { 6. })).rounded_full().bg(if on { t.accent } else { t.border }))
                .on_click(cx.listener(move |v, _, _, cx| v.go(i, cx)))
        }));
        let back = if self.page == 0 {
            button("welcome-skip", "跳过", false, &t).on_click(cx.listener(|v, _, _, cx| v.close(cx)))
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
            .overflow_hidden()
            .rounded(px(RADIUS))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .font_family(".SystemUIFont")
            .child(pages)
            .child(
                div()
                    .flex_none()
                    .h(px(FOOTER_H))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    // The buttons keep off the edge: the blur outside takes in what lies next to it.
                    .px(px(28.))
                    .pb(px(6.))
                    .child(dots)
                    .child(div().flex_1())
                    .child(back)
                    .when(last, |d| {
                        d.child(button("welcome-settings", "打开设置", false, &t).on_click(|_, _, cx| windows::show_settings(cx)))
                    })
                    .child(
                        button("welcome-next", if last { "开始使用" } else { "继续" }, true, &t)
                            .on_click(cx.listener(|v, _, _, cx| v.next(cx))),
                    ),
            )
    }
}

// ---- The animations ----
//
// Each draws in the stage's bounds at `time` seconds into its round. Shapes
// only, in the theme's colors; the round fades out at its end and starts over.

/// Eased progress from 0 at `a` to 1 at `b` seconds.
fn seg(time: f32, a: f32, b: f32) -> f32 {
    let f = ((time - a) / (b - a)).clamp(0., 1.);
    if f < 0.5 { 4. * f * f * f } else { 1. - (-2. * f + 2.).powi(3) / 2. }
}

fn lerp(a: f32, b: f32, f: f32) -> f32 {
    a + (b - a) * f
}

struct Pen<'a> {
    window: &'a mut Window,
    /// The stage's top-left, and how much is shown (fading out at the round's end).
    x0: f32,
    y0: f32,
    alpha: f32,
}

impl Pen<'_> {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Hsla) {
        if w <= 0. || h <= 0. {
            return;
        }
        let b = Bounds::new(point(px(self.x0 + x), px(self.y0 + y)), size(px(w), px(h)));
        self.window.paint_quad(fill(b, c.opacity(c.a * self.alpha)).corner_radii(px(r.min(w / 2.).min(h / 2.))));
    }

    /// A rect with a 1px border and a fill.
    #[allow(clippy::too_many_arguments)]
    fn card(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, bg: Hsla, border: Hsla) {
        self.rect(x - 1., y - 1., w + 2., h + 2., r + 1., border);
        self.rect(x, y, w, h, r, bg);
    }

    /// A rect drawn by its 1px border over a (maybe see-through) fill.
    #[allow(clippy::too_many_arguments)]
    fn outline(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, bg: Hsla, border: Hsla) {
        if w <= 0. || h <= 0. {
            return;
        }
        let b = Bounds::new(point(px(self.x0 + x), px(self.y0 + y)), size(px(w), px(h)));
        let a = self.alpha;
        self.window.paint_quad(gpui::quad(
            b,
            px(r),
            bg.opacity(bg.a * a),
            px(1.),
            border.opacity(border.a * a),
            gpui::BorderStyle::Solid,
        ));
    }

    /// A filled shape through the points.
    fn poly(&mut self, pts: &[(f32, f32)], c: Hsla) {
        let mut b = PathBuilder::fill();
        b.move_to(point(px(self.x0 + pts[0].0), px(self.y0 + pts[0].1)));
        for p in &pts[1..] {
            b.line_to(point(px(self.x0 + p.0), px(self.y0 + p.1)));
        }
        b.close();
        if let Ok(p) = b.build() {
            self.window.paint_path(p, c.opacity(c.a * self.alpha));
        }
    }

    fn dot(&mut self, x: f32, y: f32, r: f32, c: Hsla) {
        self.rect(x - r, y - r, 2. * r, 2. * r, r, c);
    }

    fn line(&mut self, pts: &[(f32, f32)], width: f32, c: Hsla) {
        let mut b = PathBuilder::stroke(px(width));
        b.move_to(point(px(self.x0 + pts[0].0), px(self.y0 + pts[0].1)));
        for p in &pts[1..] {
            b.line_to(point(px(self.x0 + p.0), px(self.y0 + p.1)));
        }
        if let Ok(p) = b.build() {
            self.window.paint_path(p, c.opacity(c.a * self.alpha));
        }
    }
}

fn paint(anim: Anim, b: Bounds<Pixels>, time: f32, t: &Theme, window: &mut Window) {
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let alpha = 1. - seg(time, LOOP - 0.45, LOOP - 0.05);
    // Nothing drawn leaves the stage.
    window.with_content_mask(Some(gpui::ContentMask { bounds: b }), |window| {
        let mut pen = Pen { window, x0: f32::from(b.origin.x), y0: f32::from(b.origin.y), alpha };
        match anim {
            Anim::Folder => folder(&mut pen, w, h, time, t),
            Anim::Blocks => blocks(&mut pen, w, h, time, t),
            Anim::Styled => styled(&mut pen, w, h, time, t),
            Anim::Navigate => navigate(&mut pen, w, h, time, t),
            Anim::Focus => focus(&mut pen, w, h, time, t),
            Anim::Agent => agent(&mut pen, w, h, time, t),
        }
    });
}

/// A block seen from afar: a card with a title strip, scaled about its middle by `pop`.
#[allow(clippy::too_many_arguments)]
fn mini(pen: &mut Pen, cx: f32, cy: f32, w: f32, h: f32, pop: f32, bg: Hsla, strip: Hsla, t: &Theme) -> (f32, f32, f32, f32) {
    let k = lerp(0.6, 1., pop);
    let (w, h) = (w * k, h * k);
    let (x, y) = (cx - w / 2., cy - h / 2.);
    let a = pen.alpha;
    pen.alpha *= pop;
    pen.card(x, y, w, h, 5., bg, t.border);
    pen.rect(x, y, w, (h * 0.16).min(12.), 5., strip);
    pen.alpha = a;
    (x, y + (h * 0.16).min(12.), w, h - (h * 0.16).min(12.))
}

/// Text in a block: bars of the given widths (fractions of `w`), from the top.
fn bars(pen: &mut Pen, x: f32, y: f32, w: f32, widths: &[f32], grow: f32, c: Hsla) {
    for (i, f) in widths.iter().enumerate() {
        let g = ((grow * widths.len() as f32) - i as f32).clamp(0., 1.);
        pen.rect(x, y + i as f32 * 9., w * f * g, 4., 2., c);
    }
}

/// Canvas dots over x, y, w, h, `step` apart, lined up with the point (ox, oy).
#[allow(clippy::too_many_arguments)]
fn dots(pen: &mut Pen, x: f32, y: f32, w: f32, h: f32, step: f32, ox: f32, oy: f32, c: Hsla) {
    let (mut gy, sx) = (y + (oy - y).rem_euclid(step), x + (ox - x).rem_euclid(step));
    while gy < y + h {
        let mut gx = sx;
        while gx < x + w {
            pen.rect(gx - 1., gy - 1., 2., 2., 1., c);
            gx += step;
        }
        gy += step;
    }
}

/// A folder opens out into a canvas, and blocks land on it.
fn folder(pen: &mut Pen, w: f32, h: f32, time: f32, t: &Theme) {
    let (cx, cy) = (w / 2., h / 2.);
    let appear = seg(time, 0.1, 0.7);
    let open = seg(time, 0.9, 1.7);
    let (fw, fh) = (110., 78.);
    let (cw, ch) = (w * 0.8, h * 0.74);
    let (rw, rh) = (lerp(fw, cw, open), lerp(fh, ch, open));
    let (x, y) = (cx - rw / 2., cy - rh / 2. + 6. * (1. - open));
    let a = pen.alpha;
    pen.alpha *= appear;
    // The folder's tab goes as it opens.
    let tab = t.accent.opacity(0.75 * (1. - open));
    pen.rect(x + 8., y - 12., 44., 16., 5., tab);
    let body = Hsla { a: 1., ..t.accent }.opacity(lerp(0.9, 0., open));
    pen.card(x, y, rw, rh, lerp(8., 10., open), t.surface, t.border.opacity(open));
    pen.rect(x, y, rw, rh, lerp(8., 10., open), body);
    pen.alpha = a * seg(time, 1.2, 1.9);
    dots(pen, x + 10., y + 10., rw - 20., rh - 20., 18., 0., 0., t.muted.opacity(0.35));
    pen.alpha = a;
    // Blocks land on it.
    let paper = Paper::of(t);
    let spots = [(-0.26, -0.12, 0.26, 0.4), (0.06, -0.2, 0.3, 0.34), (0.2, 0.18, 0.28, 0.32)];
    for (i, (dx, dy, bw, bh)) in spots.iter().enumerate() {
        let pop = seg(time, 2.0 + i as f32 * 0.25, 2.5 + i as f32 * 0.25);
        if pop <= 0. {
            continue;
        }
        let (bg, strip) = match i {
            0 => (paper.top, paper.header),
            1 => (t.surface, t.surface_hover),
            _ => (crate::terminal::background(t.dark), t.surface_hover),
        };
        let (bx, by, bw2, _) = mini(pen, cx + dx * cw, cy + dy * ch, bw * cw, bh * ch, pop, bg, strip, t);
        let ink = if i == 0 { paper.muted } else { t.muted };
        bars(pen, bx + 8., by + 8., bw2 - 16., &[0.8, 0.6, 0.7], pop, ink.opacity(0.7 * pop));
    }
    // Saved by itself: a mark in the corner.
    let saved = seg(time, 3.2, 3.6);
    if saved > 0. {
        let (sx, sy) = (cx + cw / 2. - 34., cy + ch / 2. - 30.);
        pen.rect(sx, sy, 22., 22., 11., t.accent.opacity(saved));
        pen.line(&[(sx + 6., sy + 11.5), (sx + 9.5, sy + 15.), (sx + 16., sy + 7.5)], 2., t.accent_text.opacity(saved));
    }
}

/// The kinds of block come in one by one, each doing its thing.
fn blocks(pen: &mut Pen, w: f32, h: f32, time: f32, t: &Theme) {
    let paper = Paper::of(t);
    let (cols, rows) = (3., 2.);
    let (gap, bw) = (16., ((w - 48.) - 2. * 16.) / cols);
    let bh = ((h - 40.) - 16.) / rows;
    let x0 = (w - (bw * cols + gap * 2.)) / 2.;
    let y0 = (h - (bh * rows + gap)) / 2.;
    for i in 0..6 {
        let (col, row) = ((i % 3) as f32, (i / 3) as f32);
        let pop = seg(time, 0.15 + i as f32 * 0.18, 0.6 + i as f32 * 0.18);
        if pop <= 0. {
            continue;
        }
        let (cx, cy) = (x0 + col * (bw + gap) + bw / 2., y0 + row * (bh + gap) + bh / 2.);
        let live = (time - 0.6 - i as f32 * 0.18).max(0.);
        let term = crate::terminal::background(t.dark);
        let (bg, strip) = match i {
            0 => (paper.top, paper.header),
            2 => (term, t.surface_hover),
            _ => (t.surface, t.surface_hover),
        };
        if i == 5 {
            // A stack: cards fanning out and back.
            let fan = (live * 2.2).sin().abs() * 6. * pop;
            for k in (0..3).rev() {
                let off = k as f32 * fan;
                mini(pen, cx + off - 6., cy - off + 6., bw * 0.82, bh * 0.82, pop, t.surface, t.surface_hover, t);
            }
            continue;
        }
        let (x, y, iw, ih) = mini(pen, cx, cy, bw, bh, pop, bg, strip, t);
        let grow = (live / 1.2).min(1.);
        match i {
            0 => bars(pen, x + 8., y + 8., iw - 16., &[0.9, 0.7, 0.8, 0.4], grow, paper.muted.opacity(0.8)),
            1 => {
                pen.rect(x + 8., y + 8., (iw - 16.) * 0.5 * grow, 6., 3., t.text.opacity(0.7));
                bars(pen, x + 8., y + 20., iw - 16., &[0.9, 0.8, 0.6], grow, t.muted.opacity(0.6));
            }
            2 => {
                // A shell typing; the caret blinks.
                let typed = ((live * 1.6) as usize).min(3);
                for l in 0..typed {
                    pen.rect(x + 8., y + 8. + l as f32 * 9., 5., 4., 1., t.accent);
                    pen.rect(x + 16., y + 8. + l as f32 * 9., (iw - 28.) * [0.6, 0.8, 0.45][l], 4., 2., t.muted.opacity(0.7));
                }
                if (live * 2.) as i32 % 2 == 0 {
                    pen.rect(x + 8., y + 8. + typed as f32 * 9., 5., 7., 1., t.text.opacity(0.8));
                }
            }
            3 => {
                // A page: its address, a picture, its text.
                pen.card(x + 8., y + 6., iw - 16., 7., 3.5, t.background, t.border);
                pen.rect(x + 8., y + 18., (iw - 16.) * grow, (ih - 40.).max(6.), 3., t.accent.opacity(0.25));
                bars(pen, x + 8., y + ih - 16., iw - 16., &[0.7], grow, t.muted.opacity(0.6));
            }
            _ => {
                // A chart: bars growing.
                let n = 4;
                let slot = (iw - 16.) / n as f32;
                for k in 0..n {
                    let v = [0.45, 0.8, 0.6, 0.95][k] * seg(live, k as f32 * 0.12, 0.6 + k as f32 * 0.12);
                    let bh2 = (ih - 16.) * v;
                    pen.rect(x + 8. + k as f32 * slot + slot * 0.2, y + ih - 8. - bh2, slot * 0.6, bh2, 2., t.accent.opacity(0.85));
                }
            }
        }
    }
}

/// The canvas pans, zooms in and out, and a marquee picks two blocks.
fn navigate(pen: &mut Pen, w: f32, h: f32, time: f32, t: &Theme) {
    let pan = seg(time, 0.3, 1.5) - seg(time, 1.5, 2.2) * 0.5;
    let zoom = 1. + 0.45 * (seg(time, 2.2, 2.9) - seg(time, 2.9, 3.4));
    let (ox, oy) = (-70. * pan, -24. * pan);
    let (cx, cy) = (w / 2., h / 2.);
    let to = |x: f32, y: f32| (cx + (x + ox) * zoom, cy + (y + oy) * zoom);
    let step = 22. * zoom;
    dots(pen, 0., 0., w, h, step, (cx + ox * zoom) - step * 100., (cy + oy * zoom) - step * 100., t.muted.opacity(0.35));
    let paper = Paper::of(t);
    let world = [(-150., -60., 90., 64.), (-30., -80., 100., 70.), (40., 20., 96., 60.), (150., -30., 84., 70.), (-110., 40., 80., 56.)];
    let pick = seg(time, 3.6, 4.4);
    for (i, (x, y, bw, bh)) in world.iter().enumerate() {
        let (sx, sy) = to(*x, *y);
        let (sw, sh) = (bw * zoom, bh * zoom);
        let picked = pick >= 1. && (i == 1 || i == 2);
        let border = if picked { t.accent } else { t.border };
        let bg = if i == 0 { paper.top } else { t.surface };
        pen.card(sx, sy, sw, sh, 5., bg, border);
        pen.rect(sx, sy, sw, (sh * 0.16).min(11.), 5., if i == 0 { paper.header } else { t.surface_hover });
        bars(pen, sx + 7., sy + sh * 0.3, sw - 14., &[0.8, 0.55], 1., t.muted.opacity(0.5));
    }
    // Two fingers: together while panning, apart while zooming.
    let fingers = seg(time, 0.1, 0.4) * (1. - seg(time, 3.3, 3.5));
    if fingers > 0. {
        let spread = 10. + 26. * (zoom - 1.) / 0.45;
        let (fx, fy) = (w * 0.72 + ox * 0.3, h * 0.72 + oy * 0.3);
        for s in [-1., 1.] {
            pen.rect(fx + s * spread - 7., fy - s * spread * 0.4 - 7., 14., 14., 7., t.text.opacity(0.22 * fingers));
        }
    }
    // A marquee from the empty canvas over two blocks.
    if time > 3.6 {
        let (ax, ay) = to(-45., -95.);
        let (bx, by) = to(150., 95.);
        let (mx, my) = (lerp(ax, bx, pick), lerp(ay, by, pick));
        pen.outline(ax, ay, mx - ax, my - ay, 2., t.accent.opacity(0.1), t.accent.opacity(0.9));
    }
}

/// A color of the palette, as the canvas draws it.
fn ink(name: &str, t: &Theme) -> Hsla {
    match crate::theme::paint(name, false, t) {
        crate::theme::Paint::Color(c) => c,
        _ => t.accent,
    }
}

/// A broad arrow pointing right in x, y, w, h, as a 图形 draws it.
fn arrow_pts(x: f32, y: f32, w: f32, h: f32) -> [(f32, f32); 7] {
    let head = (h * 0.6).min(w * 0.5);
    let (neck, cy, half) = (x + w - head, y + h / 2., h * 0.22);
    [(x, cy - half), (neck, cy - half), (neck, y), (x + w, cy), (neck, y + h), (neck, cy + half), (x, cy + half)]
}

/// A spinner of eight spokes, stepping round as 准备中 and 进行中 do.
fn spinner(pen: &mut Pen, cx: f32, cy: f32, r: f32, time: f32, c: Hsla) {
    let head = (time * 9.) as usize % 8;
    for i in 0..8 {
        let behind = (head + 8 - i) % 8;
        let a = i as f32 / 8. * std::f32::consts::TAU;
        let (sn, cs) = a.sin_cos();
        pen.line(&[(cx + cs * r * 0.45, cy + sn * r * 0.45), (cx + cs * r, cy + sn * r)], 1.4, c.opacity(1. - behind as f32 * 0.11));
    }
}

/// Words on the canvas, set as Markdown, and an arrow with words in it; the
/// style panel slides in, a number is scrubbed and the words in the arrow
/// grow, and the eyedropper takes a color the arrow fills with.
fn styled(pen: &mut Pen, w: f32, h: f32, time: f32, t: &Theme) {
    dots(pen, 0., 0., w, h, 22., 0., 0., t.muted.opacity(0.3));
    let text_c = ink("purple", t);
    // 文字: a heading, two items, a checked task.
    let (tx, ty) = (w * 0.08, h * 0.16);
    let words = seg(time, 0.15, 1.1);
    let tw = w * 0.34;
    pen.rect(tx, ty, tw * 0.55 * seg(time, 0.15, 0.5), 10., 3., text_c);
    for (i, f) in [0.8, 0.62].iter().enumerate() {
        let g = ((words * 3.) - 1. - i as f32).clamp(0., 1.);
        let y = ty + 24. + i as f32 * 14.;
        pen.dot(tx + 3., y + 2.5, 2. * g, text_c.opacity(0.7));
        pen.rect(tx + 12., y, (tw - 12.) * f * g, 5., 2.5, text_c.opacity(0.75));
    }
    let task = seg(time, 0.9, 1.2);
    if task > 0. {
        let y = ty + 52.;
        pen.rect(tx, y - 2., 9., 9., 2., t.accent.opacity(task));
        pen.line(&[(tx + 2., y + 2.5), (tx + 4., y + 4.5), (tx + 7.5, y + 0.5)], 1.4, t.accent_text.opacity(task));
        pen.rect(tx + 14., y, (tw - 14.) * 0.5 * task, 5., 2.5, text_c.opacity(0.45));
    }

    // 图形: an arrow drawn out from its tail, its fill turning to the color picked.
    let (ax, ay, aw, ah) = (w * 0.08, h * 0.52, w * 0.42, h * 0.32);
    let drawn = seg(time, 1.0, 1.6);
    let picked = seg(time, 3.5, 3.9);
    let soft = |c: Hsla| Hsla { a: if t.dark { 0.35 } else { 0.22 }, ..c };
    let (blue, orange) = (ink("blue", t), ink("orange", t));
    let stroke = Hsla { h: lerp(blue.h, orange.h, picked), ..blue };
    if drawn > 0. {
        let pts = arrow_pts(ax, ay, aw * lerp(0.35, 1., drawn), ah);
        let a = pen.alpha;
        pen.alpha *= drawn;
        // `opacity` scales the soft fill's own alpha.
        pen.poly(&pts, soft(blue).opacity(1. - picked));
        pen.poly(&pts, soft(orange).opacity(picked));
        let mut ring = pts.to_vec();
        ring.push(pts[0]);
        pen.line(&ring, 1.6, stroke);
        pen.alpha = a;
    }
    // Its words, growing as the size is scrubbed.
    let scrub = seg(time, 2.3, 3.0);
    let big = lerp(4., 8., scrub);
    if drawn >= 1. {
        let bw = aw * lerp(0.22, 0.32, scrub);
        pen.rect(ax + aw * 0.12, ay + ah / 2. - big / 2., bw, big, big / 2., t.text.opacity(0.75));
    }
    // Picked: the outline, while the panel is out.
    let panel_in = seg(time, 1.7, 2.2);
    if panel_in > 0. {
        pen.outline(ax - 4., ay - 4., aw + 8., ah + 8., 3., gpui::transparent_black(), t.accent.opacity(panel_in));
    }

    // The panel slides in from the right.
    let (pw, ph) = (w * 0.3, h * 0.78);
    let px0 = lerp(w + 10., w - pw - w * 0.05, panel_in);
    let py0 = (h - ph) / 2.;
    if panel_in > 0. {
        pen.card(px0, py0, pw, ph, 8., t.surface, t.border);
        // Shapes.
        for i in 0..4 {
            let bx = px0 + 10. + i as f32 * ((pw - 20.) / 4.);
            let on = i == 3;
            pen.rect(bx, py0 + 10., (pw - 20.) / 4. - 4., 14., 3., if on { t.accent.opacity(0.18) } else { t.background });
        }
        // Swatches, then the eyedropper.
        let sw_y = py0 + 36.;
        let names = ["red", "orange", "yellow", "green", "blue", "purple"];
        let step = (pw - 20.) / 7.;
        for (i, n) in names.iter().enumerate() {
            let cx = px0 + 10. + step * (i as f32 + 0.5);
            if i == 1 && picked > 0. {
                pen.dot(cx, sw_y, 6.5, t.accent.opacity(picked));
                pen.dot(cx, sw_y, 5., t.surface);
            }
            pen.dot(cx, sw_y, 4., ink(n, t));
        }
        let ex = px0 + 10. + step * 6.5;
        pen.dot(ex, sw_y, 6., t.background);
        pen.line(&[(ex - 3., sw_y + 3.), (ex + 3., sw_y - 3.)], 1.5, t.text.opacity(0.8));
        // The size, scrubbed: a field whose bar fills as the pointer drags right.
        let (fx, fy, fw) = (px0 + 10., py0 + 56., pw - 20.);
        pen.outline(fx, fy, fw, 16., 4., t.background, if scrub > 0. && scrub < 1. { t.accent } else { t.border });
        pen.rect(fx + 1., fy + 1., (fw - 2.) * lerp(0.25, 0.6, scrub), 14., 3., t.accent.opacity(0.18));
        pen.rect(fx + fw - 22., fy + 6., 14., 4., 2., t.text.opacity(0.6));
        let hand = seg(time, 2.2, 2.3) * (1. - seg(time, 3.0, 3.1));
        if hand > 0. {
            pen.dot(fx + fw * lerp(0.35, 0.7, scrub), fy + 8., 5., t.text.opacity(0.28 * hand));
        }
        // Rows below: font and alignment, standing in.
        for k in 0..3 {
            pen.rect(fx, fy + 26. + k as f32 * 14., fw * [0.9, 0.6, 0.75][k], 6., 3., t.muted.opacity(0.35));
        }
        // The eyedropper's loupe goes out to a color on the screen and takes it.
        let go = seg(time, 3.05, 3.45);
        if go > 0. && picked < 1. {
            let (lx, ly) = (lerp(ex, w * 0.36, go), lerp(sw_y, h * 0.12, go));
            pen.dot(lx, ly, 9., t.surface);
            pen.dot(lx, ly, 7.5, orange);
            pen.outline(lx - 9., ly - 9., 18., 18., 9., gpui::transparent_black(), t.text.opacity(0.5));
        }
    }
    // The color it took, on the screen beyond: a spot of it.
    pen.dot(w * 0.36, h * 0.12, 4. * seg(time, 2.9, 3.1), orange.opacity(0.9));
}

/// Foci marked on the dimmed canvas and joined into a line; then it plays:
/// the view goes to the first, glides on to the second, and says where it is.
fn focus(pen: &mut Pen, w: f32, h: f32, time: f32, t: &Theme) {
    let paper = Paper::of(t);
    let world = [(-150., -55., 96., 66.), (-20., -70., 104., 72.), (70., 25., 96., 60.), (-120., 35., 84., 56.)];
    let center = |i: usize| (world[i].0 + world[i].2 / 2., world[i].1 + world[i].3 / 2.);
    let stops = [3usize, 1, 2];
    // The camera: still, then to the first stop, then gliding to the second (out a little on the way).
    let f1 = (center(stops[0]).0, center(stops[0]).1, 1.9);
    let f2 = (center(stops[1]).0, center(stops[1]).1, 1.7);
    let a = seg(time, 2.3, 2.9);
    let b = seg(time, 3.0, 3.7);
    let (mut fx, mut fy, mut z) = (lerp(0., f1.0, a), lerp(0., f1.1, a), lerp(1., f1.2, a));
    if b > 0. {
        fx = lerp(f1.0, f2.0, b);
        fy = lerp(f1.1, f2.1, b);
        z = lerp(f1.2, f2.2, b) * (1. - 0.3 * (std::f32::consts::PI * b).sin());
    }
    let (cx, cy) = (w / 2., h / 2.);
    let to = |x: f32, y: f32| (cx + (x - fx) * z, cy + (y - fy) * z);
    let step = 22. * z;
    let (ox, oy) = to(0., 0.);
    dots(pen, 0., 0., w, h, step, ox, oy, t.muted.opacity(0.35));
    for (i, (x, y, bw, bh)) in world.iter().enumerate() {
        let (sx, sy) = to(*x, *y);
        let (sw, sh) = (bw * z, bh * z);
        let bg = if i == 0 { paper.top } else { t.surface };
        pen.card(sx, sy, sw, sh, 5., bg, t.border);
        pen.rect(sx, sy, sw, (sh * 0.16).min(11. * z), 5., if i == 0 { paper.header } else { t.surface_hover });
        bars(pen, sx + 7. * z, sy + sh * 0.32, sw - 14. * z, &[0.8, 0.55], 1., t.muted.opacity(0.5));
    }
    // The focus view: dimmed, its dots coming one by one, joined in order.
    let view = seg(time, 0.2, 0.6) * (1. - seg(time, 2.0, 2.3));
    if view > 0. {
        pen.rect(0., 0., w, h, 0., t.background.opacity(0.7 * view));
        let a0 = pen.alpha;
        pen.alpha *= view;
        let pts: Vec<(f32, f32)> = stops.iter().map(|&i| to(center(i).0, center(i).1)).collect();
        for k in 0..pts.len() - 1 {
            let g = seg(time, 1.0 + k as f32 * 0.4, 1.35 + k as f32 * 0.4);
            if g <= 0. {
                continue;
            }
            let (p, q) = (pts[k], pts[k + 1]);
            pen.line(&[p, (lerp(p.0, q.0, g), lerp(p.1, q.1, g))], 1.8, t.accent.opacity(0.75));
        }
        for (k, p) in pts.iter().enumerate() {
            let pop = seg(time, 0.55 + k as f32 * 0.25, 0.85 + k as f32 * 0.25);
            if pop <= 0. {
                continue;
            }
            pen.dot(p.0, p.1, 5.5 * pop, t.accent);
            pen.card(p.0 + 10., p.1 - 7., 34. * pop, 14., 4., t.surface, t.accent.opacity(0.5));
            pen.rect(p.0 + 15., p.1 - 2., 6. * pop, 4., 2., t.accent);
            pen.rect(p.0 + 24., p.1 - 2., 14. * pop, 4., 2., t.muted.opacity(0.6));
        }
        pen.alpha = a0;
    }
    // Playing: where it is, at the bottom.
    let hud = seg(time, 2.4, 2.7);
    if hud > 0. {
        let (hw, hh) = (118., 20.);
        let (hx, hy) = ((w - hw) / 2., h - hh - 12.);
        let a0 = pen.alpha;
        pen.alpha *= hud;
        pen.card(hx, hy, hw, hh, hh / 2., t.surface, t.border);
        pen.dot(hx + 12., hy + hh / 2., 3., t.accent);
        pen.rect(hx + 22., hy + 8., 42., 4., 2., t.text.opacity(0.7));
        // 1 / 3, then 2 / 3: three marks, the ones passed lit.
        let at = if b > 0.5 { 2 } else { 1 };
        for k in 0..3 {
            pen.rect(hx + 74. + k as f32 * 12., hy + 8., 8., 4., 2., if k < at { t.accent } else { t.muted.opacity(0.35) });
        }
        pen.alpha = a0;
    }
}

/// An agent in a terminal lines the scattered blocks up, then adds one;
/// along the bottom, kept clear for it, the capsule tells how it goes.
fn agent(pen: &mut Pen, w: f32, full_h: f32, time: f32, t: &Theme) {
    let h = full_h - 30.;
    let term = crate::terminal::background(t.dark);
    let (tx, ty, tw, th) = (w * 0.07, h * 0.2, w * 0.34, h * 0.6);
    pen.card(tx, ty, tw, th, 6., term, t.border);
    pen.rect(tx, ty, tw, 12., 6., t.surface_hover);
    let typed = ((time * 2.4) as usize).min(5);
    for l in 0..typed {
        let me = l % 2 == 0;
        pen.rect(tx + 10., ty + 22. + l as f32 * 11., 5., 4., 1., if me { t.accent } else { t.muted.opacity(0.6) });
        pen.rect(tx + 18., ty + 22. + l as f32 * 11., (tw - 30.) * [0.5, 0.8, 0.4, 0.7, 0.6][l], 4., 2., t.muted.opacity(0.7));
    }
    // What it says goes out to the canvas.
    let (bx0, by) = (tx + tw + 10., ty + th / 2.);
    let bx1 = w * 0.52;
    let beam = seg(time, 1.3, 1.8);
    let fade = 1. - seg(time, 3.0, 3.4);
    let mut x = bx0;
    while x < lerp(bx0, bx1, beam) {
        pen.rect(x, by - 1.5, 6., 3., 1.5, t.accent.opacity(0.8 * fade));
        x += 11.;
    }
    // Scattered blocks glide into a grid.
    let arrange = seg(time, 1.9, 2.9);
    let paper = Paper::of(t);
    let (gx, gy, cw, ch, gap) = (w * 0.55, h * 0.18, w * 0.15, h * 0.26, 10.);
    let scattered = [(0.62, 0.22, -6.), (0.84, 0.3, 4.), (0.58, 0.6, 2.), (0.8, 0.66, -4.)];
    for (i, (sx, sy, _)) in scattered.iter().enumerate() {
        let (col, row) = ((i % 2) as f32, (i / 2) as f32);
        let (ex, ey) = (gx + col * (cw + gap), gy + row * (ch + gap));
        let x = lerp(w * sx - cw / 2., ex, arrange);
        let y = lerp(h * sy - ch / 2., ey, arrange);
        let bg = if i == 0 { paper.top } else { t.surface };
        pen.card(x, y, cw, ch, 5., bg, t.border);
        pen.rect(x, y, cw, 9., 5., if i == 0 { paper.header } else { t.surface_hover });
        bars(pen, x + 6., y + 15., cw - 12., &[0.8, 0.5], 1., t.muted.opacity(0.5));
    }
    // And one more, made by it.
    let made = seg(time, 3.1, 3.6);
    if made > 0. {
        let (ex, ey) = (gx, gy + 2. * (ch + gap));
        let (x, y, iw, _) = mini(pen, ex + (cw * 2. + gap) / 2., ey + ch * 0.4, cw * 2. + gap, ch * 0.8, made, t.surface, t.surface_hover, t);
        let a = pen.alpha;
        pen.alpha *= made;
        pen.card(x - 1., y - 10.5, iw + 2., 0.5, 0., t.accent, t.accent);
        pen.alpha = a;
        bars(pen, x + 8., y + 6., iw - 16., &[0.6, 0.4], made, t.accent.opacity(0.7));
    }
    // What it is doing, told at the bottom: getting ready, under way as the blocks move, done.
    let shown = seg(time, 0.3, 0.6);
    if shown > 0. {
        let (cw2, chh) = (150., 20.);
        let (cx2, cy2) = ((w - cw2) / 2., full_h - chh - 8. + 6. * (1. - shown));
        let a = pen.alpha;
        pen.alpha *= shown;
        pen.card(cx2, cy2, cw2, chh, chh / 2., t.surface, t.border);
        let done = seg(time, 3.6, 3.8);
        let (dx, dy) = (cx2 + 12., cy2 + chh / 2.);
        if done > 0. {
            pen.dot(dx, dy, 6., ink("green", t).opacity(done));
            pen.line(&[(dx - 3., dy), (dx - 1., dy + 2.), (dx + 3., dy - 2.5)], 1.4, gpui::white().opacity(done));
        } else {
            let running = time > 1.3;
            spinner(pen, dx, dy, 5.5, time, if running { t.accent } else { t.muted });
        }
        pen.rect(cx2 + 24., cy2 + 8., 46., 4., 2., t.text.opacity(0.7));
        let state = if done > 0. { ink("green", t) } else if time > 1.3 { t.accent } else { t.muted };
        pen.rect(cx2 + cw2 - 32., cy2 + 8., 22., 4., 2., state.opacity(0.8));
        // How far along: a bar on the bottom edge as the blocks are put in place.
        let along = seg(time, 1.3, 3.5);
        if along > 0. && done < 1. {
            pen.rect(cx2 + 10., cy2 + chh - 2.5, (cw2 - 20.) * along, 2., 1., t.accent.opacity(0.8 * (1. - done)));
        }
        pen.alpha = a;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_named_is_in_the_commands_table() {
        for p in PAGES {
            for (k, _) in p.keys {
                if let Key::Menu(label) = k {
                    assert!(menu_keys(label).is_some(), "「{label}」不在命令表里或没有快捷键");
                }
            }
        }
    }

    #[test]
    fn the_panel_fits_without_scrolling() {
        // The smallest window there is, and a big one.
        for (vw, vh) in [(680., 420.), (1280., 800.)] {
            let (x, y, w, h) = panel_rect(vw, vh);
            assert!(x >= 16. && y >= 0. && x + w <= vw - 16. && y + h <= vh, "{vw}x{vh}");
            assert!(h - TEXT_H >= ANIM_H.0);
        }
    }
}
