//! A 图表 block's drawing: bars, lying bars, a line, an area or a pie, in
//! categorical hues in a fixed order; round axis steps (万/亿), a legend
//! for two series or more, and the category or slice hovered in a tooltip.

use std::f64::consts::PI;

use gpui::{
    AnyElement, App, Bounds, FontWeight, Hsla, InteractiveElement, StatefulInteractiveElement, IntoElement, ParentElement, PathBuilder, Pixels,
    SharedString, Styled, TextRun, Window, canvas, div, fill, point, px, size,
};

use crate::chart::{self, Data, Type, MAX_SERIES};
use crate::theme::{Theme, rgba_u8};

fn hue(i: usize, dark: bool) -> Hsla {
    const LIGHT: [u32; 8] = [0x2a78d6, 0xeb6834, 0x1baf7a, 0xeda100, 0xe87ba4, 0x008300, 0x6250d6, 0xe34948];
    const DARK: [u32; 8] = [0x3987e5, 0xd95926, 0x199e70, 0xc98500, 0xd55181, 0x008300, 0x9085e9, 0xe66767];
    let c = if dark { DARK[i % 8] } else { LIGHT[i % 8] };
    gpui::rgb(c).into()
}

#[derive(Clone, Copy)]
struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// What the drawing paints with.
struct P<'a> {
    window: &'a mut Window,
    cx: &'a mut App,
    s: f32,
    t: Theme,
}

