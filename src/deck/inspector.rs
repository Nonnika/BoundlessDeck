//! The panel on the right that styles 文字 and 图形: it shows while one or
//! more of them are selected (or the one worked in), and what is set in it
//! goes to all of them.
//!
//! It slides in from the window's right edge and back out, smeared along the
//! way: GPUI has no blur, so the smear is copies of the panel trailing it,
//! fainter the farther back, spread over how far it moves in a camera's
//! shutter time. The copies sit under a cover that takes their clicks.
//!
//! Fonts are picked in a list of our own beside the panel, from the index
//! `font` builds at launch. Custom colors come from a palette in the same
//! place: preset shades, the colors this deck already uses, a hex field, the
//! screen's eyedropper and the system's Colors panel (`nativepanel`), whose
//! picks come back through `panel_tx`. The size and the corners are scrubbed:
//! dragged left and right (⇧ for bigger steps), or clicked to type a number.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, AppContext, Context, CursorStyle, Entity, Focusable, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, SharedString, StatefulInteractiveElement, Styled, Subscription,
    UniformListScrollHandle, Window, canvas, div, point, prelude::FluentBuilder, px, relative, svg, uniform_list,
};

use super::DeckView;
use super::sharpen;
use super::styled::{self, Shape};
use crate::editor::{Editor, EditorEvent};
use crate::font;
use crate::icons;
use crate::model::{Block, BlockKind};
use crate::nativepanel::{self, Pick};
use crate::theme::{INKS, Paint, Theme, hex, paint};

/// The panel's width, and where it sits under the toolbar.
pub(super) const PANEL_W: f32 = 248.;
pub(super) const PANEL_TOP: f32 = 64.;
const PANEL_RIGHT: f32 = 12.;
/// How far it slides: from just past the window's right edge, shadow and all.
const TRAVEL: f32 = PANEL_W + PANEL_RIGHT + 28.;
/// How long sliding in and out takes.
const ENTER: Duration = Duration::from_millis(380);
const LEAVE: Duration = Duration::from_millis(240);
/// The smear: as far as the panel moves in this long, in this many copies, no longer than this.
const SHUTTER: f32 = 1. / 24.;
const GHOSTS: usize = 6;
const MAX_TRAIL: f32 = 150.;

/// The font list beside the panel: its size, and how it comes in.
const PICKER_W: f32 = 268.;
const PICKER_GAP: f32 = 8.;
const PICKER_IN: Duration = Duration::from_millis(220);
const PICKER_SLIDE: f32 = 28.;
const ROW_H: f32 = 34.;

const WEIGHTS: &[(u16, &str)] = &[(400, "常规"), (500, "中等"), (700, "粗体")];
const MAX_RADIUS: f32 = 999.;

/// The palette beside the panel: a swatch's side, and the heights its parts are laid out at.
const CHIP: f32 = 22.;
const CHIP_GAP: f32 = 2.;
const PAL_PAD: f32 = 14.;
const PAL_GAP: f32 = 12.;
const PAL_TITLE_H: f32 = 16.;
const PAL_RECENT_H: f32 = 14. + 6. + CHIP;
const PAL_FIELD_H: f32 = 30.;
/// The palette's hues, in degrees, a column each; and the shades of each, a row each, light to dark.
const HUES: [f32; 10] = [0., 24., 45., 90., 145., 175., 200., 222., 265., 320.];
const SHADES: [(f32, f32); 5] = [(0.9, 0.88), (0.8, 0.74), (0.75, 0.6), (0.7, 0.46), (0.65, 0.32)];
/// How many of the deck's own colors the palette shows.
const RECENT: usize = 10;

/// How far a scrub moves a number for each point the pointer goes, and with ⇧.
const SCRUB_SIZE: f32 = 0.25;
const SCRUB_RADIUS: f32 = 0.5;
const SCRUB_SHIFT: f32 = 5.;
/// How far the pointer goes before a press on a number is a scrub, not a click.
const SCRUB_SLOP: f32 = 3.;

/// The panel's state from frame to frame.
#[derive(Default)]
pub(super) struct Inspector {
    /// Whether it is coming in (or in), else going out.
    shown: bool,
    /// How far in it is, 0 out of sight to 1 in place, and when that was so;
    /// `at` is none while it rests.
    p: f32,
    at: Option<Instant>,
    /// What it styled last: what it shows while sliding out.
    last: Vec<String>,
    /// A number being typed.
    field: Option<Field>,
    /// A number being scrubbed.
    scrub: Option<Scrub>,
    /// The font list or the palette, beside the panel; one at a time.
    picker: Option<Picker>,
    palette: Option<Palette>,
    /// Where the panel and the font list cover the window this frame, for
    /// web pages under them to give way.
    pub(super) rects: Vec<(f32, f32, f32, f32)>,
}

/// Which number a field or stepper sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Num {
    Size,
    Radius,
    /// Top-left, top-right, bottom-right, bottom-left.
    Corner(usize),
}

pub(super) struct Field {
    which: Num,
    editor: Entity<Editor>,
    _sub: Subscription,
}

/// A press on a number, dragged left or right to change it.
pub(super) struct Scrub {
    which: Num,
    /// Where the press was, and the number then.
    from_x: f32,
    start: f32,
    /// Where the pointer was last, and the number so far, before rounding.
    last_x: f32,
    value: f32,
    moved: bool,
}

/// The palette, for one color of the blocks.
pub(super) struct Palette {
    slot: Slot,
    at: Instant,
    /// The hex field.
    hex: Entity<Editor>,
    _sub: Subscription,
}

pub(super) struct Picker {
    at: Instant,
    search: Entity<Editor>,
    rows: Vec<Row>,
    list: UniformListScrollHandle,
    /// Polling for the index, which is still being built.
    waiting: bool,
    _sub: Subscription,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Head(&'static str),
    /// A family: its name (empty for the system font) and the name shown.
    Font { name: String, shown: String },
}

/// Which color a custom pick goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    Fill,
    Stroke,
    Color,
}

/// Fast at first, settling in place. Run backwards, the slide out starts slow and speeds off.
fn ease(p: f32) -> f32 {
    1. - (1. - p).powi(3)
}

fn ease_rate(p: f32) -> f32 {
    3. * (1. - p).powi(2)
}

