//! 文字 and 图形: blocks drawn as they are, without a card. Their `style`
//! says the colors, the font, the corners and where the words sit; what it
//! leaves out takes the kind's default (`look`). A 图形 is a rectangle, an
//! ellipse, a diamond, a triangle or an arrow, with its words inside the part
//! of it they fit in. A 文字's words are Markdown, set in its style: headings,
//! lists and the rest take its font and color, and each line typed is a line.

use gpui::{
    AnyElement, Bounds, Entity, FontWeight, Hsla, IntoElement, ParentElement, PathBuilder, Pixels,
    SharedString, Styled, Window, canvas, div, fill, point, prelude::FluentBuilder, px, size,
};

use crate::ui::editor::Editor;
use crate::content::markdown::{self, Parsed};
use crate::model::{Block, BlockKind};
use crate::ui::theme::{MdColors, Paint, Theme, paint};

/// A block's style, worked out: what to draw.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Look {
    pub fill: Option<Hsla>,
    pub stroke: Option<Hsla>,
    pub color: Hsla,
    pub family: Option<SharedString>,
    /// Points at 100%.
    pub size: f32,
    pub weight: FontWeight,
    pub italic: bool,
    /// Corner radii: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
    /// Across, 0 left to 1 right; up and down, 0 top to 1 bottom.
    pub align: f32,
    pub valign: f32,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    Rect,
    Ellipse,
    Diamond,
    Triangle,
    /// A broad arrow pointing right, its words in the shaft.
    Arrow,
}

impl Shape {
    pub fn of(name: &str) -> Shape {
        match name {
            "ellipse" => Shape::Ellipse,
            "diamond" => Shape::Diamond,
            "triangle" => Shape::Triangle,
            "arrow" => Shape::Arrow,
            _ => Shape::Rect,
        }
    }
}

/// The defaults: words plain on the canvas for 文字; a soft blue rounded box
/// with its words in the middle for 图形.
pub(super) fn look(b: &Block, t: &Theme) -> Look {
    let st = &b.style;
    let shape_kind = b.kind == BlockKind::Shape;
    let pick = |spec: &str, soft: bool, default: Option<Hsla>| match paint(spec, soft, t) {
        Paint::Default => default,
        Paint::None => None,
        Paint::Color(c) => Some(c),
    };
    let (fill_d, stroke_d) = if shape_kind {
        (pick("blue", true, None), pick("blue", false, None))
    } else {
        (None, None)
    };
    let fill = pick(&st.fill, true, fill_d);
    let stroke = pick(&st.stroke, false, stroke_d);
    let color = pick(&st.color, false, Some(t.text)).unwrap_or(t.text);
    let size = font_size(b);
    let weight = if st.weight > 0 { FontWeight(st.weight.clamp(100, 900) as f32) } else { FontWeight::NORMAL };
    let along = |v: &str, default: f32| match v {
        "left" | "top" => 0.,
        "center" | "middle" => 0.5,
        "right" | "bottom" => 1.,
        _ => default,
    };
    Look {
        fill,
        stroke,
        color,
        family: (!st.font.is_empty()).then(|| SharedString::from(st.font.clone())),
        size,
        weight,
        italic: st.italic,
        radii: radii(b),
        align: along(&st.align, if shape_kind { 0.5 } else { 0. }),
        valign: along(&st.valign, if shape_kind { 0.5 } else { 0. }),
        shape: if shape_kind { Shape::of(&b.shape) } else { Shape::Rect },
    }
}

/// The words' size in points at 100%: the style's, else the kind's default.
pub(super) fn font_size(b: &Block) -> f32 {
    if b.style.size > 0. {
        b.style.size.clamp(6., 400.)
    } else if b.kind == BlockKind::Shape {
        16.
    } else {
        24.
    }
}

/// The radius on every corner unless they are set one by one: the kind's
/// default, else what the style says.
pub(super) fn radius(b: &Block) -> f32 {
    b.style.radius.unwrap_or(if b.kind == BlockKind::Shape { 12. } else { 8. }).max(0.)
}

/// Each corner's radius: top-left, top-right, bottom-right, bottom-left.
pub(super) fn radii(b: &Block) -> [f32; 4] {
    b.style.corners.map(|c| c.map(|r| r.max(0.))).unwrap_or([radius(b); 4])
}

/// The colors a 文字's Markdown is set in: its words' color, fainter for
/// quotes, rules and code, and links in the accent.
pub(super) fn md_colors(l: &Look, t: &Theme) -> MdColors {
    MdColors { text: l.color, muted: l.color.opacity(0.6), link: t.accent, code_bg: l.color.opacity(0.08), rule: l.color.opacity(0.25) }
}