impl P<'_> {
    fn rect(&mut self, r: R, color: Hsla, radius: f32) {
        if r.w <= 0. || r.h <= 0. {
            return;
        }
        self.window.paint_quad(
            fill(Bounds::new(point(px(r.x), px(r.y)), size(px(r.w), px(r.h))), color).corner_radii(px(radius)),
        );
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Hsla) {
        let w = self.s.max(0.5);
        if x0 == x1 {
            self.rect(R { x: x0 - w / 2., y: y0.min(y1), w, h: (y1 - y0).abs() }, color, 0.);
        } else {
            self.rect(R { x: x0.min(x1), y: y0 - w / 2., w: (x1 - x0).abs(), h: w }, color, 0.);
        }
    }

    fn shape(&mut self, text: &str, size_: f32, color: Hsla, bold: bool) -> gpui::ShapedLine {
        let mut font = self.window.text_style().font();
        if bold {
            font.weight = FontWeight::SEMIBOLD;
        }
        let run = TextRun { len: text.len(), font, color, background_color: None, underline: None, strikethrough: None };
        self.window.text_system().shape_line(SharedString::from(text.to_string()), px(size_ * self.s), &[run], None)
    }

    fn measure(&mut self, text: &str, size_: f32) -> f32 {
        f32::from(self.shape(text, size_, gpui::black(), false).width)
    }

    /// Paints text with its top-left at (x, y), and says how wide it was.
    fn text(&mut self, x: f32, y: f32, text: &str, size_: f32, color: Hsla, bold: bool) -> f32 {
        let line = self.shape(text, size_, color, bold);
        let w = f32::from(line.width);
        let _ = line.paint(point(px(x), px(y)), px(size_ * self.s * 1.3), self.window, self.cx);
        w
    }

    fn path(&mut self, pts: &[(f32, f32)], color: Hsla) {
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
            self.window.paint_path(p, color);
        }
    }

    fn polyline(&mut self, pts: &[(f32, f32)], width: f32, color: Hsla) {
        let mut b = PathBuilder::stroke(px(width));
        b.move_to(point(px(pts[0].0), px(pts[0].1)));
        for p in &pts[1..] {
            b.line_to(point(px(p.0), px(p.1)));
        }
        if let Ok(p) = b.build() {
            self.window.paint_path(p, color);
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Hsla) {
        let pts: Vec<(f32, f32)> = (0..24)
            .map(|k| {
                let a = k as f32 / 24. * std::f32::consts::TAU;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.path(&pts, color);
    }
}

pub struct View<'a> {
    pub kind: Type,
    pub data: &'a Data,
    pub hover: Option<(f32, f32)>,
    /// Small, on a 块堆's card: marks alone.
    pub compact: bool,
}

impl View<'_> {
    fn draw(&self, p: &mut P, r: R) {
        let s = p.s;
        let t = p.t;
        if self.data.is_empty() {
            if !self.compact {
                let msg = "还没有数据 · 双击或点标题栏的表格按钮填写";
                let w = p.measure(msg, 12.);
                p.text(r.x + (r.w - w) / 2., r.y + r.h / 2. - 8. * s, msg, 12., t.muted, false);
            }
            return;
        }
        let pad = if self.compact { 4. * s } else { 12. * s };
        let inner = R { x: r.x + pad, y: r.y + pad, w: r.w - 2. * pad, h: r.h - 2. * pad };
        if inner.w <= 8. * s || inner.h <= 8. * s {
            return;
        }
        if self.kind == Type::Pie {
            self.pie(p, inner, r);
            return;
        }
        let mut top = inner.y;
        if self.data.series.len() > 1 && !self.compact {
            let keys: Vec<(Hsla, String)> =
                self.data.series.iter().enumerate().map(|(i, n)| (hue(i, t.dark), n.clone())).collect();
            let h = legend(p, R { x: inner.x, y: inner.y, w: inner.w, h: inner.h / 3. }, &keys, true);
            top += h + 8. * s;
        }
        let mut plot = R { x: inner.x, y: top, w: inner.w, h: inner.y + inner.h - top };
        if !self.data.note.is_empty() && !self.compact {
            let nw = p.measure(&self.data.note, 10.);
            p.text(inner.x + inner.w - nw, inner.y + inner.h - 13. * s, &self.data.note.clone(), 10., t.muted, false);
            plot.h -= 17. * s;
        }
        if self.kind == Type::HBar { self.hbars(p, plot, r) } else { self.columns(p, plot, r) }
    }

    fn columns(&self, p: &mut P, plot: R, bounds: R) {
        let (s, t, d) = (p.s, p.t, self.data);
        let (lo, hi) = d.range();
        let ticks = ((plot.h / (44. * s)) as usize).clamp(2, 6);
        let (vmin, vmax, step) = chart::nice_scale(lo, hi, self.kind != Type::Line, ticks);
        let (mut label_h, mut axis_w) = (0., 0.);
        if !self.compact {
            label_h = 18. * s;
            let mut x = vmin;
            while x <= vmax + step / 2. {
                axis_w = f32::max(axis_w, p.measure(&chart::tick_text(x, vmax, step), 10.5));
                x += step;
            }
            axis_w += 8. * s;
        }
        let area = R { x: plot.x + axis_w, y: plot.y + 4. * s, w: plot.w - axis_w, h: plot.h - label_h - 4. * s };
        if area.w <= 4. * s || area.h <= 4. * s {
            return;
        }
        let y_of = |x: f64| area.y + area.h - ((x - vmin) / (vmax - vmin)) as f32 * area.h;
        let grid = t.border.opacity(0.55);
        let mut x = vmin;
        while x <= vmax + step / 2. {
            let y = y_of(x);
            p.line(area.x, y, area.x + area.w, y, grid);
            if !self.compact {
                let label = chart::tick_text(x, vmax, step);
                let w = p.measure(&label, 10.5);
                p.text(area.x - 8. * s - w, y - 7. * s, &label, 10.5, t.muted, false);
            }
            x += step;
        }
        let base = y_of(0f64.min(vmax).max(vmin));
        p.line(area.x, base, area.x + area.w, base, t.border);

        let n = d.cats.len();
        let band = area.w / n as f32;
        let mut hov = None;
        if let Some((hx, hy)) = self.hover
            && hx >= area.x
            && hx < area.x + area.w
            && hy >= plot.y
            && hy < plot.y + plot.h
        {
            let i = (((hx - area.x) / band) as usize).min(n - 1);
            hov = Some(i);
            p.rect(R { x: area.x + band * i as f32, y: area.y, w: band, h: area.h }, t.text.opacity(0.045), 3. * s);
        }
        if !self.compact {
            let widest = d.cats.iter().map(|c| p.measure(c, 10.5)).fold(0., f32::max);
            let every = (((widest + 8. * s) / band).ceil() as usize).max(1);
            for (i, c) in d.cats.iter().enumerate() {
                if i % every != 0 {
                    continue;
                }
                let w = p.measure(c, 10.5);
                let cx = area.x + band * (i as f32 + 0.5);
                p.text(cx - w / 2., area.y + area.h + 4. * s, c, 10.5, t.muted, false);
            }
        }
        let nser = d.series.len();
        match self.kind {
            Type::Bar => {
                let gap = 2. * s;
                let w = ((band * 0.72 - gap * (nser as f32 - 1.)) / nser as f32).min(24. * s).max(1.);
                let group = w * nser as f32 + gap * (nser as f32 - 1.);
                for (si, row) in d.vals.iter().enumerate() {
                    for (i, &v) in row.iter().enumerate() {
                        if v.is_nan() {
                            continue;
                        }
                        let bx = area.x + band * i as f32 + (band - group) / 2. + si as f32 * (w + gap);
                        let (y0, y1) = (y_of(v).min(base), y_of(v).max(base));
                        bar(p, R { x: bx, y: y0, w, h: y1 - y0 }, v >= 0., false, hue(si, t.dark));
                    }
                }
            }
            _ => {
                for (si, row) in d.vals.iter().enumerate() {
                    let color = hue(si, t.dark);
                    let mut run: Vec<(f32, f32)> = Vec::new();
                    let flush = |p: &mut P, run: &mut Vec<(f32, f32)>| {
                        match run.len() {
                            0 => {}
                            1 => p.circle(run[0].0, run[0].1, 3. * s, color),
                            _ => {
                                if self.kind == Type::Area {
                                    let mut poly = vec![(run[0].0, base)];
                                    poly.extend(run.iter().copied());
                                    poly.push((run[run.len() - 1].0, base));
                                    p.path(&poly, color.opacity(0.12));
                                }
                                p.polyline(run, 2. * s, color);
                            }
                        }
                        run.clear();
                    };
                    for (i, &v) in row.iter().enumerate() {
                        if v.is_nan() {
                            flush(p, &mut run);
                        } else {
                            run.push((area.x + band * (i as f32 + 0.5), y_of(v)));
                        }
                    }
                    flush(p, &mut run);
                }
                if let Some(i) = hov {
                    for (si, row) in d.vals.iter().enumerate() {
                        if !row[i].is_nan() {
                            let (cx, cy) = (area.x + band * (i as f32 + 0.5), y_of(row[i]));
                            p.circle(cx, cy, 6. * s, t.surface);
                            p.circle(cx, cy, 4. * s, hue(si, t.dark));
                        }
                    }
                }
            }
        }
        if let Some(i) = hov.filter(|_| !self.compact) {
            self.tooltip_at(p, bounds, i);
        }
    }

    fn hbars(&self, p: &mut P, plot: R, bounds: R) {
        let (s, t, d) = (p.s, p.t, self.data);
        let (lo, hi) = d.range();
        let ticks = ((plot.w / (72. * s)) as usize).clamp(2, 6);
        let (vmin, vmax, step) = chart::nice_scale(lo, hi, true, ticks);
        let (mut label_w, mut axis_h, mut right) = (0., 0., 8. * s);
        if !self.compact {
            label_w = d.cats.iter().map(|c| p.measure(c, 10.5)).fold(0., f32::max).min(plot.w * 0.35) + 8. * s;
            axis_h = 18. * s;
            right = f32::max(right, p.measure(&chart::tick_text(vmax, vmax, step), 10.5) / 2. + 2. * s);
        }
        let area = R { x: plot.x + label_w, y: plot.y, w: plot.w - label_w - right, h: plot.h - axis_h };
        if area.w <= 4. * s || area.h <= 4. * s {
            return;
        }
        let x_of = |x: f64| area.x + ((x - vmin) / (vmax - vmin)) as f32 * area.w;
        let grid = t.border.opacity(0.55);
        let mut x = vmin;
        while x <= vmax + step / 2. {
            let gx = x_of(x);
            p.line(gx, area.y, gx, area.y + area.h, grid);
            if !self.compact {
                let label = chart::tick_text(x, vmax, step);
                let w = p.measure(&label, 10.5);
                p.text(gx - w / 2., area.y + area.h + 4. * s, &label, 10.5, t.muted, false);
            }
            x += step;
        }
        let base = x_of(0f64.min(vmax).max(vmin));
        p.line(base, area.y, base, area.y + area.h, t.border);
        let n = d.cats.len();
        let band = area.h / n as f32;
        let mut hov = None;
        if let Some((hx, hy)) = self.hover
            && hy >= area.y
            && hy < area.y + area.h
            && hx >= plot.x
            && hx < plot.x + plot.w
        {
            let i = (((hy - area.y) / band) as usize).min(n - 1);
            hov = Some(i);
            p.rect(R { x: area.x, y: area.y + band * i as f32, w: area.w, h: band }, t.text.opacity(0.045), 3. * s);
        }
        let nser = d.series.len();
        let gap = 2. * s;
        let h = ((band * 0.72 - gap * (nser as f32 - 1.)) / nser as f32).min(24. * s).max(1.);
        let group = h * nser as f32 + gap * (nser as f32 - 1.);
        if !self.compact {
            let every = ((16. * s / band).ceil() as usize).max(1);
            for (i, c) in d.cats.iter().enumerate() {
                if i % every != 0 {
                    continue;
                }
                let w = p.measure(c, 10.5).min(label_w - 8. * s);
                let cy = area.y + band * (i as f32 + 0.5);
                p.text((area.x - 8. * s - w).max(plot.x), cy - 7. * s, c, 10.5, t.muted, false);
            }
        }
        for (si, row) in d.vals.iter().enumerate() {
            for (i, &v) in row.iter().enumerate() {
                if v.is_nan() {
                    continue;
                }
                let by = area.y + band * i as f32 + (band - group) / 2. + si as f32 * (h + gap);
                let (x0, x1) = (x_of(v).min(base), x_of(v).max(base));
                bar(p, R { x: x0, y: by, w: x1 - x0, h }, v >= 0., true, hue(si, t.dark));
            }
        }
        if let Some(i) = hov.filter(|_| !self.compact) {
            self.tooltip_at(p, bounds, i);
        }
    }

    fn tooltip_at(&self, p: &mut P, bounds: R, i: usize) {
        let d = self.data;
        let rows: Vec<(Hsla, String, String)> = d
            .series
            .iter()
            .enumerate()
            .map(|(si, n)| (hue(si, p.t.dark), n.clone(), chart::value_text(d.vals[si][i])))
            .collect();
        self.tooltip(p, bounds, &d.cats[i], &rows);
    }

    fn tooltip(&self, p: &mut P, bounds: R, title: &str, rows: &[(Hsla, String, String)]) {
        let Some((hx, hy)) = self.hover else { return };
        let (s, t) = (p.s, p.t);
        let (pad, line_h, sw) = (8. * s, 17. * s, 8. * s);
        let mut w = p.measure(title, 11.5);
        let name_w = rows.iter().map(|r| p.measure(&r.1, 11.)).fold(0., f32::max);
        let value_w = rows.iter().map(|r| p.measure(&r.2, 11.)).fold(0., f32::max);
        w = w.max(sw + 6. * s + name_w + 16. * s + value_w);
        let mut b = R { x: hx + 14. * s, y: hy + 14. * s, w: w + 2. * pad, h: line_h * (rows.len() + 1) as f32 + 2. * pad - 2. * s };
        if b.x + b.w > bounds.x + bounds.w - 4. * s {
            b.x = hx - 14. * s - b.w;
        }
        if b.y + b.h > bounds.y + bounds.h - 4. * s {
            b.y = hy - 14. * s - b.h;
        }
        b.x = b.x.max(bounds.x + 4. * s);
        b.y = b.y.max(bounds.y + 4. * s);
        p.rect(R { x: b.x, y: b.y + 2. * s, w: b.w, h: b.h }, rgba_u8(0, 0, 0, if t.dark { 0.4 } else { 0.1 }), 6. * s);
        p.rect(R { x: b.x - 1., y: b.y - 1., w: b.w + 2., h: b.h + 2. }, t.border, 6. * s);
        p.rect(b, t.surface, 6. * s);
        p.text(b.x + pad, b.y + pad, title, 11.5, t.text, true);
        for (i, (c, name, value)) in rows.iter().enumerate() {
            let y = b.y + pad + line_h * (i + 1) as f32;
            p.rect(R { x: b.x + pad, y: y + (line_h - sw) / 2. - 2. * s, w: sw, h: sw }, *c, 2. * s);
            p.text(b.x + pad + sw + 6. * s, y, name, 11., t.muted, false);
            let vw = p.measure(value, 11.);
            p.text(b.x + b.w - pad - vw, y, value, 11., t.text, true);
        }
    }

    fn pie(&self, p: &mut P, inner: R, bounds: R) {
        let (s, t, d) = (p.s, p.t, self.data);
        let mut slices: Vec<(String, f64, Hsla)> = Vec::new();
        let mut total = 0.;
        for (i, &x) in d.vals[0].iter().enumerate() {
            if !x.is_nan() && x > 0. {
                slices.push((d.cats[i].clone(), x, gpui::black()));
                total += x;
            }
        }
        if slices.len() > MAX_SERIES {
            let rest: f64 = slices[MAX_SERIES - 1..].iter().map(|s| s.1).sum();
            slices.truncate(MAX_SERIES - 1);
            slices.push(("其他".into(), rest, gpui::black()));
        }
        let n = slices.len();
        for (i, sl) in slices.iter_mut().enumerate() {
            sl.2 = if sl.0 == "其他" && i == MAX_SERIES - 1 { t.muted.opacity(0.6) } else { hue(i, t.dark) };
        }
        if total <= 0. {
            return;
        }
        let mut disc = inner;
        if !self.compact {
            let keys: Vec<(Hsla, String)> =
                slices.iter().map(|sl| (sl.2, format!("{} {}", sl.0, chart::share_text(sl.1, total)))).collect();
            if inner.w >= inner.h * 1.3 {
                let lw = (inner.w * 0.4).min(180. * s);
                disc.w -= lw + 12. * s;
                let line_h = 16. * s;
                let ly = inner.y + ((inner.h - line_h * keys.len() as f32) / 2.).max(0.);
                for (i, k) in keys.iter().enumerate() {
                    legend(p, R { x: inner.x + inner.w - lw, y: ly + line_h * i as f32, w: lw, h: line_h }, std::slice::from_ref(k), true);
                }
            } else {
                let lr = R { x: inner.x, y: 0., w: inner.w, h: inner.h / 3. };
                let h = legend(p, lr, &keys, false);
                legend(p, R { y: inner.y + inner.h - h, ..lr }, &keys, true);
                disc.h -= h + 10. * s;
            }
        }
        let radius = disc.w.min(disc.h) / 2. - 2. * s;
        if radius <= 4. * s {
            return;
        }
        let (cx, cy) = (disc.x + disc.w / 2., disc.y + disc.h / 2.);
        let mut hovered = None;
        if let Some((hx, hy)) = self.hover {
            let (dx, dy) = (hx - cx, hy - cy);
            if dx * dx + dy * dy <= radius * radius {
                let mut a = (dy as f64).atan2(dx as f64) + PI / 2.;
                if a < 0. {
                    a += 2. * PI;
                }
                let mut at = 0.;
                for (i, sl) in slices.iter().enumerate() {
                    let share = sl.1 / total * 2. * PI;
                    if a >= at && a < at + share {
                        hovered = Some(i);
                    }
                    at += share;
                }
            }
        }
        let mut at = -PI / 2.;
        for (i, sl) in slices.iter().enumerate() {
            let share = sl.1 / total * 2. * PI;
            let (mut x, mut y) = (cx, cy);
            if hovered == Some(i) {
                let mid = at + share / 2.;
                x += mid.cos() as f32 * 4. * s;
                y += mid.sin() as f32 * 4. * s;
            }
            let steps = ((share / (PI / 90.)) as usize).max(2);
            let mut pts = Vec::new();
            if n > 1 {
                pts.push((x, y));
            }
            for k in 0..=steps {
                let a = at + share * k as f64 / steps as f64;
                pts.push((x + radius * a.cos() as f32, y + radius * a.sin() as f32));
            }
            p.path(&pts, sl.2);
            if n > 1 {
                // The surface parts it from the next: a gap, not a stroke.
                let mut edge = pts.clone();
                edge.push(pts[0]);
                p.polyline(&edge, 2. * s, t.surface);
            }
            at += share;
        }
        if let (Some(i), false) = (hovered, self.compact) {
            let sl = &slices[i];
            let rows = vec![
                (sl.2, d.series[0].clone(), chart::value_text(sl.1)),
                (sl.2, "占比".to_string(), chart::share_text(sl.1, total)),
            ];
            self.tooltip(p, bounds, &sl.0, &rows);
        }
    }
}

