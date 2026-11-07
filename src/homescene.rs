//! The home screen's background: one dotted canvas plane leaning back,
//! running off into the distance, a few blocks lying on it. GPUI has no 3D
//! transform, so the plane is projected here, point by point, as the Go
//! build's CSS-like transform has it (leaning back planeLean° about its
//! near edge, seen from planeDepth window-heights); near blocks fade where
//! the Go build blurs them. Also the marks the home screen draws: the
//! robot of the app icon, the deck mark of a recent folder.

use std::f32::consts::PI;

use gpui::{Bounds, Hsla, PathBuilder, Pixels, Window, fill, point, px, size};

use crate::tape::{self, Tape};
use crate::theme::{Paper, Theme, rgba_u8};

/// Below this width the plane holds no blocks: they would crowd the column.
const SCENE_MIN_WIDTH: f32 = 960.;
const PLANE_LEAN: f32 = 40.;
const PLANE_DEPTH: f32 = 1.8;
const PLANE_NEAR: f32 = 80.;
const PLANE_FAR_Y: f32 = 30.;
const PLANE_WIDTH: f32 = 2.4;
/// How far up the plane blocks stop fading.
const SCENE_FOCUS: f32 = 1300.;

#[derive(Clone, Copy)]
enum Kind {
    Note,
    File,
    Terminal,
    Image,
}

/// Where the blocks lie: u across from the middle, v up from the near edge,
/// in plane units; their size and their turn on the plane.
const BLOCKS: [(Kind, f32, f32, f32, f32, f32); 10] = [
    (Kind::Terminal, -600., 40., 400., 250., -4.),
    (Kind::Note, 560., 110., 250., 220., 6.),
    (Kind::Image, -700., 520., 270., 210., 3.),
    (Kind::File, 700., 580., 290., 220., -3.),
    (Kind::Note, -700., 1000., 210., 190., -5.),
    (Kind::Terminal, 720., 1080., 320., 200., 2.),
    (Kind::File, -820., 1560., 260., 200., 4.),
    (Kind::Note, 820., 1650., 200., 180., -4.),
    (Kind::Image, -330., 1820., 230., 170., -2.),
    (Kind::Note, 380., 1860., 200., 170., 5.),
];

pub fn void_color(t: &Theme) -> Hsla {
    if t.dark { gpui::rgb(0x101114).into() } else { gpui::rgb(0xeceef2).into() }
}

/// The plane as the window shows it.
struct Plane {
    /// Window center x, the near edge's y.
    cx: f32,
    oy: f32,
    /// The perspective distance.
    d: f32,
    sin: f32,
    cos: f32,
    /// Its width and depth in plane units.
    w: f32,
    depth: f32,
}

impl Plane {
    fn new(cw: f32, ch: f32) -> Plane {
        let (sin, cos) = (PLANE_LEAN * PI / 180.).sin_cos();
        let d = PLANE_DEPTH * ch;
        let oy = ch + PLANE_NEAR;
        let rise = oy - PLANE_FAR_Y;
        let mut depth = rise / (cos - rise * sin / d);
        if depth <= 0. || depth > 8. * ch {
            depth = 8. * ch;
        }
        Plane { cx: cw / 2., oy, d, sin, cos, w: PLANE_WIDTH * cw, depth }
    }

    /// Where the plane's point (u, v) shows, and how much it is scaled there.
    fn at(&self, u: f32, v: f32) -> (f32, f32, f32) {
        let k = self.d / (self.d + v * self.sin);
        (self.cx + u * k, self.oy - v * self.cos * k, k)
    }
}

/// A block's local point (x, y from its top-left) on the plane, turned by deg.
fn on_plane(pl: &Plane, b: (f32, f32, f32, f32, f32), x: f32, y: f32) -> (f32, f32) {
    let (u0, v0, w, h, deg) = b;
    let (s, c) = (deg * PI / 180.).sin_cos();
    // About its middle; y runs down the block, v up the plane.
    let (dx, dy) = (x - w / 2., y - h / 2.);
    let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
    let (sx, sy, _) = pl.at(u0 + rx, v0 + h / 2. - ry);
    (sx, sy)
}

