//! Layouts work out where blocks go first; the model then moves them as one change.

pub type Rect = (String, f32, f32, f32, f32);

pub fn bounds(rs: &[Rect]) -> (f32, f32, f32, f32) {
    let x0 = rs.iter().map(|r| r.1).fold(f32::MAX, f32::min);
    let y0 = rs.iter().map(|r| r.2).fold(f32::MAX, f32::min);
    let x1 = rs.iter().map(|r| r.1 + r.3).fold(f32::MIN, f32::max);
    let y1 = rs.iter().map(|r| r.2 + r.4).fold(f32::MIN, f32::max);
    (x0, y0, x1 - x0, y1 - y0)
}

pub fn align(
    rs: &[Rect],
    align: &str,
    r: (f32, f32, f32, f32),
    ref_id: Option<&str>,
    spacing: Option<f32>,
) -> Vec<(String, f32, f32)> {
    let horizontal = matches!(align, "left" | "right" | "centerX");
    let mut out: Vec<(String, f32, f32)> = rs
        .iter()
        .map(|b| {
            if Some(b.0.as_str()) == ref_id {
                return (b.0.clone(), b.1, b.2);
            }
            let (mut x, mut y) = (b.1, b.2);
            match align {
                "left" => x = r.0,
                "right" => x = r.0 + r.2 - b.3,
                "centerX" => x = r.0 + r.2 / 2. - b.3 / 2.,
                "top" => y = r.1,
                "bottom" => y = r.1 + r.3 - b.4,
                _ => y = r.1 + r.3 / 2. - b.4 / 2.,
            }
            (b.0.clone(), x, y)
        })
        .collect();
    if let Some(sp) = spacing {
        let mut order: Vec<usize> = (0..rs.len()).collect();
        order.sort_by(|&i, &j| {
            let (a, b) = (&rs[i], &rs[j]);
            let k = |r: &Rect| if horizontal { (r.2, r.1) } else { (r.1, r.2) };
            k(a).partial_cmp(&k(b)).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut cursor = if horizontal { r.1 } else { r.0 };
        if let Some(rb) = ref_id.and_then(|id| rs.iter().find(|b| b.0 == id)) {
            cursor = if horizontal { rb.2 } else { rb.1 };
        }
        for i in order {
            if horizontal {
                out[i].2 = cursor;
                cursor += rs[i].4 + sp;
            } else {
                out[i].1 = cursor;
                cursor += rs[i].3 + sp;
            }
        }
    }
    out
}

#[allow(clippy::type_complexity)]
pub fn distribute(
    rs: &[Rect],
    horizontal: bool,
    centers: bool,
    allow_overlap: bool,
) -> Result<(Vec<String>, Vec<(String, f32, f32)>, f32), String> {
    let key = |r: &Rect| match (horizontal, centers) {
        (true, true) => r.1 + r.3 / 2.,
        (true, false) => r.1,
        (false, true) => r.2 + r.4 / 2.,
        (false, false) => r.2,
    };
    let cross = |r: &Rect| if horizontal { r.2 } else { r.1 };
    let mut o: Vec<&Rect> = rs.iter().collect();
    o.sort_by(|a, b| (key(a), cross(a)).partial_cmp(&(key(b), cross(b))).unwrap_or(std::cmp::Ordering::Equal));
    let (first, last) = (o[0], o[o.len() - 1]);
    let n = o.len() as f32;
    let mut ps = Vec::new();
    let value;
    if !centers {
        let span = if horizontal { last.1 + last.3 - first.1 } else { last.2 + last.4 - first.2 };
        let sizes: f32 = o.iter().map(|r| if horizontal { r.3 } else { r.4 }).sum();
        let gap = (span - sizes) / (n - 1.);
        if gap < 0. && !allow_overlap {
            return Err(format!(
                "按当前首尾排不下：block 共占 {sizes:.0}，首尾跨度 {span:.0}；允许重叠传 allowOverlap=true"
            ));
        }
        let mut cursor = if horizontal { first.1 } else { first.2 };
        for r in &o {
            if horizontal {
                ps.push((r.0.clone(), cursor, r.2));
                cursor += r.3 + gap;
            } else {
                ps.push((r.0.clone(), r.1, cursor));
                cursor += r.4 + gap;
            }
        }
        value = gap;
    } else {
        let (c0, c1) = (key(first), key(last));
        let step = (c1 - c0) / (n - 1.);
        for (i, r) in o.iter().enumerate() {
            let c = c0 + step * i as f32;
            if horizontal {
                ps.push((r.0.clone(), c - r.3 / 2., r.2));
            } else {
                ps.push((r.0.clone(), r.1, c - r.4 / 2.));
            }
        }
        value = step;
    }
    Ok((o.iter().map(|r| r.0.clone()).collect(), ps, value))
}

#[allow(clippy::type_complexity)]
pub fn arrange(
    rs: &[Rect],
    layout: &str,
    columns: usize,
    gap_x: f32,
    gap_y: f32,
    origin: (Option<f32>, Option<f32>),
    by_position: bool,
) -> (Vec<String>, Vec<(String, f32, f32)>, usize, usize) {
    let mut o: Vec<&Rect> = rs.iter().collect();
    if by_position {
        let row = layout == "row";
        o.sort_by(|a, b| {
            let k = |r: &Rect| if row { (r.1, r.2) } else { (r.2, r.1) };
            k(a).partial_cmp(&k(b)).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    let n = o.len();
    let cols = match layout {
        "row" => n,
        "column" => 1,
        _ if columns > 0 => columns,
        _ => (n as f64).sqrt().ceil() as usize,
    }
    .clamp(1, n.max(1));
    let rows = n.div_ceil(cols);
    let mut col_w = vec![0f32; cols];
    let mut row_h = vec![0f32; rows];
    for (i, r) in o.iter().enumerate() {
        col_w[i % cols] = col_w[i % cols].max(r.3);
        row_h[i / cols] = row_h[i / cols].max(r.4);
    }
    let mut col_x = vec![0f32; cols];
    let mut row_y = vec![0f32; rows];
    for c in 1..cols {
        col_x[c] = col_x[c - 1] + col_w[c - 1] + gap_x;
    }
    for r in 1..rows {
        row_y[r] = row_y[r - 1] + row_h[r - 1] + gap_y;
    }
    let owned: Vec<Rect> = o.iter().map(|r| (*r).clone()).collect();
    let b = bounds(&owned);
    let (ox, oy) = (origin.0.unwrap_or(b.0), origin.1.unwrap_or(b.1));
    let ps = o
        .iter()
        .enumerate()
        .map(|(i, r)| (r.0.clone(), ox + col_x[i % cols], oy + row_y[i / cols]))
        .collect();
    (o.iter().map(|r| r.0.clone()).collect(), ps, cols, rows)
}
