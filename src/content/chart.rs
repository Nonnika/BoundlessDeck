//! A small table, one row per category and a column per series, as the 图表
//! block reads it. Cells part at tabs (as a spreadsheet copies them), else
//! at commas or semicolons; a Markdown table reads too. The first row names
//! the series when a cell after its first is no number.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Type {
    Bar,
    HBar,
    Line,
    Area,
    Pie,
}

/// Every chart a block draws, in the order its menu lists them.
pub const TYPES: [(Type, &str, &str, &str); 5] = [
    (Type::Bar, "bar", "柱状图", "chart"),
    (Type::HBar, "hbar", "条形图", "chart_bar"),
    (Type::Line, "line", "折线图", "chart_line"),
    (Type::Area, "area", "面积图", "chart_area"),
    (Type::Pie, "pie", "饼图", "chart_pie"),
];

/// The chart a block's `chart` field names: bars when it names none.
pub fn of(name: &str) -> Type {
    TYPES.iter().find(|t| t.1 == name).map(|t| t.0).unwrap_or(Type::Bar)
}

pub fn label(t: Type) -> &'static str {
    TYPES.iter().find(|x| x.0 == t).map(|x| x.2).unwrap_or("图表")
}

pub fn icon(t: Type) -> &'static str {
    TYPES.iter().find(|x| x.0 == t).map(|x| x.3).unwrap_or("chart")
}

/// The table a new 图表 block starts with.
pub const SAMPLE: &str = "月份, 收入, 支出\n1月, 120, 80\n2月, 150, 95\n3月, 98, 70\n4月, 180, 110\n5月, 165, 120";

pub const MAX_SERIES: usize = 8;
const MAX_CATS: usize = 200;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Data {
    pub series: Vec<String>,
    pub cats: Vec<String>,
    /// [series][category]; NaN where a cell holds no number.
    pub vals: Vec<Vec<f64>>,
    /// What of the table was left out.
    pub note: String,
}

impl Data {
    pub fn is_empty(&self) -> bool {
        self.cats.is_empty() || self.series.is_empty() || self.vals.iter().flatten().all(|v| v.is_nan())
    }

    /// The line a card in a 块堆 says of the chart.
    pub fn summary(&self, t: Type) -> String {
        if self.is_empty() {
            format!("{} · 还没有数据", label(t))
        } else if self.series.len() == 1 {
            format!("{} · {} 项", label(t), self.cats.len())
        } else {
            format!("{} · {} 项 × {} 组", label(t), self.cats.len(), self.series.len())
        }
    }

    pub fn range(&self) -> (f64, f64) {
        self.vals.iter().flatten().filter(|v| !v.is_nan()).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
            (lo.min(v), hi.max(v))
        })
    }
}

fn is_rule(cells: &[String]) -> bool {
    cells.iter().all(|c| c.trim().trim_matches(|ch| ch == '-' || ch == ':' || ch == ' ').is_empty())
}

/// A cell as a number: 1,200 and 1200 alike, a currency sign or % left out.
pub fn cell_number(s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches(['¥', '￥', '$', '€', '£']);
    let s = s.strip_suffix('%').unwrap_or(s).replace([',', '_'], "");
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok().filter(|f| f.is_finite())
}

pub fn parse(src: &str) -> Data {
    let mut d = Data::default();
    let tabs = src.contains('\t');
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cells: Vec<String> = if line.starts_with('|') {
            let c: Vec<String> = line.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
            if is_rule(&c) {
                continue;
            }
            c
        } else if tabs {
            line.split('\t').map(|c| c.trim().to_string()).collect()
        } else {
            line.replace(['，', ';', '；'], ",").split(',').map(|c| c.trim().to_string()).collect()
        };
        rows.push(cells);
    }
    if rows.is_empty() {
        return d;
    }
    let head = rows[0].clone();
    let from = if head.len() == 1 { 0 } else { 1 };
    let header = head[from..].iter().any(|c| !c.is_empty() && cell_number(c).is_none());
    let mut data: Vec<Vec<String>> = if header { rows[1..].to_vec() } else { rows };
    let cols = data.iter().map(Vec::len).max().unwrap_or(0);
    let labeled = cols >= 2;
    let mut nser = if labeled { cols - 1 } else { 1 };
    if nser > MAX_SERIES {
        d.note = format!("只画前 {MAX_SERIES} 列");
        nser = MAX_SERIES;
    }
    if data.len() > MAX_CATS {
        d.note = format!("只画前 {MAX_CATS} 行");
        data.truncate(MAX_CATS);
    }
    for i in 0..nser {
        let mut name = String::new();
        if header && labeled && i + 1 < head.len() {
            name = head[i + 1].clone();
        } else if header && !labeled {
            name = head[0].clone();
        }
        if name.is_empty() {
            name = if nser > 1 { format!("系列 {}", i + 1) } else { "数值".into() };
        }
        d.series.push(name);
    }
    d.vals = vec![Vec::new(); nser];
    for (i, r) in data.iter().enumerate() {
        let (cat, cells) = if labeled { (r[0].clone(), &r[1..]) } else { ((i + 1).to_string(), &r[..]) };
        d.cats.push(cat);
        for si in 0..nser {
            d.vals[si].push(cells.get(si).and_then(|c| cell_number(c)).unwrap_or(f64::NAN));
        }
    }
    d
}

