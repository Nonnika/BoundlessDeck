//! A block on the canvas: its card (or bare 文字 and 图形), title bar,
//! resize handle, and the pop-in it enters with.

use std::collections::HashSet;
use std::time::Instant;

use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, PathBuilder, ScrollWheelEvent, SharedString, Styled, Window, canvas, div, linear_color_stop,
    linear_gradient, point, prelude::FluentBuilder, px, svg,
};

use super::{DeckView, Drag, EditPart, Popup, TITLE_FONT, TITLE_H, popin, styled};
use crate::lifespan;
use crate::model::{Block, BlockKind, LOD_SCALE};
use crate::theme::{Paper, Theme};
use crate::{icons, tape};

impl DeckView {
    pub(super) fn block_view(&mut self, id: &str, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(b) = self.m.block(id).cloned() else { return vec![] };
        let s = self.m.cam.s;
        let (sx, sy) = self.m.cam.to_screen(b.x, b.y);
        let (w, h) = (b.w * s, b.h * s);
        let selected = self.m.sel.contains(id);
        let opacity = if b.kind == BlockKind::Note { lifespan::note_opacity(b.expires, self.m.now_ms()) } else { 1.0 };
        let mut out = Vec::new();

        // Just made: it opens out of a line (`popin`); the card sits in a clip that grows.
        let pop = if s >= LOD_SCALE { self.rts.get_mut(id).and_then(|r| r.pop.frame(Instant::now())) } else { None };
        let clip = pop.map(|f| {
            let (cw, ch) = ((w * f.width).max(2.), (h * f.height).max(2.));
            ((w - cw) / 2., (h - ch) / 2., cw, ch)
        });
        self.animating |= pop.is_some();
        if b.kind.bare() {
            return self.bare_view(&b, t, (sx, sy, w, h), selected, window, cx);
        }

        let note = b.kind == BlockKind::Note;
        let paper = Paper::of(t);
        let eid = id.to_string();
        let mut card = div()
            .id(SharedString::from(format!("block:{id}")))
            .absolute()
            .left(px(clip.map_or(sx, |c| -c.0)))
            .top(px(clip.map_or(sy, |c| -c.1)))
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .overflow_hidden()
            .occlude()
            .opacity(opacity)
            .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_card_down(&eid, ev, window, cx)));
        card = if note {
            card.bg(linear_gradient(180., linear_color_stop(paper.top, 0.), linear_color_stop(paper.bottom, 1.)))
                .shadow(vec![gpui::BoxShadow {
                    color: paper.shadow,
                    offset: point(px(0.), px(3. * s.min(1.5))),
                    blur_radius: px(10. * s.min(1.5)),
                    spread_radius: px(0.),
                }])
        } else {
            card.bg(t.surface).shadow(vec![gpui::BoxShadow {
                color: t.shade,
                offset: point(px(0.), px(2. * s.min(1.5))),
                blur_radius: px(8. * s.min(1.5)),
                spread_radius: px(0.),
            }])
        };
        card = if selected {
            card.border(px(2.)).border_color(t.accent)
        } else if note {
            card
        } else {
            card.border_1().border_color(t.border)
        };

        // Zoomed out, the body is a mask of the card's own background; coming
        // back, the mask fades off the content.
        let mask = |d: Div| {
            if note {
                d.bg(linear_gradient(180., linear_color_stop(paper.top, 0.), linear_color_stop(paper.bottom, 1.)))
            } else {
                d.bg(t.surface)
            }
        };
        let rt = self.rts.entry(id.to_string()).or_default();
        // A block coming back from LOD waits its turn (`LOD_WAKE_PER_FRAME`).
        let held = s >= LOD_SCALE && rt.lod && self.lod_wake == 0;
        if held {
            self.animating = true;
        } else if s >= LOD_SCALE && rt.lod {
            self.lod_wake -= 1;
        }
        if s < LOD_SCALE || held {
            rt.lod = true;
            // Zoomed out, a card is its frame, its title and the mask.
            let cs = self.content_scale();
            let color = if note { paper.text } else { t.text };
            card = card
                .child(
                    div()
                        .h(px(TITLE_H * s))
                        .flex_none()
                        .px(px(8. * s))
                        .flex()
                        .items_center()
                        .text_size(px(TITLE_FONT * cs))
                        .text_color(color)
                        .whitespace_nowrap()
                        .child(b.title.clone()),
                )
                .child(mask(div().flex_1().w_full().min_h_0()));
            out.push(card.into_any_element());
            return out;
        }

        // Back from LOD: the mask over the body fades off under the title it kept.
        let now = Instant::now();
        if std::mem::take(&mut rt.lod) {
            rt.appear.reveal(now);
        }
        let appearing = rt.appear.tick(now);
        let appear = rt.appear.alpha(now);
        self.animating |= appearing;
        card = card.child(self.title_bar(&b, t, &paper, window, cx));
        let body = self.body(&b, t, &paper, window, cx);
        // A block worked in keeps the wheel: its content scrolls (it was
        // offered the wheel first), and the canvas does not pan under it.
        // GPUI's scrolling leaves the event going, so it is stopped here.
        let wid = id.to_string();
        // While zooming, text holds its scale (`zoomlayer`); a body of text is
        // laid out at that scale too, in a box of its own size, so its lines
        // do not wrap anew each frame as the card grows or shrinks around it.
        let body = if holds_layout(&b) {
            let cs = self.content_scale();
            div().w(px(b.w * cs)).h(px((b.h - TITLE_H) * cs)).child(body).into_any_element()
        } else {
            body
        };
        let mut body_box = div()
            .relative()
            .flex_1()
            .w_full()
            .min_h_0()
            .overflow_hidden()
            .child(body)
            .on_scroll_wheel(cx.listener(move |d, ev: &ScrollWheelEvent, _, cx| {
                let zoom = ev.modifiers.platform || ev.modifiers.control;
                let gesture = d.canvas_scroll_until.is_some_and(|t| t > Instant::now());
                let worked_in = d.active.as_deref() == Some(wid.as_str()) || d.editing(&wid, EditPart::Body).is_some();
                if worked_in && !zoom && !gesture {
                    cx.stop_propagation();
                }
            }));
        if let Some(cover) = self.body_cover(&b, cx) {
            body_box = body_box.child(cover);
        }
        if appearing {
            body_box = body_box.child(mask(div().absolute().top_0().left_0().size_full()).opacity(1. - appear));
        }
        card = card.child(body_box);
        if note {
            // The folded corner: the canvas shows where it was, the flap lies on the sheet.
            let fold = 22. * s;
            let (bg, flap) = (t.background, paper.flap);
            card = card.child(
                canvas(|_, _, _| (), move |bounds, _, window, _| {
                    let (x1, y1) = (bounds.right(), bounds.bottom());
                    let f = px(fold);
                    let mut cut = PathBuilder::fill();
                    cut.move_to(point(x1 - f, y1));
                    cut.line_to(point(x1, y1 - f));
                    cut.line_to(point(x1, y1));
                    cut.close();
                    if let Ok(p) = cut.build() {
                        window.paint_path(p, bg);
                    }
                    let mut fl = PathBuilder::fill();
                    fl.move_to(point(x1 - f, y1));
                    fl.line_to(point(x1, y1 - f));
                    fl.line_to(point(x1 - f, y1 - f));
                    fl.close();
                    if let Ok(p) = fl.build() {
                        window.paint_path(p, flap);
                    }
                })
                .absolute()
                .left_0()
                .top_0()
                .size_full(),
            );
        }
        if let (Some(f), Some((ox, oy, cw, ch))) = (pop, clip) {
            out.extend(popin_layers(f, (sx + ox, sy + oy, cw, ch), card.into_any_element(), if note { paper.top } else { t.surface }, t, s));
            return out;
        }
        out.push(card.into_any_element());

        out.extend(self.resize_handle(id, (sx, sy, w, h), note, t, cx));
        out
    }

    /// A 文字 or 图形, at sx, sy, w by h: drawn as it is, without a card.
    /// Selected, worked in or edited, an outline goes round it and its title
    /// shows above it, where a press moves it and a double click renames it.
    pub(super) fn bare_view(
        &mut self,
        b: &Block,
        t: &Theme,
        (sx, sy, w, h): (f32, f32, f32, f32),
        selected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let s = self.m.cam.s;
        let cs = self.content_scale();
        let lod = s < LOD_SCALE;
        let look = styled::look(b, t);
        // The words are laid out at the scale text holds while zooming (`zoomlayer`).
        let hold = if s > 0. { cs / s } else { 1. };
        let editor = if lod { None } else { self.editing(&b.id, EditPart::Body).cloned() };
        if let Some(ed) = &editor {
            ed.update(cx, |e, _| {
                styled::dress(e, &look, cs);
                e.muted = look.color.opacity(0.45);
                e.accent = t.accent;
                e.placeholder = "输入文字".into();
            });
        }
        // A 文字's words are Markdown; a 图形's are drawn as typed.
        let md = (b.kind == BlockKind::Text && !lod && editor.is_none() && !b.text.trim().is_empty()).then(|| {
            let colors = styled::md_colors(&look, t);
            (self.text_markdown(&b.id, &b.text, &colors, t.dark), colors)
        });
        let eid = b.id.clone();
        let card = div()
            .id(SharedString::from(format!("block:{}", b.id)))
            .absolute()
            .left(px(sx))
            .top(px(sy))
            .w(px(w))
            .h(px(h))
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_card_down(&eid, ev, window, cx)))
            .child(styled::view(b, &look, (w, h), hold, cs, editor, md, lod));
        let mut out = vec![card.into_any_element()];
        let shown = selected
            || self.active.as_deref() == Some(b.id.as_str())
            || self.editing.as_ref().is_some_and(|e| e.id == b.id);
        if !shown || lod {
            return out;
        }
        // The outline: drawn only, it takes no clicks.
        out.push(
            div()
                .absolute()
                .left(px(sx - 3.))
                .top(px(sy - 3.))
                .w(px(w + 6.))
                .h(px(h + 6.))
                .border(px(1.5))
                .border_color(t.accent)
                .rounded(px(4.))
                .into_any_element(),
        );
        let paper = Paper::of(t);
        let bar_h = TITLE_H * s;
        let eid = b.id.clone();
        out.push(
            div()
                .id(SharedString::from(format!("title:{}", b.id)))
                .absolute()
                .left(px(sx - 3.))
                .top(px(sy - bar_h - 8.))
                .w(px((w + 6.).max(160.)))
                .h(px(bar_h))
                .bg(t.surface)
                .border_1()
                .border_color(t.border)
                .rounded(px(6.))
                .shadow(vec![gpui::BoxShadow {
                    color: t.shade,
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(8.),
                    spread_radius: px(0.),
                }])
                .overflow_hidden()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_card_down(&eid, ev, window, cx)))
                .child(self.title_bar(b, t, &paper, window, cx))
                .into_any_element(),
        );
        out.extend(self.resize_handle(&b.id, (sx, sy, w, h), false, t, cx));
        out
    }

    /// The resize handle, in the bottom-right corner of a block at sx, sy, w by h.
    pub(super) fn resize_handle(&mut self, id: &str, (sx, sy, w, h): (f32, f32, f32, f32), note: bool, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let s = self.m.cam.s;
        let hs = (22. * s).max(14.);
        if w >= hs * 3. && h >= hs * 3. {
            self.resize_corners.push([sx + w - hs, sy + h - hs, hs, hs]);
            let eid = id.to_string();
            let grip = t.muted;
            let resizing = matches!(&self.drag, Some(Drag::Resize { id: d, .. }) if d == id);
            return Some(
                div()
                    .id(SharedString::from(format!("resize:{id}")))
                    .absolute()
                    .left(px(sx + w - hs))
                    .top(px(sy + h - hs))
                    .w(px(hs))
                    .h(px(hs))
                    .occlude()
                    .cursor(gpui::CursorStyle::ResizeUpLeftDownRight)
                    .when(!note, |d| {
                        d.child(
                            canvas(|_, _, _| (), move |bounds, _, window, _| {
                                let hovered = bounds.contains(&window.mouse_position()) || resizing;
                                tape::paint_grip(window, bounds, grip.opacity(if hovered { 1.0 } else { 0.45 }));
                            })
                            .size_full(),
                        )
                    })
                    .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_resize_down(&eid, ev, window, cx)))
                    .into_any_element(),
            );
        }
        None
    }

    pub(super) fn title_bar(&mut self, b: &Block, t: &Theme, paper: &Paper, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        let cs = self.content_scale();
        let note = b.kind == BlockKind::Note;
        let color = if note { paper.text } else { t.text };
        // The bar spans the card; what is in it is laid out at the content
        // scale, as wide as the bar is at that scale, so while zooming the
        // title is cut (and its ellipsis placed) the same in every frame.
        let width = if b.kind.bare() { (b.w * cs + 6.).max(160.) } else { b.w * cs };
        let frame = div()
            .h(px(TITLE_H * s))
            .w_full()
            .flex_none()
            .overflow_hidden()
            .when(note, |d| d.bg(paper.header));
        let mut bar = div()
            .h_full()
            .w(px(width))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6. * cs))
            .px(px(if note { 10. } else { 8. } * cs))
            .text_size(px(TITLE_FONT * cs))
            .text_color(color)
            .when(note, |d| d.font_weight(FontWeight::SEMIBOLD));
        if let Some(ed) = self.editing(&b.id, EditPart::Title).cloned() {
            ed.update(cx, |e, _| {
                e.font_size = TITLE_FONT * cs;
                e.color = color;
                e.muted = t.muted;
                e.accent = t.accent;
            });
            bar = bar.child(div().flex_1().min_w_0().child(ed));
        } else {
            bar = bar.child(
                div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(b.title.clone()),
            );
        }
        if note {
            bar = bar.child(self.life_chip(b, paper, cx));
        } else if b.kind == BlockKind::Chart {
            bar = bar.child(self.chart_controls(b, t, cx));
        } else if b.kind.bare() {
            // Its title is all it shows; its style is in the panel on the right.
        } else {
            bar = bar.child(
                div().text_size(px(10. * cs)).text_color(t.muted).whitespace_nowrap().child(b.kind.label()),
            );
        }
        frame.child(bar).into_any_element()
    }

    /// A chart's type chip (a menu of types) and its table button.
    pub(super) fn chart_controls(&mut self, b: &Block, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
        let kind = crate::chart::of(&b.chart);
        let editing = self.editing(&b.id, EditPart::Body).is_some();
        let (id1, id2) = (b.id.clone(), b.id.clone());
        let chip = |id: SharedString| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4. * s))
                .px(px(6. * s))
                .py(px(2. * s))
                .rounded(px(5. * s))
                .text_size(px(11. * s))
                .text_color(t.muted)
                .hover(|d| d.bg(t.surface_hover))
                .cursor_pointer()
                .occlude()
        };
        div()
            .flex()
            .flex_row()
            .gap(px(2. * s))
            .child(
                chip(SharedString::from(format!("chart-kind:{}", b.id)))
                    .child(svg().path(icons::path(crate::chart::icon(kind))).size(px(12. * s)).text_color(t.muted))
                    .child(crate::chart::label(kind))
                    .child(svg().path(icons::path("chevron_down")).size(px(10. * s)).text_color(t.muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |d, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            d.popup = Some(Popup::ChartType {
                                id: id1.clone(),
                                at: (f32::from(ev.position.x), f32::from(ev.position.y) + 10.),
                            });
                            cx.notify();
                        }),
                    ),
            )
            .child(
                chip(SharedString::from(format!("chart-table:{}", b.id)))
                    .when(editing, |d| d.bg(t.accent.opacity(0.15)).text_color(t.accent))
                    .child(svg().path(icons::path("table")).size(px(12. * s)).text_color(if editing { t.accent } else { t.muted }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |d, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if d.editing(&id2, EditPart::Body).is_some() {
                                d.stop_editing(cx);
                            } else {
                                d.m.sel = HashSet::from([id2.clone()]);
                                d.start_editing(&id2, EditPart::Body, window, cx);
                            }
                        }),
                    ),
            )
            .into_any_element()
    }

    /// How long a QuickLook has left; a click picks another span.
    pub(super) fn life_chip(&mut self, b: &Block, paper: &Paper, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
        let label = if b.expires <= 0 { "∞".to_string() } else { lifespan::life_left(b.expires - self.m.now_ms()) };
        let id = b.id.clone();
        div()
            .id(SharedString::from(format!("life:{}", b.id)))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3. * s))
            .px(px(5. * s))
            .rounded(px(4. * s))
            .text_size(px(10.5 * s))
            .text_color(paper.muted)
            .hover(|d| d.bg(paper.flap.opacity(0.5)))
            .cursor_pointer()
            .occlude()
            .child(svg().path(icons::path("timer")).size(px(11. * s)).text_color(paper.muted))
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |d, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    d.popup = Some(Popup::Life {
                        id: id.clone(),
                        at: (f32::from(ev.position.x), f32::from(ev.position.y) + 8.),
                    });
                    cx.notify();
                }),
            )
            .into_any_element()
    }
}

