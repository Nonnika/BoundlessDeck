//! The 图表 block's table and a plain drawing of it: bars, lines or a pie.
//! (The Go build's full chart — hbar/area, axis ticks, legends, tooltips —
//! is not ported yet; other types draw as bars.)

use gpui::{AnyElement, Bounds, Hsla, IntoElement, ParentElement, PathBuilder, Styled, canvas, div, fill, point, px, size};

use crate::theme::{Theme, rgba_u8};

#[derive(Debug, Default, PartialEq)]
pub struct Data {
    pub series: Vec<String>,
    pub labels: Vec<String>,
    /// values[row][series]
    pub values: Vec<Vec<f64>>,
}

fn cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let sep: &[char] = if line.contains('\t') {
        &['\t']
    } else if line.contains('|') {
        &['|']
    } else {
        &[',', '，', ';', '；']
    };
    line.split(sep).map(|c| c.trim().to_string()).collect()
}

fn number(s: &str) -> Option<f64> {
    s.replace([',', '，', '%'], "").trim().parse().ok()
}

/// Reads rows of `名称, 数值…`; a first row with a non-number names the series.
pub fn parse(src: &str) -> Data {
    let mut d = Data::default();
    for (i, line) in src.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let c = cells(line);
        if c.iter().all(|x| x.chars().all(|ch| matches!(ch, '-' | ':' | ' '))) {
            continue; // a Markdown table's rule
        }
        if c.len() < 2 {
            continue;
        }
        let nums: Vec<Option<f64>> = c[1..].iter().map(|x| number(x)).collect();
        if i == 0 && nums.iter().any(Option::is_none) {
            d.series = c[1..].to_vec();
            continue;
        }
        d.labels.push(c[0].clone());
        d.values.push(nums.into_iter().map(|n| n.unwrap_or(0.)).collect());
    }
    let n = d.values.iter().map(Vec::len).max().unwrap_or(0).min(8);
    d.series.resize(n, String::new());
    for row in &mut d.values {
        row.resize(n, 0.);
    }
    d
}

/// Categorical hues, in a fixed order.
fn hue(i: usize) -> Hsla {
    const H: [(u8, u8, u8); 8] = [
        (0x3b, 0x6c, 0xf6),
        (0x1f, 0xa8, 0x8f),
        (0xf5, 0x9e, 0x0b),
        (0xe1, 0x4d, 0x6b),
        (0x8b, 0x5c, 0xf6),
        (0x0e, 0xa5, 0xe9),
        (0x84, 0xa3, 0x16),
        (0x6b, 0x72, 0x80),
    ];
    let (r, g, b) = H[i % 8];
    rgba_u8(r, g, b, 1.)
}

pub fn render(src: &str, kind: &str, s: f32, t: &Theme) -> AnyElement {
    let data = parse(src);
    if data.values.is_empty() {
        return div()
            .size_full()
            .p(px(10. * s))
            .text_size(px(12. * s))
            .text_color(t.muted)
            .child("双击编辑数据：每行「名称, 数值」")
            .into_any_element();
    }
    let kind = kind.to_string();
    let (axis, muted) = (t.border, t.muted);
    let labels = data.labels.clone();
    div()
        .size_full()
        .flex()
        .flex_col()
        .p(px(10. * s))
        .gap(px(4. * s))
        .child(
            canvas(|_, _, _| (), move |b, _, window, _| draw(window, b, &data, &kind, axis))
                .flex_1()
                .w_full(),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .justify_around()
                .text_size(px(10. * s))
                .text_color(muted)
                .children(labels.into_iter().map(|l| div().overflow_hidden().whitespace_nowrap().child(l))),
        )
        .into_any_element()
}

fn draw(window: &mut gpui::Window, b: Bounds<gpui::Pixels>, d: &Data, kind: &str, axis: Hsla) {
    let (x0, y0) = (f32::from(b.origin.x), f32::from(b.origin.y));
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let max = d.values.iter().flatten().cloned().fold(0f64, f64::max).max(1e-9) as f32;
    let n = d.values.len();
    let ns = d.series.len().max(1);
    match kind {
        "pie" => {
            let total: f64 = d.values.iter().map(|r| r.first().copied().unwrap_or(0.).max(0.)).sum();
            if total <= 0. {
                return;
            }
            let (cx, cy, r) = (x0 + w / 2., y0 + h / 2., w.min(h) / 2. * 0.9);
            let mut a = -std::f32::consts::FRAC_PI_2;
            for (i, row) in d.values.iter().enumerate() {
                let sweep = (row.first().copied().unwrap_or(0.).max(0.) / total) as f32 * std::f32::consts::TAU;
                let mut p = PathBuilder::fill();
                p.move_to(point(px(cx), px(cy)));
                let steps = ((sweep / 0.05).ceil() as i32).max(2);
                for k in 0..=steps {
                    let ang = a + sweep * k as f32 / steps as f32;
                    p.line_to(point(px(cx + r * ang.cos()), px(cy + r * ang.sin())));
                }
                p.close();
                if let Ok(p) = p.build() {
                    window.paint_path(p, hue(i));
                }
                a += sweep;
            }
        }
        "line" | "area" => {
            window.paint_quad(fill(Bounds::new(point(px(x0), px(y0 + h - 1.)), size(px(w), px(1.))), axis));
            for si in 0..ns {
                let mut p = PathBuilder::stroke(px(2.));
                for (i, row) in d.values.iter().enumerate() {
                    let x = x0 + w * (i as f32 + 0.5) / n as f32;
                    let y = y0 + h - h * (row[si] as f32 / max);
                    if i == 0 {
                        p.move_to(point(px(x), px(y)));
                    } else {
                        p.line_to(point(px(x), px(y)));
                    }
                }
                if let Ok(p) = p.build() {
                    window.paint_path(p, hue(si));
                }
            }
        }
        _ => {
            window.paint_quad(fill(Bounds::new(point(px(x0), px(y0 + h - 1.)), size(px(w), px(1.))), axis));
            let slot = w / n as f32;
            let bw = (slot * 0.7 / ns as f32).min(24.);
            for (i, row) in d.values.iter().enumerate() {
                let start = x0 + slot * i as f32 + (slot - bw * ns as f32) / 2.;
                for (si, v) in row.iter().enumerate() {
                    let bh = h * (*v as f32 / max).max(0.);
                    window.paint_quad(
                        fill(Bounds::new(point(px(start + bw * si as f32), px(y0 + h - bh)), size(px(bw - 1.), px(bh))), hue(si))
                            .corner_radii(gpui::Corners { top_left: px(3.), top_right: px(3.), ..Default::default() }),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tabs_commas_and_markdown_tables() {
        let d = parse("名称\t销量\t利润\n一月\t12\t3\n二月\t1,200\t4");
        assert_eq!(d.series, vec!["销量", "利润"]);
        assert_eq!(d.values[1], vec![1200., 4.]);
        let d = parse("a, 1\nb, 2");
        assert_eq!(d.labels, vec!["a", "b"]);
        let d = parse("| 名称 | 值 |\n|---|---|\n| x | 5 |");
        assert_eq!(d.values, vec![vec![5.]]);
    }
}