/// The axis the values lie on: from lo to hi in round steps; with zero, 0 is on it.
pub fn nice_scale(mut lo: f64, mut hi: f64, zero: bool, ticks: usize) -> (f64, f64, f64) {
    if zero {
        lo = lo.min(0.);
        hi = hi.max(0.);
    }
    if hi == lo {
        let pad = if hi == 0. { 1. } else { hi.abs() * 0.5 };
        if zero && lo == 0. {
            hi += pad;
        } else {
            lo -= pad;
            hi += pad;
        }
    }
    let raw = (hi - lo) / ticks.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let step = mag
        * if f <= 1. {
            1.
        } else if f <= 2. {
            2.
        } else if f <= 2.5 {
            2.5
        } else if f <= 5. {
            5.
        } else {
            10.
        };
    ((lo / step + 1e-9).floor() * step, (hi / step - 1e-9).ceil() * step, step)
}

/// An axis value: in 万 or 亿 on an axis that reaches them, as many decimals as its step needs.
pub fn tick_text(v: f64, top: f64, step: f64) -> String {
    let (unit, div) = if top.abs() >= 1e8 {
        ("亿", 1e8)
    } else if top.abs() >= 1e4 {
        ("万", 1e4)
    } else {
        ("", 1.)
    };
    let mut dec = 0;
    let mut st = step / div;
    while dec < 4 && (st - st.round()).abs() > 1e-9 * st.max(1.) {
        st *= 10.;
        dec += 1;
    }
    if v == 0. {
        return "0".into();
    }
    format!("{:.*}{unit}", dec, v / div)
}

/// A value as a tooltip shows it: in full, thousands grouped, at most two decimals.
pub fn value_text(v: f64) -> String {
    if v.is_nan() {
        return "—".into();
    }
    let r = (v * 100.).round() / 100.;
    let s = format!("{r}");
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => ("-", rest.to_string()),
        None => ("", s),
    };
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w.to_string(), Some(f.to_string())),
        None => (s, None),
    };
    let mut b = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            b.push(',');
        }
        b.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{b}.{f}"),
        None => format!("{sign}{b}"),
    }
}

/// Part of total, in percent.
pub fn share_text(part: f64, total: f64) -> String {
    let pct = part / total * 100.;
    if pct >= 10. || pct == pct.trunc() { format!("{}%", pct.round()) } else { format!("{pct:.1}%") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_tables_as_typed() {
        let cd = parse("月份, 收入, 支出\n1月, 120, 80\n2月, , ¥150\n3月，98，70%");
        assert_eq!(cd.series, vec!["收入", "支出"]);
        assert_eq!(cd.cats, vec!["1月", "2月", "3月"]);
        assert_eq!((cd.vals[0][0], cd.vals[1][2], cd.vals[1][1]), (120., 70., 150.));
        assert!(cd.vals[0][1].is_nan());

        let tabs = parse("城市\t人口\n北京\t2,189\n上海\t2,487");
        assert_eq!((tabs.series[0].as_str(), tabs.vals[0][1]), ("人口", 2487.));

        let md = parse("| 项目 | 数量 |\n|---|---:|\n| 甲 | 3 |\n| 乙 | 5 |");
        assert_eq!((md.cats.clone(), md.vals[0][1]), (vec!["甲".to_string(), "乙".to_string()], 5.));

        let bare = parse("2019, 30, 40\n2020, 35, 45");
        assert_eq!((bare.cats.len(), bare.cats[0].as_str(), bare.series[1].as_str()), (2, "2019", "系列 2"));

        let one = parse("销量\n5\n8");
        assert_eq!((one.cats.join("|"), one.series[0].as_str(), one.vals[0][1]), ("1|2".into(), "销量", 8.));

        let wide = parse("x,a,b,c,d,e,f,g,h,i\n1,1,2,3,4,5,6,7,8,9");
        assert!(wide.series.len() == MAX_SERIES && !wide.note.is_empty());

        assert!(parse("").is_empty() && parse("a, b\nx, y").is_empty());
    }

    #[test]
    fn scale_is_round() {
        assert_eq!(nice_scale(3., 187., true, 5), (0., 200., 50.));
        let (lo, hi, step) = nice_scale(-12., 30., true, 4);
        assert!(lo <= -12. && hi >= 30. && (lo % step) == 0.);
        assert_eq!(tick_text(20000., 40000., 10000.), "2万");
        assert_eq!(tick_text(0.5, 1., 0.25), "0.50");
        assert_eq!(value_text(1234567.891), "1,234,567.89");
        assert_eq!(value_text(-1200.), "-1,200");
        assert_eq!(share_text(1., 3.), "33%");
        assert_eq!(share_text(1., 30.), "3.3%");
    }
}