impl DeckView {
    /// The 文字 and 图形 the panel styles: those selected, else the one worked in.
    pub(super) fn style_targets(&self) -> Vec<String> {
        let sel: Vec<String> =
            self.m.blocks.iter().filter(|b| b.kind.bare() && self.m.sel.contains(&b.id)).map(|b| b.id.clone()).collect();
        if !sel.is_empty() {
            return sel;
        }
        let worked = self.editing.as_ref().map(|e| e.id.clone()).or_else(|| self.active.clone());
        worked.into_iter().filter(|id| self.m.block(id).is_some_and(|b| b.kind.bare())).collect()
    }

    /// Changes each block the panel styles, then saves.
    fn restyle(&mut self, f: impl Fn(&mut Block), cx: &mut Context<Self>) {
        for id in self.style_targets() {
            if let Some(b) = self.m.block_mut(&id) {
                f(b);
            }
        }
        self.request_save(cx);
        cx.notify();
    }

    /// A pick from the Colors panel: to the blocks the panel styles.
    pub(super) fn on_pick(&mut self, pick: Pick, cx: &mut Context<Self>) {
        let Pick::Color(c) = pick;
        let slot = self.color_slot;
        self.set_color(slot, c, cx);
    }

    /// Sets one color of the blocks the panel styles, and shows it in the palette's field.
    fn set_color(&mut self, slot: Slot, spec: String, cx: &mut Context<Self>) {
        if let Some(pl) = self.insp.palette.as_ref().filter(|pl| pl.slot == slot) {
            let shown = if spec.starts_with('#') { spec.clone() } else { String::new() };
            pl.hex.update(cx, |e, cx| e.set_text(shown, cx));
        }
        self.restyle(move |b| *slot_of(b, slot) = spec.clone(), cx);
    }

    /// The `#rrggbb` colors the deck's 文字 and 图形 use, first found first.
    fn recent_colors(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for b in self.m.blocks.iter().filter(|b| b.kind.bare()) {
            for spec in [&b.style.fill, &b.style.stroke, &b.style.color] {
                if let Some(h) = parse_hex(spec).filter(|_| spec.starts_with('#'))
                    && !out.contains(&h)
                {
                    out.push(h);
                }
            }
            if out.len() >= RECENT {
                break;
            }
        }
        out.truncate(RECENT);
        out
    }

    /// Before the blocks are drawn: whether the panel is coming or going, how
    /// far it is, and what it covers.
    pub(super) fn inspector_tick(&mut self) {
        let targets = self.style_targets();
        let want = !targets.is_empty();
        if want {
            self.insp.last = targets;
        } else {
            self.insp.picker = None;
            self.insp.palette = None;
            self.insp.field = None;
            self.insp.scrub = None;
        }
        let recent = self.insp.palette.is_some() && !self.recent_colors().is_empty();
        let goal = if want { 1. } else { 0. };
        let st = &mut self.insp;
        st.shown = want;
        let now = Instant::now();
        if sharpen::reduce_motion() || st.p == goal {
            st.p = goal;
            st.at = None;
        } else {
            // The first frame of a move only starts its clock: the time since the last frame may be long.
            let dt = st.at.map_or(0., |a| (now - a).as_secs_f32());
            st.at = Some(now);
            let dur = if want { ENTER } else { LEAVE };
            st.p = if want { (st.p + dt / dur.as_secs_f32()).min(1.) } else { (st.p - dt / dur.as_secs_f32()).max(0.) };
            self.animating = true;
        }
        if st.p <= 0. {
            st.last.clear();
        }

        st.rects.clear();
        if st.p > 0. {
            let (vw, vh) = self.view;
            let dx = (1. - ease(st.p)) * TRAVEL;
            let right = PANEL_RIGHT - dx;
            st.rects.push((vw - right - PANEL_W, PANEL_TOP, PANEL_W, vh - PANEL_TOP - 48.));
            let side = match (&st.picker, &st.palette) {
                (Some(pk), _) => Some((pk.at, picker_h(vh))),
                (None, Some(pl)) => Some((pl.at, palette_h(recent))),
                _ => None,
            };
            if let Some((at, h)) = side {
                let e = ease((at.elapsed().as_secs_f32() / PICKER_IN.as_secs_f32()).min(1.));
                let pr = right + PANEL_W + PICKER_GAP + (1. - e) * PICKER_SLIDE;
                st.rects.push((vw - pr - PICKER_W, PANEL_TOP, PICKER_W, h));
                if e < 1. {
                    self.animating = true;
                }
            }
        }
    }

    /// The panel and the font list, while there is something to style or the panel is still sliding out.
    pub(super) fn inspector(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.insp.p;
        if p <= 0. {
            return vec![];
        }
        let targets: Vec<String> =
            self.insp.last.iter().filter(|id| self.m.block(id).is_some_and(|b| b.kind.bare())).cloned().collect();
        if targets.is_empty() {
            return vec![];
        }
        let dx = (1. - ease(p)) * TRAVEL;
        let right = PANEL_RIGHT - dx;
        // How far it moves in a shutter's time: the length of its smear, behind it.
        let trail = if self.insp.at.is_some() {
            let dur = if self.insp.shown { ENTER } else { LEAVE };
            (TRAVEL * ease_rate(p) / dur.as_secs_f32() * SHUTTER).min(MAX_TRAIL)
        } else {
            0.
        };
        let behind = if self.insp.shown { 1. } else { -1. };
        let blur = trail / MAX_TRAIL;

        let mut layer = div().absolute().top(px(PANEL_TOP)).right(px(right)).w(px(PANEL_W));
        if trail > 1. {
            for k in (1..=GHOSTS).rev() {
                let f = k as f32 / GHOSTS as f32;
                let ghost = self.panel(&targets, false, t, cx);
                layer = layer.child(
                    div()
                        .id(("inspector-ghost", k))
                        .absolute()
                        .top_0()
                        .left(px(behind * trail * f))
                        .w(px(PANEL_W))
                        .opacity((0.42 * (1. - f * 0.8) * blur.max(0.35)).min(1.))
                        .child(ghost),
                );
            }
            // The copies are only seen: this takes the presses that would reach them.
            layer = layer.child(div().absolute().top_0().bottom_0().left(px(-trail)).right(px(-trail)).occlude());
        }
        let main = self.panel(&targets, true, t, cx);
        layer = layer.child(div().relative().opacity(1. - 0.5 * blur).child(main));

        let mut out = vec![layer.into_any_element()];
        if let Some(list) = self.picker_view(&targets, right, t, cx) {
            out.push(list);
        }
        if let Some(pal) = self.palette_view(&targets, right, t, cx) {
            out.push(pal);
        }
        out
    }