/// Lays legend keys out in rows from r's top-left, drawing them when `draw`
/// says so; says how tall they came out.
fn legend(p: &mut P, r: R, keys: &[(Hsla, String)], draw: bool) -> f32 {
    let s = p.s;
    let (sw, gap, line_h) = (8. * s, 14. * s, 16. * s);
    let (mut x, mut y) = (r.x, r.y);
    for (color, name) in keys {
        let w = p.measure(name, 11.).min(r.w - sw - 6. * s);
        if x > r.x && x + sw + 6. * s + w > r.x + r.w {
            x = r.x;
            y += line_h;
            if y + line_h > r.y + r.h {
                break;
            }
        }
        if draw {
            p.rect(R { x, y: y + (line_h - sw) / 2., w: sw, h: sw }, *color, 2. * s);
            let muted = p.t.muted;
            p.text(x + sw + 6. * s, y + 1.5 * s, name, 11., muted, false);
        }
        x += sw + 6. * s + w + gap;
    }
    y + line_h - r.y
}

/// A bar: its data end rounded, square where it stands on the baseline.
fn bar(p: &mut P, r: R, up: bool, flat: bool, color: Hsla) {
    if r.w <= 0. || r.h <= 0. {
        return;
    }
    let k = if flat { (4. * p.s).min(r.h / 2.).min(r.w) } else { (4. * p.s).min(r.w / 2.).min(r.h) };
    // The rounded end as a rounded rect, the square end covered by a plain one.
    p.rect(r, color, k);
    let sq = match (flat, up) {
        (false, true) => R { y: r.y + r.h - k, h: k, ..r },
        (false, false) => R { h: k, ..r },
        (true, true) => R { w: k, ..r },
        (true, false) => R { x: r.x + r.w - k, w: k, ..r },
    };
    p.rect(sq, color, 0.);
}

