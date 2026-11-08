//! Patching a 文字's or 图形's style from what an agent sent.

use serde_json::Value;

use super::schema::STYLE_KEYS;
use crate::model;

/// `base` with the keys of `v` (an object) set; null puts a key back to its default.
pub fn style_patch(base: &model::Style, v: &Value) -> Result<model::Style, String> {
    let obj = v.as_object().ok_or("style 要是对象")?;
    let mut st = base.clone();
    for (k, x) in obj {
        if !STYLE_KEYS.contains(&k.as_str()) {
            return Err(format!("style 没有 {k:?}：可用 {}", STYLE_KEYS.join("、")));
        }
        let reset = x.is_null();
        let text = || x.as_str().map(str::trim).map(str::to_string).ok_or(format!("style.{k} 要是字符串"));
        match k.as_str() {
            "fill" | "stroke" | "color" => {
                let c = if reset { String::new() } else { text()? };
                let ok = c.is_empty()
                    || c == "none"
                    || crate::theme::INKS.iter().any(|i| i.name == c)
                    || (c.len() == 7 && c.starts_with('#') && u32::from_str_radix(&c[1..], 16).is_ok());
                if !ok {
                    return Err(format!("style.{k} 不认识 {c:?}：写调色板名、#rrggbb 或 none"));
                }
                let c = c.to_ascii_lowercase();
                match k.as_str() {
                    "fill" => st.fill = c,
                    "stroke" => st.stroke = c,
                    _ => st.color = c,
                }
            }
            "font" => st.font = if reset { String::new() } else { text()? },
            "size" => {
                st.size = if reset { 0. } else { x.as_f64().ok_or("style.size 要是数字")? as f32 };
                if st.size != 0. && !(6. ..=400.).contains(&st.size) {
                    return Err(format!("style.size 要在 6 到 400 之间，收到 {}", st.size));
                }
            }
            "weight" => {
                let w = if reset { 0 } else { x.as_u64().ok_or("style.weight 要是整数")? };
                if w != 0 && (!(100..=900).contains(&w) || w % 100 != 0) {
                    return Err(format!("style.weight 要是 100 到 900 的整百，收到 {w}"));
                }
                st.weight = if w == 400 { 0 } else { w as u16 };
            }
            "italic" => st.italic = !reset && x.as_bool().ok_or("style.italic 要是布尔值")?,
            "radius" => {
                st.radius = if reset { None } else { Some(x.as_f64().ok_or("style.radius 要是数字")? as f32) };
                if st.radius.is_some_and(|r| r < 0.) {
                    return Err("style.radius 不能为负".into());
                }
            }
            "align" | "valign" => {
                let a = if reset { String::new() } else { text()? };
                let allowed: &[&str] = if k == "align" { &["left", "center", "right"] } else { &["top", "middle", "bottom"] };
                if !a.is_empty() && !allowed.contains(&a.as_str()) {
                    return Err(format!("style.{k} 要是 {}，收到 {a:?}", allowed.join("/")));
                }
                if k == "align" {
                    st.align = a;
                } else {
                    st.valign = a;
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(st)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn style_patches_merge_and_are_checked() {
        let base = model::Style { fill: "red".into(), size: 30., ..Default::default() };
        let st = style_patch(&base, &json!({"stroke": "#AABBCC", "weight": 700, "align": "center", "size": null})).unwrap();
        assert_eq!(st.fill, "red", "keys not given stay");
        assert_eq!((st.stroke.as_str(), st.weight, st.align.as_str(), st.size), ("#aabbcc", 700, "center", 0.));
        assert_eq!(style_patch(&st, &json!({"weight": 400})).unwrap().weight, 0, "regular is the default");
        for bad in [
            json!({"fill": "rouge"}),
            json!({"fill": "#12345"}),
            json!({"size": 2}),
            json!({"weight": 450}),
            json!({"align": "middle"}),
            json!({"radius": -1}),
            json!({"shadow": 1}),
            json!("red"),
        ] {
            assert!(style_patch(&base, &bad).is_err(), "{bad}");
        }
    }
}