fn poly(window: &mut Window, pts: &[(f32, f32)], color: Hsla) {
    if pts.len() < 3 {
        return;
    }
    let mut b = PathBuilder::fill();
    b.move_to(point(px(pts[0].0), px(pts[0].1)));
    for p in &pts[1..] {
        b.line_to(point(px(p.0), px(p.1)));
    }
    b.close();
    if let Ok(p) = b.build() {
        window.paint_path(p, color);
    }
}

/// A rectangle of a block, in its local coordinates, projected.
fn block_rect(window: &mut Window, pl: &Plane, b: (f32, f32, f32, f32, f32), r: (f32, f32, f32, f32), color: Hsla) {
    let (x, y, w, h) = r;
    let pts = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)].map(|(px_, py_)| on_plane(pl, b, px_, py_));
    poly(window, &pts, color);
}

pub fn paint(window: &mut Window, bounds: Bounds<Pixels>, t: &Theme, paper: &Paper) {
    let (cw, ch) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let pl = Plane::new(cw, ch);
    window.paint_quad(fill(bounds, void_color(t)));
    // The plane itself, and its dots, 24 apart, shrinking with distance.
    let corners = [(-pl.w / 2., 0.), (pl.w / 2., 0.), (pl.w / 2., pl.depth), (-pl.w / 2., pl.depth)]
        .map(|(u, v)| {
            let (x, y, _) = pl.at(u, v);
            (x, y)
        });
    poly(window, &corners, t.background);
    let dot = t.muted.opacity(0.4);
    let mut v = 12.;
    while v < pl.depth {
        let (_, y, k) = pl.at(0., v);
        let r = 1.5 * k;
        if r < 0.35 || y < 0. {
            break;
        }
        if y - r < ch {
            // Only the dots the window shows.
            let half = (cw / 2. / k).min(pl.w / 2.);
            let mut u = -(half / 24.).floor() * 24. - 12.;
            while u <= half {
                let (x, _, _) = pl.at(u, v);
                window.paint_quad(
                    fill(Bounds::new(point(px(x - r), px(y - r)), size(px(2. * r), px(2. * r))), dot).corner_radii(px(r)),
                );
                u += 24.;
            }
        }
        v += 24.;
    }
    // The blocks lying on it, the far ones first.
    if cw >= SCENE_MIN_WIDTH {
        for &(kind, u, v, w, h, deg) in BLOCKS.iter().rev() {
            let b = (u, v, w, h, deg);
            let near = if v >= SCENE_FOCUS { 0. } else { (1. - v / SCENE_FOCUS).powi(2) };
            let a = 1. - 0.55 * near;
            scene_block(window, &pl, b, kind, t, paper, a);
        }
    }
    // Mist where the plane meets the void, the void above it.
    let void = void_color(t);
    window.paint_quad(fill(Bounds::new(bounds.origin, size(bounds.size.width, px(PLANE_FAR_Y + 2.))), void));
    for i in 0..14 {
        let f = i as f32 / 14.;
        window.paint_quad(fill(
            Bounds::new(point(bounds.origin.x, px(PLANE_FAR_Y + 5. * i as f32)), size(bounds.size.width, px(5.))),
            void.opacity(1. - f),
        ));
    }
}

