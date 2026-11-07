//! A strip of washi tape sticking out above a card's top edge, as if it held
//! the card down: the home scene's cards wear it (deck blocks no longer do).

use gpui::{Bounds, Hsla, PathBuilder, Pixels, Window, point, px};

use crate::theme::rgba_u8;

#[derive(Clone, Copy, Debug)]
pub struct Tape {
    /// Its middle, in window pixels.
    pub cx: f32,
    pub cy: f32,
    /// Its length and thickness.
    pub l: f32,
    pub t: f32,
    pub sin: f32,
    pub cos: f32,
}

/// FNV-1a of the ID, as the Go build hashes it, so a block's tape tilts the same way in both.
fn fnv32a(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

/// The tape of a block at (sx, sy), w wide on the screen, at scale s:
/// centered across its top edge, tilted -4°…4° by its ID.
pub fn tape_of(id: &str, sx: f32, sy: f32, w: f32, s: f32) -> Tape {
    let l = (96. * s).min(w * 0.6);
    let th = 30. * s;
    let deg = (fnv32a(id) % 9) as f64 - 4.;
    let a = deg.to_radians();
    Tape { cx: sx + w / 2., cy: sy - th * 0.3, l, t: th, sin: a.sin() as f32, cos: a.cos() as f32 }
}

impl Tape {
    /// The point (u, v) of the tape's own axes in window pixels.
    fn at(&self, u: f32, v: f32) -> (f32, f32) {
        (self.cx + u * self.cos - v * self.sin, self.cy + u * self.sin + v * self.cos)
    }

    /// Its bounds: x, y, w, h.
    #[cfg(test)]
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (a, b) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
            let (x, y) = self.at(a * self.l / 2., b * self.t / 2.);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        (x0, y0, x1 - x0, y1 - y0)
    }

    /// Its outline, ends torn in a zigzag, moved by (dx, dy).
    fn outline(&self, dx: f32, dy: f32, stroke: Option<f32>) -> Option<gpui::Path<Pixels>> {
        const TEETH: i32 = 5;
        let mut p = match stroke {
            Some(w) => PathBuilder::stroke(px(w)),
            None => PathBuilder::fill(),
        };
        let (hl, ht) = (self.l / 2., self.t / 2.);
        let tooth = self.t * 0.09;
        let mut first = true;
        let mut go = |u: f32, v: f32, p: &mut PathBuilder| {
            let (x, y) = self.at(u, v);
            let pt = point(px(x + dx), px(y + dy));
            if first {
                p.move_to(pt);
                first = false;
            } else {
                p.line_to(pt);
            }
        };
        go(-hl, -ht, &mut p);
        go(hl, -ht, &mut p);
        for i in 1..=TEETH * 2 {
            let u = if i % 2 == 1 { hl - tooth } else { hl };
            go(u, -ht + self.t * i as f32 / (TEETH * 2) as f32, &mut p);
        }
        go(-hl, ht, &mut p);
        for i in 1..TEETH * 2 {
            let u = if i % 2 == 1 { -hl + tooth } else { -hl };
            go(u, ht - self.t * i as f32 / (TEETH * 2) as f32, &mut p);
        }
        p.close();
        p.build().ok()
    }
}

#[derive(Clone, Copy)]
pub struct Look {
    pub hovered: bool,
    pub held: bool,
    pub selected: bool,
    pub dark: bool,
    pub accent: Hsla,
}

/// Paints the tape, moved by (dx, dy): clear, frosted white, its edges
/// catching the light, a faint shadow that lifts as it is held. A selected
/// block's tape takes the accent.
pub fn paint(window: &mut Window, tp: &Tape, dx: f32, dy: f32, look: Look) {
    let lift = tp.t * if look.held { 0.12 } else { 0.04 };
    let shadow = if look.dark { rgba_u8(0, 0, 0, 0.3) } else { rgba_u8(0, 0, 0, 0.07) };
    if let Some(p) = tp.outline(dx + lift * 0.4, dy + lift, None) {
        window.paint_path(p, shadow);
    }
    let (mut alpha, mut edge, shine) = if look.dark {
        (0.2, rgba_u8(255, 255, 255, 0.28), 0.3)
    } else {
        (0.62, rgba_u8(0, 0, 0, 0.16), 0.75)
    };
    if look.hovered {
        alpha += 0.12;
    }
    let mut paper = rgba_u8(255, 255, 255, alpha);
    let mut width = (tp.t * 0.03).max(0.75);
    if look.selected {
        let mut a = look.accent;
        a.l = a.l + (1. - a.l) * 0.82;
        a.a = (alpha + 0.1).min(1.);
        paper = a;
        edge = look.accent;
        width = (tp.t * 0.06).max(1.5);
    }
    if let Some(p) = tp.outline(dx, dy, None) {
        window.paint_path(p, paper);
    }
    if let Some(p) = tp.outline(dx, dy, Some(width)) {
        window.paint_path(p, edge);
    }
    let (gx, gy) = tp.at(0., -tp.t * 0.28);
    let gloss = Tape { cx: gx, cy: gy, l: tp.l * 0.9, t: tp.t * 0.1, ..*tp };
    if let Some(p) = gloss.outline(dx, dy, None) {
        window.paint_path(p, rgba_u8(255, 255, 255, shine));
    }
}

/// Three diagonal strokes in the corner, as a window's resize grip.
pub fn paint_grip(window: &mut Window, r: Bounds<Pixels>, color: Hsla) {
    let (x, y) = (f32::from(r.origin.x), f32::from(r.origin.y));
    let (w, h) = (f32::from(r.size.width), f32::from(r.size.height));
    let (x1, y1) = (x + w - w * 0.2, y + h - h * 0.2);
    let sw = (w * 0.07).max(1.);
    for i in 1..=3 {
        let f = i as f32 * w * 0.22;
        let mut p = PathBuilder::stroke(px(sw));
        p.move_to(point(px(x1 - f), px(y1)));
        p.line_to(point(px(x1), px(y1 - f)));
        if let Ok(p) = p.build() {
            window.paint_path(p, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tape_tilt_follows_the_id_and_stays_small() {
        let a = tape_of("b123abc", 0., 0., 400., 1.);
        let b = tape_of("b123abc", 50., 50., 400., 1.);
        assert_eq!(a.sin, b.sin);
        assert!(a.sin.abs() <= (4f32).to_radians().sin() + 1e-6);
        let (_, y, _, h) = a.bounds();
        assert!(y < 0. && y + h > 0., "it sticks out above the block");
    }

    #[test]
    fn fnv_matches_go() {
        // hash/fnv New32a of "" and "a".
        assert_eq!(fnv32a(""), 0x811c9dc5);
        assert_eq!(fnv32a("a"), 0xe40c292c);
    }
}