/// An arrow's head: how far it reaches back from the tip, in a w by h box.
fn arrow_head(w: f32, h: f32) -> f32 {
    (h * 0.6).min(w * 0.5)
}

/// Sets an editor up to look as the block's words do.
pub(super) fn dress(e: &mut Editor, l: &Look, scale: f32) {
    e.font_size = l.size * scale;
    e.color = l.color;
    e.family = l.family.clone();
    e.weight = Some(l.weight);
    e.italic = l.italic;
    e.align = l.align;
    e.valign = l.valign;
}

/// The part of a w by h shape the words go in: left, top, right, bottom insets.
fn text_inset(shape: Shape, w: f32, h: f32, pad: f32) -> [f32; 4] {
    match shape {
        Shape::Rect => [pad; 4],
        Shape::Ellipse => [w * 0.15 + pad / 2., h * 0.15 + pad / 2., w * 0.15 + pad / 2., h * 0.15 + pad / 2.],
        Shape::Diamond => [w * 0.25, h * 0.25, w * 0.25, h * 0.25],
        Shape::Triangle => [w * 0.25, h * 0.45, w * 0.25, pad],
        Shape::Arrow => [pad, h * 0.28, arrow_head(w, h) * 0.6 + pad, h * 0.28],
    }
}

/// The block's body, w by h on screen, its words at `scale` (the content
/// scale); being edited, `editor` takes the words' place. With `md`, the
/// words are that Markdown in those colors. Far out (`lod`) only the shape is drawn.
#[allow(clippy::too_many_arguments)]
pub(super) fn view(
    b: &Block,
    l: &Look,
    w: f32,
    h: f32,
    scale: f32,
    editor: Option<Entity<Editor>>,
    md: Option<(Parsed, MdColors)>,
    lod: bool,
) -> AnyElement {
    let shape = l.shape;
    let (fill_c, stroke_c, radii) = (l.fill, l.stroke, l.radii.map(|r| r * scale));
    let stroke_w = (if b.kind == BlockKind::Shape { 2. } else { 1.5 }) * scale;
    let body = canvas(|_, _, _| (), move |bounds, _, window, _| paint_shape(window, bounds, shape, fill_c, stroke_c, stroke_w, radii))
        .absolute()
        .left_0()
        .top_0()
        .size_full();
    let mut out = div().relative().size_full().child(body);
    if lod {
        return out.into_any_element();
    }
    let [il, it, ir, ib] = text_inset(shape, w, h, 10. * scale);
    let area = div().absolute().left(px(il)).top(px(it)).right(px(ir)).bottom(px(ib));
    let words = match editor {
        Some(ed) => area.child(ed).into_any_element(),
        None => {
            let mut text = div()
                .w_full()
                .text_size(px(l.size * scale))
                .line_height(px(l.size * scale * 1.5))
                .font_weight(l.weight)
                .text_color(l.color)
                .when(l.italic, |d| d.italic())
                .when_some(l.family.clone(), |d, f| d.font_family(f));
            text = match &md {
                Some((doc, colors)) => text.child(markdown::render_parsed(doc, l.size * scale, colors)),
                None => text.child(b.text.clone()),
            };
            text = if l.align >= 0.75 {
                text.text_right()
            } else if l.align >= 0.25 {
                text.text_center()
            } else {
                text
            };
            let justify = |d: gpui::Div| {
                if l.valign >= 0.75 {
                    d.justify_end()
                } else if l.valign >= 0.25 {
                    d.justify_center()
                } else {
                    d.justify_start()
                }
            };
            justify(area.flex().flex_col().overflow_hidden()).child(text).into_any_element()
        }
    };
    out = out.child(words);
    out.into_any_element()
}