/// A chart of the table, filling what it is given; hovering shows values.
pub fn render(src: &str, kind: &str, s: f32, t: &Theme) -> AnyElement {
    render_view(chart::parse(src), chart::of(kind), s, *t, false)
}

pub fn render_compact(src: &str, kind: &str, s: f32, t: &Theme) -> AnyElement {
    render_view(chart::parse(src), chart::of(kind), s, *t, true)
}

fn render_view(data: Data, kind: Type, s: f32, t: Theme, compact: bool) -> AnyElement {
    div()
        .id("chart")
        .size_full()
        // The pointer moving over it redraws the tooltip.
        .on_mouse_move(|_, window, _| window.refresh())
        .on_hover(|_, window, _| window.refresh())
        .child(
            canvas(|_, _, _| (), move |b: Bounds<Pixels>, _, window, cx| {
                let r = R {
                    x: f32::from(b.origin.x),
                    y: f32::from(b.origin.y),
                    w: f32::from(b.size.width),
                    h: f32::from(b.size.height),
                };
                let m = window.mouse_position();
                let hover = (!compact && b.contains(&m)).then(|| (f32::from(m.x), f32::from(m.y)));
                let view = View { kind, data: &data, hover, compact };
                window.with_content_mask(Some(gpui::ContentMask { bounds: b }), |window| {
                    let mut p = P { window, cx, s, t };
                    view.draw(&mut p, r);
                });
            })
            .size_full(),
        )
        .into_any_element()
}