/// A block entering (`popin`): its glow, then its card in a clip at r (window
/// points) that grows, the part not yet printed covered in blank, the scan
/// line on that cover's edge, and the box's bright edge.
fn popin_layers(f: popin::Frame, r: (f32, f32, f32, f32), card: AnyElement, blank: Hsla, t: &Theme, s: f32) -> [AnyElement; 2] {
    let (x, y, w, h) = r;
    let at = || div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h));
    let layer = || div().absolute().left_0().top_0().size_full();
    let glow = at()
        .shadow(vec![gpui::BoxShadow {
            color: t.accent.opacity(0.6 * f.glow),
            offset: point(px(0.), px(0.)),
            blur_radius: px(22. * s.min(1.5)),
            spread_radius: px(1.),
        }])
        .into_any_element();
    let line = (2. * s.min(1.5)).max(1.);
    let scan_y = h * f.scan;
    let clip = at()
        .overflow_hidden()
        .occlude()
        .child(card)
        .when(f.scan < 1., |d| {
            d.child(div().absolute().left_0().right_0().top(px(scan_y)).bottom_0().bg(blank)).child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px((scan_y - line / 2.).max(0.)))
                    .h(px(line))
                    .bg(t.accent.opacity(f.glow.max(0.3)))
                    .when(f.height < 1., |d| d.opacity(0.)),
            )
        })
        .child(layer().bg(t.accent.opacity(f.tint)))
        .child(layer().border_1().border_color(t.accent.opacity(f.glow)))
        .into_any_element();
    [glow, clip]
}

/// Whether a block's body is text laid out at the content scale, held still
/// while zooming. Pictures (an image, a PDF page, a page's snapshot, a video)
/// and a stack's fanned cards are drawn to the card's size instead, stretching
/// smoothly with it.
fn holds_layout(b: &Block) -> bool {
    match b.kind {
        BlockKind::Browser | BlockKind::Stack => false,
        BlockKind::File => !matches!(crate::preview::of(&b.path), crate::preview::Kind::Image | crate::preview::Kind::Pdf | crate::preview::Kind::Video),
        _ => true,
    }
}
