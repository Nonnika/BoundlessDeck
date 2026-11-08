//! The panel on the right that styles 文字 and 图形: it shows while one or
//! more of them are selected (or the one worked in), and what is set in it
//! goes to all of them. Fonts and custom colors are picked in the system's
//! own panels (`nativepanel`), whose picks come back through `panel_tx`.

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, canvas, div, prelude::FluentBuilder, px, svg,
};

use super::DeckView;
use super::styled::{self, Shape};
use crate::icons;
use crate::model::{Block, BlockKind};
use crate::nativepanel::{self, Pick};
use crate::theme::{INKS, Paint, Theme, hex, paint};

/// The panel's width, and where it sits under the toolbar.
pub(super) const PANEL_W: f32 = 248.;
pub(super) const PANEL_TOP: f32 = 64.;

/// Which color a custom pick goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    Fill,
    Stroke,
    Color,
}

const RADII: &[f32] = &[0., 4., 8., 16., 32.];
const WEIGHTS: &[(u16, &str)] = &[(400, "常规"), (500, "中等"), (700, "粗体")];

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

    /// A pick from a system panel: to the blocks the panel styles.
    pub(super) fn on_pick(&mut self, pick: Pick, cx: &mut Context<Self>) {
        match pick {
            Pick::Font { family, size, weight, italic } => self.restyle(
                move |b| {
                    b.style.font = family.clone();
                    b.style.size = size.round().clamp(6., 400.);
                    b.style.weight = if weight == 400 { 0 } else { weight };
                    b.style.italic = italic;
                },
                cx,
            ),
            Pick::Color(c) => {
                let slot = self.color_slot;
                self.restyle(move |b| *slot_of(b, slot) = c.clone(), cx);
            }
        }
    }

    /// The panel, when there is something for it to style.
    pub(super) fn inspector(&mut self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let targets = self.style_targets();
        let first = self.m.block(targets.first()?)?.clone();
        let look = styled::look(&first, t);
        let any_shape = targets.iter().any(|id| self.m.block(id).is_some_and(|b| b.kind == BlockKind::Shape));
        let title = if targets.len() > 1 { format!("{} 个对象", targets.len()) } else { first.kind.label().to_string() };

        let mut panel = div()
            .id("inspector")
            .absolute()
            .top(px(PANEL_TOP))
            .right(px(12.))
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
                offset: gpui::point(px(0.), px(6.)),
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
            let buttons = [("rect", Shape::Rect), ("ellipse", Shape::Ellipse), ("diamond", Shape::Diamond), ("triangle", Shape::Triangle)]
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

        // The font: its family in the system's Fonts panel; size, weight and italic here too.
        let family = if first.style.font.is_empty() { "系统字体".to_string() } else { first.style.font.clone() };
        let size = look.size;
        let font_button = div()
            .id("font-panel")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .h(px(30.))
            .rounded(px(7.))
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .cursor_pointer()
            .hover(|d| d.bg(t.surface_hover))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_size(px(12.)).text_color(t.text).child(family))
            .child(div().text_size(px(11.)).text_color(t.muted).child("更改…"))
            .on_click(cx.listener(move |d, _, _, _| {
                let Some(b) = d.style_targets().first().and_then(|id| d.m.block(id)).cloned() else { return };
                nativepanel::font_panel(&b.style.font, size, if b.style.weight == 0 { 400 } else { b.style.weight }, b.style.italic, &d.panel_tx);
            }));
        let step = |id: &'static str, label: &'static str, by: f32| {
            toggle(id.into(), false, t).w(px(30.)).h(px(28.)).text_size(px(14.)).text_color(t.text).child(label).on_click(cx.listener(
                move |d, _, _, cx| {
                    d.restyle(move |b| b.style.size = (size + by).clamp(6., 400.), cx);
                },
            ))
        };
        let sizes = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(step("size-down", "−", -2.))
            .child(div().flex_1().text_center().text_size(px(12.)).text_color(t.text).child(format!("{} pt", size.round())))
            .child(step("size-up", "+", 2.));
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
                .child(sizes)
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

        // Corners: for rectangles and text.
        let cornered = targets.iter().any(|id| {
            self.m.block(id).is_some_and(|b| b.kind == BlockKind::Text || Shape::of(&b.shape) == Shape::Rect)
        });
        if cornered {
            let radii = div().flex().flex_row().gap(px(4.)).children(RADII.iter().map(|&r| {
                let on = (look.radius - r).abs() < 0.5;
                toggle(SharedString::from(format!("radius-{r}")), on, t)
                    .flex_1()
                    .h(px(28.))
                    .text_size(px(12.))
                    .child(format!("{r}"))
                    .on_click(cx.listener(move |d, _, _, cx| d.restyle(move |b| b.style.radius = Some(r), cx)))
            }));
            panel = panel.child(section("圆角", t).child(radii));
        }
        Some(panel.into_any_element())
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
                            p.move_to(gpui::point(b.origin.x + px(2.), b.origin.y + b.size.height - px(2.)));
                            p.line_to(gpui::point(b.origin.x + b.size.width - px(2.), b.origin.y + px(2.)));
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
        // Custom: the Colors panel, on the color in use.
        let custom = spec.starts_with('#');
        let shown: Hsla = match paint(spec, false, t) {
            Paint::Color(c) => c,
            _ => t.accent,
        };
        row.child(
            swatch(SharedString::from(format!("{slot:?}-custom")), custom)
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
                .on_click(cx.listener(move |d, _, _, _| {
                    d.color_slot = slot;
                    nativepanel::color_panel(&hex(shown), &d.panel_tx);
                })),
        )
    }
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