fn paint_shape(
    window: &mut Window,
    b: Bounds<Pixels>,
    shape: Shape,
    fill_c: Option<Hsla>,
    stroke_c: Option<Hsla>,
    stroke_w: f32,
    radii: [f32; 4],
) {
    let (x, y, w, h) = (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
    if shape == Shape::Rect {
        let lim = (w / 2.).min(h / 2.);
        let r = gpui::Corners {
            top_left: px(radii[0].min(lim)),
            top_right: px(radii[1].min(lim)),
            bottom_right: px(radii[2].min(lim)),
            bottom_left: px(radii[3].min(lim)),
        };
        match (fill_c, stroke_c) {
            (f, Some(s)) => window.paint_quad(gpui::quad(
                b,
                r,
                f.unwrap_or(gpui::transparent_black()),
                px(stroke_w),
                s,
                gpui::BorderStyle::Solid,
            )),
            (Some(f), None) => window.paint_quad(fill(b, f).corner_radii(r)),
            (None, None) => {}
        }
        return;
    }
    // The outline, inset by half the stroke so it stays inside the block.
    let k = stroke_c.map_or(0., |_| stroke_w / 2.);
    let (x0, y0, x1, y1) = (x + k, y + k, x + w - k, y + h - k);
    let (cx, cy) = ((x0 + x1) / 2., (y0 + y1) / 2.);
    let pts: Vec<(f32, f32)> = match shape {
        Shape::Diamond => vec![(cx, y0), (x1, cy), (cx, y1), (x0, cy)],
        Shape::Triangle => vec![(cx, y0), (x1, y1), (x0, y1)],
        Shape::Arrow => {
            let head = arrow_head(x1 - x0, y1 - y0);
            let (neck, half) = (x1 - head, (y1 - y0) * 0.22);
            vec![(x0, cy - half), (neck, cy - half), (neck, y0), (x1, cy), (neck, y1), (neck, cy + half), (x0, cy + half)]
        }
        _ => {
            let (rx, ry) = ((x1 - x0) / 2., (y1 - y0) / 2.);
            (0..96)
                .map(|i| {
                    let a = i as f32 / 96. * std::f32::consts::TAU;
                    (cx + rx * a.cos(), cy + ry * a.sin())
                })
                .collect()
        }
    };
    let path = |mut pb: PathBuilder| {
        pb.move_to(point(px(pts[0].0), px(pts[0].1)));
        for p in &pts[1..] {
            pb.line_to(point(px(p.0), px(p.1)));
        }
        pb.close();
        pb.build().ok()
    };
    if let Some(f) = fill_c
        && let Some(p) = path(PathBuilder::fill())
    {
        window.paint_path(p, f);
    }
    if let Some(s) = stroke_c
        && let Some(p) = path(PathBuilder::stroke(px(stroke_w)))
    {
        window.paint_path(p, s);
    }
}

/// A small picture of a shape, for the panel's shape buttons.
pub(super) fn paint_icon(window: &mut Window, b: Bounds<Pixels>, shape: Shape, color: Hsla) {
    let inset = 3.;
    let inner = Bounds::new(
        point(b.origin.x + px(inset), b.origin.y + px(inset)),
        size(b.size.width - px(2. * inset), b.size.height - px(2. * inset)),
    );
    paint_shape(window, inner, shape, None, Some(color), 1.5, [3.; 4]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Style;

    #[test]
    fn an_arrow_keeps_its_words_in_the_shaft() {
        assert_eq!(Shape::of("arrow"), Shape::Arrow);
        let [l, t, r, b] = text_inset(Shape::Arrow, 220., 100., 10.);
        assert!(l < r, "the head leaves less room on the right");
        assert!(t > 20. && b > 20. && 100. - t - b > 30., "the words sit in the shaft");
        assert!(arrow_head(40., 100.) <= 20., "a narrow arrow is not all head");
    }

    #[test]
    fn styles_fall_back_to_the_kind() {
        let t = Theme::light();
        let mut b = Block::new(BlockKind::Text);
        let l = look(&b, &t);
        assert_eq!((l.fill, l.stroke, l.size, l.align, l.valign), (None, None, 24., 0., 0.));
        b.kind = BlockKind::Shape;
        b.shape = "ellipse".into();
        let l = look(&b, &t);
        assert!(l.fill.is_some() && l.stroke.is_some());
        assert_eq!((l.shape, l.align, l.valign, l.radii), (Shape::Ellipse, 0.5, 0.5, [12.; 4]));
        b.style = Style {
            fill: "none".into(),
            color: "#ff0000".into(),
            size: 40.,
            weight: 700,
            radius: Some(0.),
            align: "right".into(),
            valign: "bottom".into(),
            font: "Kaiti SC".into(),
            ..Style::default()
        };
        let l = look(&b, &t);
        assert_eq!(l.fill, None);
        assert_eq!(crate::ui::theme::hex(l.color), "#ff0000");
        assert_eq!((l.size, l.weight, l.radii, l.align, l.valign), (40., FontWeight(700.), [0.; 4], 1., 1.));
        b.style.corners = Some([0., 4., 16., -2.]);
        assert_eq!(look(&b, &t).radii, [0., 4., 16., 0.], "corners one by one, never below 0");
        assert_eq!(l.family.as_ref().map(|f| f.as_ref()), Some("Kaiti SC"));
    }
}
