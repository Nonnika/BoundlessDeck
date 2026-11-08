//! The tools that lay blocks out: align, distribute and arrange. Where
//! blocks go is worked out in `layout`; the model moves them as one change.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Args, Ctx, arg_bool, arg_int, arg_num, arg_str, arg_strings, layout, round};

impl Ctx<'_> {
    pub(super) fn layout_ids(&self, args: &Args) -> Result<Vec<String>, String> {
        let mut ids = arg_strings(args, "ids")?;
        if ids.is_empty() {
            ids = self.selected_ids();
        }
        if ids.is_empty() {
            return Err("没有指定 block：给 ids，或先在画布上选中几个".into());
        }
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
        self.check_ids(&ids)?;
        Ok(ids)
    }

    pub(super) fn rects(&self, ids: &[String]) -> Vec<(String, f32, f32, f32, f32)> {
        ids.iter()
            .map(|id| {
                let b = self.m.block(id).unwrap();
                (id.clone(), b.x, b.y, b.w, b.h)
            })
            .collect()
    }

    pub(super) fn geometry_list(&self, ids: &[String]) -> Vec<Value> {
        ids.iter()
            .map(|id| {
                let b = self.m.block(id).unwrap();
                json!({"id": id, "kind": b.kind.mcp_name(), "title": b.title, "x": round(b.x), "y": round(b.y), "w": round(b.w), "h": round(b.h)})
            })
            .collect()
    }

    pub(super) fn align_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = self.layout_ids(args)?;
        let align = arg_str(args, "align")?.ok_or("align_blocks 需要 align：left、right、top、bottom、centerX 或 centerY")?;
        let align = match align.trim().to_ascii_lowercase().as_str() {
            "left" | "start" => "left",
            "right" | "end" => "right",
            "top" => "top",
            "bottom" => "bottom",
            "centerx" | "center-x" | "hc" | "hcenter" | "horizontal-center" | "xcenter" => "centerX",
            "centery" | "center-y" | "vc" | "vcenter" | "vertical-center" | "middle" | "ycenter" => "centerY",
            _ => return Err(format!("align 要是 left、right、top、bottom、centerX 或 centerY，收到 {align:?}")),
        };
        let to = arg_str(args, "to")?.unwrap_or_default();
        let spacing = arg_num(args, "spacing")?;
        if spacing.is_some_and(|s| s < 0.) {
            return Err("spacing 不能是负数".into());
        }
        let rs = self.rects(&ids);
        let bounds = layout::bounds(&rs);
        let (ref_rect, ref_id) = match to.trim().to_ascii_lowercase().as_str() {
            "" | "bounds" | "set" => {
                if rs.len() < 2 {
                    return Err("按 bounds 对齐需要至少两个 block；对齐到一个 block 用 to".into());
                }
                (bounds, None)
            }
            "first" | "last" => {
                if rs.len() < 2 {
                    return Err(format!("按 {to} 对齐需要至少两个 block"));
                }
                let r = if to.eq_ignore_ascii_case("first") { &rs[0] } else { &rs[rs.len() - 1] };
                ((r.1, r.2, r.3, r.4), Some(r.0.clone()))
            }
            _ => {
                let b = self.m.block(&to).ok_or_else(|| format!("to 要是 bounds、first、last 或一个 block ID，收到 {to:?}"))?;
                ((b.x, b.y, b.w, b.h), Some(to.clone()))
            }
        };
        let ps = layout::align(&rs, align, ref_rect, ref_id.as_deref(), spacing.map(|s| s as f32));
        let moved = self.m.apply_placements(&ps);
        self.dirty |= moved > 0;
        let mut out = json!({"align": align, "to": ref_id.unwrap_or("bounds".into()), "moved": moved, "blocks": self.geometry_list(&ids)});
        if let Some(s) = spacing {
            out["spacing"] = s.into();
        }
        Ok(out)
    }

    pub(super) fn distribute_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = self.layout_ids(args)?;
        let axis = match arg_str(args, "axis")?.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "x" | "h" | "horizontal" | "row" => "x",
            "y" | "v" | "vertical" | "column" => "y",
            a => return Err(format!("axis 要是 x 或 y，收到 {a:?}")),
        };
        let mode = match arg_str(args, "mode")?.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "gaps" | "gap" | "space" => "gaps",
            "centers" | "center" => "centers",
            m => return Err(format!("mode 要是 gaps 或 centers，收到 {m:?}")),
        };
        let allow = arg_bool(args, "allowOverlap")?.unwrap_or(false);
        if ids.len() < 3 {
            return Err(format!("distribute_blocks 需要至少三个 block（首尾保持不动），收到 {} 个", ids.len()));
        }
        let rs = self.rects(&ids);
        let (order, ps, gap_or_step) = layout::distribute(&rs, axis == "x", mode == "centers", allow)?;
        let moved = self.m.apply_placements(&ps);
        self.dirty |= moved > 0;
        let mut out = json!({"axis": axis, "mode": mode, "moved": moved, "blocks": self.geometry_list(&order)});
        out[if mode == "gaps" { "gap" } else { "step" }] = round(gap_or_step).into();
        Ok(out)
    }

    pub(super) fn arrange_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = self.layout_ids(args)?;
        let layout_ = match arg_str(args, "layout")?.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "grid" => "grid",
            "row" | "line" | "horizontal" => "row",
            "column" | "col" | "vertical" | "stack" => "column",
            l => return Err(format!("layout 要是 row、column 或 grid，收到 {l:?}")),
        };
        let gap_x = arg_num(args, "gapX")?.unwrap_or(24.);
        let gap_y = arg_num(args, "gapY")?.unwrap_or(24.);
        if gap_x < 0. || gap_y < 0. {
            return Err("gapX 与 gapY 不能是负数".into());
        }
        let columns = arg_int(args, "columns")?.unwrap_or(0).max(0) as usize;
        let origin = (arg_num(args, "originX")?.map(|v| v as f32), arg_num(args, "originY")?.map(|v| v as f32));
        let by_position = match arg_str(args, "order")?.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "ids" => false,
            "position" => true,
            o => return Err(format!("order 要是 ids 或 position，收到 {o:?}")),
        };
        let rs = self.rects(&ids);
        let (order, ps, cols, rows) =
            layout::arrange(&rs, layout_, columns, gap_x as f32, gap_y as f32, origin, by_position);
        let moved = self.m.apply_placements(&ps);
        self.dirty |= moved > 0;
        Ok(json!({"layout": layout_, "columns": cols, "rows": rows, "gapX": gap_x, "gapY": gap_y, "moved": moved, "blocks": self.geometry_list(&order)}))
    }
}