    /// The panel's content for the targets. `live` is the one used; the
    /// smear's copies show numbers being typed as they were.
    fn panel(&mut self, targets: &[String], live: bool, t: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let first = targets.first().and_then(|id| self.m.block(id)).cloned().unwrap_or_else(|| Block::new(BlockKind::Text));
        let look = styled::look(&first, t);
        let any_shape = targets.iter().any(|id| self.m.block(id).is_some_and(|b| b.kind == BlockKind::Shape));
        let title = if targets.len() > 1 { format!("{} 个对象", targets.len()) } else { first.kind.label().to_string() };

        let mut panel = div()
            .id("inspector")
            .w(px(PANEL_W))
            .max_h(px((self.view.1 - PANEL_TOP - 60.).max(120.)))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(14.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(px(12.))
            .shadow(vec![gpui::BoxShadow {
                color: t.shade,
                offset: point(px(0.), px(6.)),
                blur_radius: px(20.),
                spread_radius: px(0.),
            }])
            .occlude()
            .font_family(".SystemUIFont")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child(title));

        if any_shape {
            let current = Shape::of(&first.shape);
            let buttons = [
                ("rect", Shape::Rect),
                ("ellipse", Shape::Ellipse),
                ("diamond", Shape::Diamond),
                ("triangle", Shape::Triangle),
                ("arrow", Shape::Arrow),
            ]
                .into_iter()
                .map(|(name, shape)| {
                    let on = current == shape && first.kind == BlockKind::Shape;
                    let color = if on { t.accent } else { t.muted };
                    toggle(SharedString::from(format!("shape-{name}")), on, t)
                        .flex_1()
                        .h(px(30.))
                        .child(canvas(|_, _, _| (), move |b, _, window, _| styled::paint_icon(window, b, shape, color)).size(px(20.)))
                        .on_click(cx.listener(move |d, _, _, cx| {
                            d.restyle(
                                |b| {
                                    if b.kind == BlockKind::Shape {
                                        b.shape = name.to_string();
                                    }
                                },
                                cx,
                            )
                        }))
                });
            panel = panel.child(section("形状", t).child(div().flex().flex_row().gap(px(6.)).children(buttons)));
        }

        for (slot, label) in [(Slot::Fill, "填充"), (Slot::Stroke, "边框"), (Slot::Color, "文字颜色")] {
            let spec = slot_value(&first, slot);
            panel = panel.child(section(label, t).child(self.swatches(slot, &spec, t, cx)));
        }

        // The font: its family from the list beside the panel; size, weight and italic here.
        let shown = font_shown(&first.style.font);
        let open = self.insp.picker.is_some();
        let font_button = div()
            .id("font-family")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .h(px(32.))
            .rounded(px(7.))
            .border_1()
            .border_color(if open { t.accent } else { t.border })
            .bg(t.background)
            .cursor_pointer()
            .hover(|d| d.bg(t.surface_hover))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(13.))
                    .text_color(t.text)
                    .when(!first.style.font.is_empty(), |d| d.font_family(first.style.font.clone()))
                    .child(shown),
            )
            .child(svg().path(icons::path("chevron_left")).size(px(14.)).text_color(if open { t.accent } else { t.muted }))
            .on_click(cx.listener(|d, _, window, cx| d.toggle_picker(window, cx)));
        let weight = first.style.weight;
        let weights = div().flex().flex_row().gap(px(4.)).children(WEIGHTS.iter().map(|&(w, label)| {
            let on = (if weight == 0 { 400 } else { weight }) == w;
            toggle(SharedString::from(format!("weight-{w}")), on, t)
                .flex_1()
                .h(px(28.))
                .text_size(px(12.))
                .child(label)
                .on_click(cx.listener(move |d, _, _, cx| d.restyle(move |b| b.style.weight = if w == 400 { 0 } else { w }, cx)))
        }));
        let italic = first.style.italic;
        let italic_button = toggle("italic".into(), italic, t)
            .w(px(36.))
            .h(px(28.))
            .text_size(px(13.))
            .italic()
            .child("I")
            .on_click(cx.listener(move |d, _, _, cx| d.restyle(move |b| b.style.italic = !italic, cx)));
        panel = panel.child(
            section("字体", t)
                .child(font_button)
                .child(self.scrubber(Num::Size, &first, "pt", live, t, cx))
                .child(div().flex().flex_row().gap(px(6.)).child(div().flex_1().child(weights)).child(italic_button)),
        );

        // Where the words sit.
        let across = [("left", "align_left"), ("center", "align_center"), ("right", "align_right")].into_iter().map(|(v, icon)| {
            let on = look.align == along(v);
            toggle(SharedString::from(format!("align-{v}")), on, t)
                .flex_1()
                .h(px(28.))
                .child(svg().path(icons::path(icon)).size(px(14.)).text_color(if on { t.accent } else { t.muted }))
                .on_click(cx.listener(move |d, _, _, cx| d.restyle(move |b| b.style.align = v.to_string(), cx)))
        });
        let down = [("top", "上"), ("middle", "中"), ("bottom", "下")].into_iter().map(|(v, label)| {
            let on = look.valign == along(v);
            toggle(SharedString::from(format!("valign-{v}")), on, t)
                .flex_1()
                .h(px(28.))
                .text_size(px(12.))
                .child(label)
                .on_click(cx.listener(move |d, _, _, cx| d.restyle(move |b| b.style.valign = v.to_string(), cx)))
        });
        panel = panel.child(
            section("对齐", t)
                .child(div().flex().flex_row().gap(px(4.)).children(across))
                .child(div().flex().flex_row().gap(px(4.)).children(down)),
        );

        // Corners: for rectangles and text, all four as one or each its own.
        let cornered = targets.iter().any(|id| {
            self.m.block(id).is_some_and(|b| b.kind == BlockKind::Text || Shape::of(&b.shape) == Shape::Rect)
        });
        if cornered {
            let each = first.style.corners.is_some();
            let split = toggle("corners-each".into(), each, t)
                .h(px(22.))
                .px(px(6.))
                .gap(px(4.))
                .text_size(px(11.))
                .child(svg().path(icons::path("corners")).size(px(12.)).text_color(if each { t.accent } else { t.muted }))
                .child("四角分别")
                .on_click(cx.listener(move |d, _, _, cx| {
                    d.insp.field = None;
                    d.restyle(
                        move |b| {
                            if each {
                                b.style.radius = Some(styled::radii(b)[0]);
                                b.style.corners = None;
                            } else {
                                b.style.corners = Some(styled::radii(b));
                            }
                        },
                        cx,
                    )
                }));
            let head = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(div().text_size(px(11.)).text_color(t.muted).child("圆角"))
                .child(split);
            let body = if each {
                let mut grid = div().flex().flex_col().gap(px(6.));
                for pair in [[0, 1], [3, 2]] {
                    let mut row = div().flex().flex_row().gap(px(6.));
                    for i in pair {
                        row = row.child(div().flex_1().min_w_0().child(self.scrubber(Num::Corner(i), &first, "", live, t, cx)));
                    }
                    grid = grid.child(row);
                }
                grid
            } else {
                div().child(self.scrubber(Num::Radius, &first, "px", live, t, cx))
            };
            panel = panel.child(div().flex().flex_col().gap(px(6.)).child(head).child(body));
        }
        panel
    }

    /// A number to scrub: its icon, a bar as full as the number is far along
    /// its usual range, and the number. Dragged left or right it changes; a
    /// click types one in.
    fn scrubber(&mut self, which: Num, first: &Block, unit: &str, live: bool, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let v = num_of(first, which);
        let id = SharedString::from(format!("{which:?}-value"));
        let typing = live.then(|| self.insp.field.as_ref().filter(|f| f.which == which).map(|f| f.editor.clone())).flatten();
        let frame = || div().relative().flex_1().min_w_0().h(px(28.)).rounded(px(6.)).border_1().bg(t.background).overflow_hidden();
        if let Some(editor) = typing {
            return div().flex().child(frame().id(id).px(px(4.)).border_color(t.accent).child(editor));
        }
        let scrubbing = live && self.insp.scrub.as_ref().is_some_and(|sc| sc.which == which && sc.moved);
        let icon: AnyElement = match which {
            Num::Size => svg().path(icons::path("type")).size(px(12.)).flex_none().text_color(t.muted).into_any_element(),
            Num::Radius => svg().path(icons::path("corners")).size(px(12.)).flex_none().text_color(t.muted).into_any_element(),
            Num::Corner(i) => corner_icon(i, t.muted).into_any_element(),
        };
        let shown = if unit.is_empty() { number(v) } else { format!("{} {unit}", number(v)) };
        let muted = t.muted;
        let field = frame()
            .id(id)
            .border_color(if scrubbing { t.accent } else { t.border })
            .cursor(CursorStyle::ResizeLeftRight)
            .when(!scrubbing, |d| d.hover(move |d| d.border_color(muted)))
            .child(div().absolute().left_0().top_0().bottom_0().w(relative(scrub_frac(which, v))).bg(t.accent.opacity(0.13)))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .child(icon)
                    .child(div().flex_1())
                    .child(div().text_size(px(12.)).text_color(t.text).child(shown)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |d, ev: &MouseDownEvent, _, cx| {
                    let x = f32::from(ev.position.x);
                    d.insp.field = None;
                    d.insp.scrub = Some(Scrub { which, from_x: x, start: v, last_x: x, value: v, moved: false });
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        div().flex().child(field)
    }

    /// The pointer moving while a number is pressed: past a few points, a
    /// scrub, each point of the way stepping it. False when none is pressed.
    pub(super) fn scrub_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let Some(sc) = self.insp.scrub.as_mut() else { return false };
        let x = f32::from(ev.position.x);
        if !sc.moved {
            if (x - sc.from_x).abs() < SCRUB_SLOP {
                return true;
            }
            sc.moved = true;
            sc.last_x = x;
        }
        let rate = if sc.which == Num::Size { SCRUB_SIZE } else { SCRUB_RADIUS };
        let rate = if ev.modifiers.shift { rate * SCRUB_SHIFT } else { rate };
        let (lo, hi) = num_range(sc.which);
        // Kept within range, so turning back changes it at once.
        sc.value = (sc.value + (x - sc.last_x) * rate).clamp(lo, hi);
        sc.last_x = x;
        let (which, to) = (sc.which, sc.value.round());
        for id in self.style_targets() {
            if let Some(b) = self.m.block_mut(&id) {
                set_num(b, which, to);
            }
        }
        cx.notify();
        true
    }

    /// Letting go of a pressed number: a scrub is saved; a press that did not
    /// move types a number in. False when none is pressed.
    pub(super) fn scrub_up(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(sc) = self.insp.scrub.take() else { return false };
        if sc.moved {
            self.request_save(cx);
            cx.notify();
        } else {
            self.type_number(sc.which, sc.start, window, cx);
        }
        true
    }

    /// Puts a field over a number, its text selected; ↵, Esc or a click elsewhere sets it.
    fn type_number(&mut self, which: Num, v: f32, window: &mut Window, cx: &mut Context<Self>) {
        let t = Theme::of(window);
        let editor = cx.new(|cx| {
            let mut e = Editor::new(number(v), false, window, cx);
            e.font_size = 12.;
            e.color = t.text;
            e.muted = t.muted;
            e.accent = t.accent;
            e.align = 0.5;
            e.valign = 0.5;
            e.select_all_text();
            e
        });
        let sub = cx.subscribe(&editor, move |d: &mut DeckView, ed, ev: &EditorEvent, cx| {
            if matches!(ev, EditorEvent::Done | EditorEvent::Blurred) && d.insp.field.as_ref().is_some_and(|f| f.which == which) {
                let typed = parse_number(&ed.read(cx).text);
                d.insp.field = None;
                match typed {
                    Some(to) => d.restyle(move |b| set_num(b, which, to), cx),
                    None => cx.notify(),
                }
            }
        });
        crate::webfocus::release();
        window.focus(&editor.focus_handle(cx));
        self.insp.field = Some(Field { which, editor, _sub: sub });
        cx.notify();
    }

    /// Opens the font list, its search focused, or closes it.
    fn toggle_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.insp.picker.take().is_some() {
            cx.notify();
            return;
        }
        self.insp.palette = None;
        let t = Theme::of(window);
        let search = cx.new(|cx| {
            let mut e = Editor::new(String::new(), false, window, cx);
            e.placeholder = "搜索字体".into();
            e.font_size = 12.;
            e.color = t.text;
            e.muted = t.muted;
            e.accent = t.accent;
            e.valign = 0.5;
            e
        });
        let sub = cx.subscribe(&search, |d: &mut DeckView, ed, ev: &EditorEvent, cx| match ev {
            EditorEvent::Changed => {
                let q = ed.read(cx).text.clone();
                d.fill_picker(&q, cx);
                if let Some(pk) = &d.insp.picker {
                    pk.list.scroll_to_item(0, gpui::ScrollStrategy::Top);
                }
                cx.notify();
            }
            // ↵ or Esc closes it; a click on a font takes the keyboard but leaves it open.
            EditorEvent::Done => {
                d.insp.picker = None;
                cx.notify();
            }
            EditorEvent::Blurred => {}
        });
        crate::webfocus::release();
        window.focus(&search.focus_handle(cx));
        self.insp.picker =
            Some(Picker { at: Instant::now(), search, rows: vec![], list: UniformListScrollHandle::new(), waiting: false, _sub: sub });
        self.fill_picker("", cx);
        cx.notify();
    }

    /// The list's rows for a search: with none, the fonts this deck uses, then every font.
    fn fill_picker(&mut self, query: &str, cx: &mut Context<Self>) {
        let Some(all) = font::families() else {
            // Still indexing: look again shortly.
            if let Some(pk) = self.insp.picker.as_mut().filter(|pk| !pk.waiting) {
                pk.waiting = true;
                cx.spawn(async move |this, cx| {
                    while font::families().is_none() {
                        cx.background_executor().timer(Duration::from_millis(120)).await;
                    }
                    let _ = this.update(cx, |d, cx| {
                        let q = d.insp.picker.as_ref().map(|pk| pk.search.read(cx).text.clone()).unwrap_or_default();
                        d.fill_picker(&q, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            return;
        };
        let font_row = |f: &font::Family| Row::Font { name: f.name.clone(), shown: f.shown.clone() };
        let mut rows = Vec::new();
        if query.trim().is_empty() {
            let mut used: Vec<String> = Vec::new();
            for b in &self.m.blocks {
                if b.kind.bare() && !b.style.font.is_empty() && !used.contains(&b.style.font) {
                    used.push(b.style.font.clone());
                }
            }
            if !used.is_empty() {
                rows.push(Row::Head("本画布在用"));
                rows.extend(used.iter().map(|n| Row::Font { name: n.clone(), shown: font_shown(n) }));
            }
            rows.push(Row::Head("全部字体"));
            rows.push(Row::Font { name: String::new(), shown: "系统字体".into() });
            rows.extend(all.iter().map(font_row));
        } else {
            rows.extend(font::search(all, query).into_iter().map(font_row));
        }
        if let Some(pk) = self.insp.picker.as_mut() {
            pk.rows = rows;
        }
    }

    /// The font list, left of the panel, sliding out from under it.
    fn picker_view(&mut self, targets: &[String], panel_right: f32, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pk = self.insp.picker.as_ref()?;
        let e = ease((pk.at.elapsed().as_secs_f32() / PICKER_IN.as_secs_f32()).min(1.));
        let right = panel_right + PANEL_W + PICKER_GAP + (1. - e) * PICKER_SLIDE;
        let current = targets.first().and_then(|id| self.m.block(id)).map(|b| b.style.font.clone()).unwrap_or_default();
        let (search, rows, list) = (pk.search.clone(), pk.rows.clone(), pk.list.clone());
        let indexed = font::families().is_some();

        let field = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .mx(px(10.))
            .mt(px(10.))
            .mb(px(6.))
            .px(px(8.))
            .h(px(30.))
            .flex_none()
            .rounded(px(7.))
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .child(svg().path(icons::path("search")).size(px(13.)).text_color(t.muted))
            .child(div().flex_1().min_w_0().h_full().child(search));
        let body = if !indexed {
            note("正在建立字体索引…", t)
        } else if rows.is_empty() {
            note("没有找到字体", t)
        } else {
            let t = *t;
            let count = rows.len();
            uniform_list(
                "font-list",
                count,
                cx.processor(move |_: &mut DeckView, range: Range<usize>, _, cx| {
                    range.map(|i| font_row_view(i, &rows[i], &current, &t, cx)).collect::<Vec<_>>()
                }),
            )
            .track_scroll(list)
            .flex_1()
            .pb(px(6.))
            .into_any_element()
        };
        Some(
            div()
                .id("font-picker")
                .absolute()
                .top(px(PANEL_TOP))
                .right(px(right))
                .w(px(PICKER_W))
                .h(px(picker_h(self.view.1)))
                .flex()
                .flex_col()
                .opacity(e)
                .bg(t.surface)
                .border_1()
                .border_color(t.border)
                .rounded(px(12.))
                .overflow_hidden()
                .shadow(vec![gpui::BoxShadow {
                    color: t.shade,
                    offset: point(px(0.), px(6.)),
                    blur_radius: px(20.),
                    spread_radius: px(0.),
                }])
                .occlude()
                .font_family(".SystemUIFont")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(field)
                .child(body)
                .into_any_element(),
        )
    }

    /// A color slot's choices: none, the palette, and a custom color from the Colors panel.
    fn swatches(&mut self, slot: Slot, spec: &str, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let ring = |on: bool| if on { t.accent } else { gpui::transparent_black() };
        let swatch = |id: SharedString, on: bool| {
            div()
                .id(id)
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(ring(on))
                .cursor_pointer()
        };
        let mut row = div().flex().flex_row().flex_wrap().gap(px(2.));
        // None: a crossed-out dot.
        let off = spec == "none";
        let border = t.border;
        let danger = t.danger;
        row = row.child(
            swatch(SharedString::from(format!("{slot:?}-none")), off)
                .child(
                    div().size(px(14.)).rounded_full().border_1().border_color(border).child(
                        canvas(|_, _, _| (), move |b, _, window, _| {
                            let mut p = gpui::PathBuilder::stroke(px(1.5));
                            p.move_to(point(b.origin.x + px(2.), b.origin.y + b.size.height - px(2.)));
                            p.line_to(point(b.origin.x + b.size.width - px(2.), b.origin.y + px(2.)));
                            if let Ok(p) = p.build() {
                                window.paint_path(p, danger);
                            }
                        })
                        .size_full(),
                    ),
                )
                .on_click(cx.listener(move |d, _, _, cx| d.restyle(move |b| *slot_of(b, slot) = "none".into(), cx))),
        );
        for ink in INKS {
            let Paint::Color(color) = paint(ink.name, false, t) else { continue };
            let name = ink.name;
            row = row.child(
                swatch(SharedString::from(format!("{slot:?}-{name}")), spec == name)
                    .child(div().size(px(14.)).rounded_full().bg(color))
                    .on_click(cx.listener(move |d, _, _, cx| d.restyle(move |b| *slot_of(b, slot) = name.into(), cx))),
            );
        }
        // Custom: the palette beside the panel, for this color.
        let custom = spec.starts_with('#');
        let open = self.insp.palette.as_ref().is_some_and(|pl| pl.slot == slot);
        let shown: Hsla = match paint(spec, false, t) {
            Paint::Color(c) => c,
            _ => t.accent,
        };
        row.child(
            swatch(SharedString::from(format!("{slot:?}-custom")), custom || open)
                .child(
                    div()
                        .size(px(14.))
                        .rounded_full()
                        .when(custom, |d| d.bg(shown))
                        .when(!custom, |d| d.border_1().border_color(t.muted))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!custom, |d| d.child(svg().path(icons::path("plus")).size(px(10.)).text_color(t.muted))),
                )
                .on_click(cx.listener(move |d, _, window, cx| d.toggle_palette(slot, window, cx))),
        )
        .child(eyedropper(SharedString::from(format!("{slot:?}-sample")), CHIP, t).on_click(cx.listener(move |d, _, _, _| {
            d.color_slot = slot;
            nativepanel::sample_color(&d.panel_tx);
        })))
    }

    /// Opens the palette for a color, or closes it; the font list makes way.
    fn toggle_palette(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        if self.insp.palette.take().is_some_and(|pl| pl.slot == slot) {
            cx.notify();
            return;
        }
        self.insp.picker = None;
        let t = Theme::of(window);
        let spec = self.style_targets().first().and_then(|id| self.m.block(id)).map(|b| slot_value(b, slot)).unwrap_or_default();
        let hex = cx.new(|cx| {
            let mut e = Editor::new(if spec.starts_with('#') { spec.clone() } else { String::new() }, false, window, cx);
            e.placeholder = "#rrggbb".into();
            e.font_size = 12.;
            e.color = t.text;
            e.muted = t.muted;
            e.accent = t.accent;
            e.valign = 0.5;
            e
        });
        let sub = cx.subscribe(&hex, move |d: &mut DeckView, ed, ev: &EditorEvent, cx| {
            let text = ed.read(cx).text.clone();
            match (ev, parse_hex(&text)) {
                // A whole color typed is put on at once.
                (EditorEvent::Changed, Some(h)) => d.restyle(move |b| *slot_of(b, slot) = h.clone(), cx),
                (EditorEvent::Changed, None) => {}
                // Left half typed, the field goes back to the color in use.
                (EditorEvent::Done | EditorEvent::Blurred, _) => {
                    let now = d.style_targets().first().and_then(|id| d.m.block(id)).map(|b| slot_value(b, slot)).unwrap_or_default();
                    ed.update(cx, |e, cx| e.set_text(if now.starts_with('#') { now } else { String::new() }, cx));
                }
            }
        });
        self.insp.palette = Some(Palette { slot, at: Instant::now(), hex, _sub: sub });
        cx.notify();
    }

    /// The palette, left of the panel, sliding out from under it: preset
    /// shades, the deck's own colors, and a field for a hex code beside the
    /// eyedropper and the system's Colors panel.
    fn palette_view(&mut self, targets: &[String], panel_right: f32, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pl = self.insp.palette.as_ref()?;
        let e = ease((pl.at.elapsed().as_secs_f32() / PICKER_IN.as_secs_f32()).min(1.));
        let right = panel_right + PANEL_W + PICKER_GAP + (1. - e) * PICKER_SLIDE;
        let (slot, field) = (pl.slot, pl.hex.clone());
        let first = targets.first().and_then(|id| self.m.block(id))?;
        let spec = slot_value(first, slot);
        let current = parse_hex(&spec).filter(|_| spec.starts_with('#'));
        let shown: Hsla = match paint(&spec, false, t) {
            Paint::Color(c) => c,
            _ => t.accent,
        };
        let recent = self.recent_colors();
        let title = match slot {
            Slot::Fill => "填充颜色",
            Slot::Stroke => "边框颜色",
            Slot::Color => "文字颜色",
        };

        let chip = |id: String, h: String, cx: &mut Context<Self>| {
            let on = current.as_deref() == Some(h.as_str());
            let color = hex_color(&h);
            div()
                .id(SharedString::from(id))
                .size(px(CHIP))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .border_2()
                .border_color(if on { t.accent } else { gpui::transparent_black() })
                .cursor_pointer()
                .child(div().size(px(CHIP - 6.)).rounded(px(4.)).bg(color).border_1().border_color(t.border.opacity(0.7)))
                .on_click(cx.listener(move |d, _, _, cx| d.set_color(slot, h.clone(), cx)))
        };
        let grid = div().flex().flex_col().gap(px(CHIP_GAP)).children(presets().into_iter().enumerate().map(|(r, row)| {
            div()
                .flex()
                .flex_row()
                .gap(px(CHIP_GAP))
                .children(row.into_iter().enumerate().map(|(c, h)| chip(format!("pal-{r}-{c}"), h, cx)).collect::<Vec<_>>())
        }).collect::<Vec<_>>());
        let recents = (!recent.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .h(px(PAL_RECENT_H))
                .child(div().h(px(14.)).text_size(px(11.)).text_color(t.muted).child("本画布在用"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(CHIP_GAP))
                        .children(recent.into_iter().enumerate().map(|(i, h)| chip(format!("pal-recent-{i}"), h, cx)).collect::<Vec<_>>()),
                )
        });
        let hex_field = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .rounded(px(7.))
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .child(div().size(px(14.)).flex_none().rounded(px(3.)).bg(shown).border_1().border_color(t.border))
            .child(div().flex_1().min_w_0().h_full().child(field));
        let system = toggle("pal-system".into(), false, t)
            .size(px(PAL_FIELD_H))
            .flex_none()
            .border_1()
            .border_color(t.border)
            .child(svg().path(icons::path("palette")).size(px(15.)).text_color(t.text))
            .on_click(cx.listener(move |d, _, _, _| {
                d.color_slot = slot;
                nativepanel::color_panel(&hex(shown), &d.panel_tx);
            }));
        let sample = eyedropper("pal-sample".into(), PAL_FIELD_H, t).border_1().border_color(t.border).on_click(cx.listener(move |d, _, _, _| {
            d.color_slot = slot;
            nativepanel::sample_color(&d.panel_tx);
        }));
        let has_recent = recents.is_some();
        Some(
            div()
                .id("palette")
                .absolute()
                .top(px(PANEL_TOP))
                .right(px(right))
                .w(px(PICKER_W))
                .h(px(palette_h(has_recent)))
                .flex()
                .flex_col()
                .gap(px(PAL_GAP))
                .p(px(PAL_PAD))
                .opacity(e)
                .bg(t.surface)
                .border_1()
                .border_color(t.border)
                .rounded(px(12.))
                .overflow_hidden()
                .shadow(vec![gpui::BoxShadow {
                    color: t.shade,
                    offset: point(px(0.), px(6.)),
                    blur_radius: px(20.),
                    spread_radius: px(0.),
                }])
                .occlude()
                .font_family(".SystemUIFont")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(div().h(px(PAL_TITLE_H)).text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child(title))
                .child(grid)
                .children(recents)
                .child(div().h(px(PAL_FIELD_H)).flex().flex_row().gap(px(6.)).child(hex_field).child(sample).child(system))
                .into_any_element(),
        )
    }
}

/// The palette's height, laid out as `palette_view` lays it out.
fn palette_h(recent: bool) -> f32 {
    let rows = SHADES.len() as f32 + 1.;
    let grid = rows * CHIP + (rows - 1.) * CHIP_GAP;
    let mut h = 2. * PAL_PAD + PAL_TITLE_H + PAL_GAP + grid + PAL_GAP + PAL_FIELD_H + 2.;
    if recent {
        h += PAL_RECENT_H + PAL_GAP;
    }
    h
}

/// The palette's preset colors as `#rrggbb`: a row of grays, white to black,
/// then a row for each shade with a column for each hue.
fn presets() -> Vec<Vec<String>> {
    let grays = (0..HUES.len()).map(|i| hex(gpui::hsla(0., 0., 1. - i as f32 / (HUES.len() - 1) as f32, 1.))).collect();
    let mut rows = vec![grays];
    for (sat, light) in SHADES {
        rows.push(HUES.iter().map(|&h| hex(gpui::hsla(h / 360., sat, light, 1.))).collect());
    }
    rows
}

/// A color typed as hex, `#rgb` or `#rrggbb`, the `#` optional, as `#rrggbb`.
fn parse_hex(s: &str) -> Option<String> {
    let v = s.trim().trim_start_matches('#');
    if !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = v.to_ascii_lowercase();
    match v.len() {
        6 => Some(format!("#{v}")),
        3 => Some(format!("#{}", v.chars().flat_map(|c| [c, c]).collect::<String>())),
        _ => None,
    }
}

/// A `#rrggbb` the palette made as a color to draw.
fn hex_color(h: &str) -> Hsla {
    let rgb = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0);
    gpui::rgb(rgb).into()
}

/// The eyedropper's button, `side` square.
fn eyedropper(id: SharedString, side: f32, t: &Theme) -> gpui::Stateful<gpui::Div> {
    toggle(id, false, t).size(px(side)).flex_none().child(svg().path(icons::path("pipette")).size(px(side * 0.55)).text_color(t.text))
}

/// The font list's height in a window vh tall.
fn picker_h(vh: f32) -> f32 {
    (vh - PANEL_TOP - 60.).clamp(160., 480.)
}

/// A family as the panel names it: in the system's language when the index has that.
fn font_shown(name: &str) -> String {
    if name.is_empty() {
        return "系统字体".into();
    }
    font::families().and_then(|all| all.iter().find(|f| f.name == name)).map_or_else(|| name.to_string(), |f| f.shown.clone())
}

/// A row of the font list: a family's name set in that family, its other name small beside it.
fn font_row_view(i: usize, row: &Row, current: &str, t: &Theme, cx: &mut Context<DeckView>) -> AnyElement {
    match row {
        Row::Head(title) => div()
            .h(px(ROW_H))
            .flex()
            .items_end()
            .px(px(16.))
            .pb(px(6.))
            .text_size(px(11.))
            .text_color(t.muted)
            .child(*title)
            .into_any_element(),
        Row::Font { name, shown } => {
            let on = name == current;
            let pick = name.clone();
            div()
                .id(("font-row", i))
                .h(px(ROW_H))
                .mx(px(6.))
                .px(px(10.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .rounded(px(7.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent.opacity(0.14)))
                .when(!on, |d| d.hover(|d| d.bg(t.surface_hover)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(15.))
                        .text_color(if on { t.accent } else { t.text })
                        .when(!name.is_empty(), |d| d.font_family(name.clone()))
                        .child(shown.clone()),
                )
                .when(!name.is_empty() && name != shown, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .max_w(px(110.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(10.5))
                            .text_color(t.muted)
                            .child(name.clone()),
                    )
                })
                .when(on, |d| d.child(svg().path(icons::path("check")).size(px(14.)).flex_none().text_color(t.accent)))
                .on_click(cx.listener(move |d, _, _, cx| {
                    let font = pick.clone();
                    d.restyle(move |b| b.style.font = font.clone(), cx)
                }))
                .into_any_element()
        }
    }
}

fn note(text: &'static str, t: &Theme) -> AnyElement {
    div().flex_1().flex().items_center().justify_center().text_size(px(12.)).text_color(t.muted).child(text).into_any_element()
}

/// A number as the panel shows it: whole, or to a tenth.
fn number(v: f32) -> String {
    let v = (v * 10.).round() / 10.;
    if v.fract() == 0. { format!("{v:.0}") } else { format!("{v:.1}") }
}

/// What was typed, as a number: units and spaces around it are let be.
fn parse_number(s: &str) -> Option<f32> {
    let s = s.trim().trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace());
    s.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// What a stepper shows for the block.
fn num_of(b: &Block, which: Num) -> f32 {
    match which {
        Num::Size => styled::font_size(b),
        Num::Radius => styled::radius(b),
        Num::Corner(i) => styled::radii(b)[i],
    }
}

/// What a number may be, least to most.
fn num_range(which: Num) -> (f32, f32) {
    match which {
        Num::Size => (6., 400.),
        Num::Radius | Num::Corner(_) => (0., MAX_RADIUS),
    }
}

/// How full a scrubber's bar is: how far the number is along the range it is
/// mostly set in (it may go past it).
fn scrub_frac(which: Num, v: f32) -> f32 {
    let (lo, hi) = match which {
        Num::Size => (6., 120.),
        Num::Radius | Num::Corner(_) => (0., 64.),
    };
    ((v - lo) / (hi - lo)).clamp(0., 1.)
}

/// Sets a number on a block, kept within what it may be.
fn set_num(b: &mut Block, which: Num, v: f32) {
    let v = (v * 10.).round() / 10.;
    match which {
        Num::Size => b.style.size = v.clamp(6., 400.),
        Num::Radius => {
            b.style.radius = Some(v.clamp(0., MAX_RADIUS));
            b.style.corners = None;
        }
        Num::Corner(i) => {
            let mut c = styled::radii(b);
            c[i] = v.clamp(0., MAX_RADIUS);
            b.style.corners = Some(c);
        }
    }
}

/// A rounded corner, drawn turned to the corner it stands for.
fn corner_icon(i: usize, color: Hsla) -> impl IntoElement {
    canvas(|_, _, _| (), move |b, _, window, _| {
        let (x, y, s) = (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width));
        // The top-left corner in a unit square, mirrored for the others.
        let (fx, fy) = (matches!(i, 1 | 2), matches!(i, 2 | 3));
        let at = |u: f32, v: f32| {
            let u = if fx { 1. - u } else { u };
            let v = if fy { 1. - v } else { v };
            point(px(x + 1.5 + u * (s - 3.)), px(y + 1.5 + v * (s - 3.)))
        };
        let mut p = gpui::PathBuilder::stroke(px(1.5));
        p.move_to(at(0., 1.));
        p.line_to(at(0., 0.55));
        p.curve_to(at(0.55, 0.), at(0., 0.));
        p.line_to(at(1., 0.));
        if let Ok(p) = p.build() {
            window.paint_path(p, color);
        }
    })
    .size(px(12.))
    .flex_none()
}

fn along(v: &str) -> f32 {
    match v {
        "center" | "middle" => 0.5,
        "right" | "bottom" => 1.,
        _ => 0.,
    }
}

fn slot_of(b: &mut Block, slot: Slot) -> &mut String {
    match slot {
        Slot::Fill => &mut b.style.fill,
        Slot::Stroke => &mut b.style.stroke,
        Slot::Color => &mut b.style.color,
    }
}

fn slot_value(b: &Block, slot: Slot) -> String {
    match slot {
        Slot::Fill => b.style.fill.clone(),
        Slot::Stroke => b.style.stroke.clone(),
        Slot::Color => b.style.color.clone(),
    }
}

fn section(title: &str, t: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(div().text_size(px(11.)).text_color(t.muted).child(title.to_string()))
}

/// A button that is on or off.
fn toggle(id: SharedString, on: bool, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .text_color(if on { t.accent } else { t.text })
        .when(on, |d| d.bg(t.accent.opacity(0.14)))
        .when(!on, |d| d.bg(t.background).hover(|d| d.bg(t.surface_hover)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_typed_and_set_stay_in_range() {
        assert_eq!(parse_number(" 18 pt"), Some(18.));
        assert_eq!(parse_number("12.5px"), Some(12.5));
        assert_eq!(parse_number("abc"), None);
        assert_eq!((number(16.), number(12.25)), ("16".to_string(), "12.3".to_string()));

        let mut b = Block::new(BlockKind::Shape);
        set_num(&mut b, Num::Size, 2.);
        assert_eq!(b.style.size, 6., "no smaller than 6 pt");
        set_num(&mut b, Num::Corner(2), 20.);
        assert_eq!(b.style.corners, Some([12., 12., 20., 12.]), "the other corners keep the radius they had");
        set_num(&mut b, Num::Radius, -3.);
        assert_eq!((b.style.radius, b.style.corners), (Some(0.), None), "one radius for all four again");
    }

    #[test]
    fn colors_typed_and_offered() {
        assert_eq!(parse_hex("#3E63DD"), Some("#3e63dd".into()));
        assert_eq!(parse_hex(" f0a "), Some("#ff00aa".into()));
        assert_eq!((parse_hex("#12345"), parse_hex("blue"), parse_hex("")), (None, None, None));
        let rows = presets();
        assert_eq!(rows.len(), SHADES.len() + 1);
        assert!(rows.iter().all(|r| r.len() == HUES.len() && r.iter().all(|h| parse_hex(h).as_deref() == Some(h.as_str()))));
        assert_eq!((rows[0][0].as_str(), rows[0][HUES.len() - 1].as_str()), ("#ffffff", "#000000"));
        assert_eq!(crate::theme::hex(hex_color("#3e63dd")), "#3e63dd");
    }

    #[test]
    fn scrubbing_keeps_a_bar_in_bounds() {
        assert_eq!((scrub_frac(Num::Size, 2.), scrub_frac(Num::Size, 500.)), (0., 1.));
        assert!((scrub_frac(Num::Radius, 32.) - 0.5).abs() < 1e-6);
        assert_eq!(num_range(Num::Corner(1)), (0., MAX_RADIUS));
    }

    #[test]
    fn the_slide_starts_fast_and_settles() {
        assert_eq!((ease(0.), ease(1.)), (0., 1.));
        assert!(ease_rate(0.) > ease_rate(0.5) && ease_rate(1.) == 0.);
    }
}