fn scene_block(window: &mut Window, pl: &Plane, b: (f32, f32, f32, f32, f32), kind: Kind, t: &Theme, paper: &Paper, a: f32) {
    let (w, h) = (b.2, b.3);
    let shade = if t.dark { rgba_u8(0, 0, 0, 0.45 * a) } else { rgba_u8(0, 0, 0, 0.1 * a) };
    block_rect(window, pl, b, (4., 6., w, h), shade);
    match kind {
        Kind::Note => {
            block_rect(window, pl, b, (0., 0., w, h), paper.top.opacity(a));
            block_rect(window, pl, b, (0., h * 0.6, w, h * 0.4), paper.bottom.opacity(a * 0.6));
            block_rect(window, pl, b, (16., 18., 60., 12.), paper.text.opacity(0.7 * a));
            for i in 0..3 {
                block_rect(window, pl, b, (16., 54. + i as f32 * 24., w * 0.55 - i as f32 * 14., 7.), paper.muted.opacity(0.6 * a));
            }
            block_rect(window, pl, b, (w / 2. - 37., -8., 74., 22.), rgba_u8(255, 255, 255, if t.dark { 0.25 } else { 0.62 } * a));
        }
        _ => {
            block_rect(window, pl, b, (-1., -1., w + 2., h + 2.), t.border.opacity(a));
            block_rect(window, pl, b, (0., 0., w, h), t.surface.opacity(a));
            block_rect(window, pl, b, (12., 10., 70., 10.), t.text.opacity(0.6 * a));
            block_rect(window, pl, b, (0., 30., w, 1.), t.border.opacity(a));
            block_rect(window, pl, b, (w / 2. - 48., -9., 96., 21.), rgba_u8(255, 255, 255, if t.dark { 0.2 } else { 0.62 } * a));
            match kind {
                Kind::Terminal => {
                    block_rect(window, pl, b, (1., 31., w - 2., h - 32.), rgba_u8(0x1d, 0x1f, 0x24, a));
                    let green = rgba_u8(0x7e, 0xe7, 0x87, a);
                    let dim = rgba_u8(0x8b, 0x91, 0x9a, a);
                    block_rect(window, pl, b, (14., 46., 48., 8.), dim);
                    block_rect(window, pl, b, (14., 68., 8., 8.), green);
                    block_rect(window, pl, b, (28., 68., 120., 8.), rgba_u8(0xe6, 0xe6, 0xe6, a));
                    block_rect(window, pl, b, (14., 90., 140., 8.), dim);
                    block_rect(window, pl, b, (14., 112., 8., 8.), green);
                    block_rect(window, pl, b, (28., 110., 7., 14.), rgba_u8(0xe6, 0xe6, 0xe6, 0.8 * a));
                }
                Kind::Image => {
                    block_rect(window, pl, b, (1., 31., w - 2., h - 32.), rgba_u8(0xc4, 0xdb, 0xff, a));
                    block_rect(window, pl, b, (w - 56., 49., 26., 26.), rgba_u8(0xff, 0xd1, 0x66, a));
                    block_rect(window, pl, b, (1., h - 46., w * 0.8, 45.), rgba_u8(0x6f, 0xbf, 0x73, a));
                    block_rect(window, pl, b, (w * 0.4, h - 34., w * 0.6 - 1., 33.), rgba_u8(0x4f, 0xa6, 0x5a, a));
                }
                _ => {
                    block_rect(window, pl, b, (14., 46., 120., 10.), t.text.opacity(0.7 * a));
                    for (i, f) in [0.82, 0.64, 0.74, 0.4].iter().enumerate() {
                        block_rect(window, pl, b, (14., 78. + i as f32 * 16., (w - 28.) * f, 6.), t.muted.opacity(0.25 * a));
                    }
                }
            }
        }
    }
}

/// A rounded rectangle's outline, turned by deg about its middle.
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32, deg: f32) -> Vec<(f32, f32)> {
    let r = r.min(w / 2.).min(h / 2.);
    let (cx, cy) = (x + w / 2., y + h / 2.);
    let (s, c) = (deg * PI / 180.).sin_cos();
    let mut pts = Vec::new();
    for (ox, oy, a0) in [(x + w - r, y + r, -PI / 2.), (x + w - r, y + h - r, 0.), (x + r, y + h - r, PI / 2.), (x + r, y + r, PI)] {
        for i in 0..=6 {
            let a = a0 + PI / 2. * i as f32 / 6.;
            let (px_, py_) = (ox + r * a.cos() - cx, oy + r * a.sin() - cy);
            pts.push((cx + px_ * c - py_ * s, cy + px_ * s + py_ * c));
        }
    }
    pts
}

/// The figure of the app icon: a white block with the robot's face, two
/// blocks of the deck fanned out behind it, in a square of `r`.
pub fn paint_robot(window: &mut Window, r: Bounds<Pixels>, t: &Theme) {
    let (x0, y0) = (f32::from(r.origin.x), f32::from(r.origin.y));
    let k = f32::from(r.size.width) / 480.;
    let at = |x: f32, y: f32, w: f32, h: f32| (x0 + x * k, y0 + y * k, w * k, h * k);
    let back: Hsla = if t.dark { t.accent } else { gpui::rgb(0x5a86fa).into() };
    let rad = 88. * k;
    let (x, y, w, h) = at(64., 50., 400., 400.);
    poly(window, &rounded(x, y, w, h, rad, 16.), back.opacity(0.28));
    let (x, y, w, h) = at(44., 60., 400., 400.);
    poly(window, &rounded(x, y, w, h, rad, 8.), back.opacity(0.5));
    let (x, y, w, h) = at(24., 72., 400., 400.);
    let shadow = if t.dark { rgba_u8(0, 0, 0, 0.45) } else { rgba_u8(0x10, 0x24, 0x5e, 0.18) };
    poly(window, &rounded(x, y + 16. * k, w, h, rad, 0.), shadow);
    poly(window, &rounded(x, y, w, h, rad, 0.), gpui::white());
    let (x, y, w, h) = at(84., 202., 280., 168.);
    poly(window, &rounded(x, y, w, h, 84. * k, 0.), gpui::rgb(0x202129).into());
    for ex in [165., 249.] {
        let (x, y, w, h) = at(ex, 247., 34., 78.);
        poly(window, &rounded(x, y, w, h, 17. * k, 0.), gpui::white());
    }
}

/// A recent folder's mark: a QuickLook sheet over two blocks fanned out.
pub fn paint_deck_mark(window: &mut Window, r: Bounds<Pixels>, t: &Theme, paper: &Paper, faint: bool) {
    let (x0, y0) = (f32::from(r.origin.x), f32::from(r.origin.y));
    let u = f32::from(r.size.width) / 34.;
    let a = if faint { 0.4 } else { 1. };
    let back: Hsla = if t.dark { t.surface } else { gpui::white() };
    let (sx, sy, sw, sh) = (x0 + 7. * u, y0 + 8. * u, 21. * u, 20. * u);
    for (i, deg) in [-16f32, -8.].into_iter().enumerate() {
        let off = (2 - i) as f32;
        let (x, y) = (sx - off * 2.5 * u, sy + off * 0.8 * u);
        poly(window, &rounded(x - 0.5 * u, y - 0.5 * u, sw + u, sh + u, 0.5 * u, deg), t.muted.opacity(0.35 * a));
        poly(window, &rounded(x, y, sw, sh, 0., deg), back.opacity(a));
    }
    poly(window, &rounded(sx, sy + u, sw, sh, 0., 4.), paper.shadow.opacity(a));
    poly(window, &rounded(sx, sy, sw, sh, 0., 4.), paper.top.opacity(a));
    let tp = Tape { cx: sx + sw / 2., cy: sy + u, l: 14. * u, t: 6. * u, sin: -0.07, cos: 0.9975 };
    tape::paint(window, &tp, 0., 0., tape::Look { hovered: false, held: false, selected: false, dark: t.dark, accent: t.accent });
}

/// The words that take turns before "无止境", how long one stays and how
/// long the next takes to slide in.
pub const ENDLESS_WORDS: [&str; 3] = ["创意", "创造", "创作"];
pub const ENDLESS_HOLD_MS: u64 = 2200;
pub const ENDLESS_SLIDE_MS: u64 = 450;

/// Which word shows at `ms` and how far the next has slid in (0 while none slides).
pub fn endless_at(ms: u64) -> (usize, f32) {
    let period = ENDLESS_HOLD_MS + ENDLESS_SLIDE_MS;
    let word = ((ms / period) % ENDLESS_WORDS.len() as u64) as usize;
    let into = ms % period;
    if into < ENDLESS_HOLD_MS { (word, 0.) } else { (word, (into - ENDLESS_HOLD_MS) as f32 / ENDLESS_SLIDE_MS as f32) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plane_runs_from_below_the_window_to_its_far_edge() {
        let pl = Plane::new(1280., 800.);
        let (_, near, k0) = pl.at(0., 0.);
        assert_eq!(near, 880.);
        assert_eq!(k0, 1.);
        let (_, far, k) = pl.at(0., pl.depth);
        assert!((far - PLANE_FAR_Y).abs() < 0.5, "far edge at {far}");
        assert!(k < 0.6, "it shrinks with distance");
    }

    #[test]
    fn words_take_turns() {
        assert_eq!(endless_at(0), (0, 0.));
        assert_eq!(endless_at(ENDLESS_HOLD_MS + ENDLESS_SLIDE_MS / 2), (0, 0.5));
        assert_eq!(endless_at(ENDLESS_HOLD_MS + ENDLESS_SLIDE_MS).0, 1);
    }
}
