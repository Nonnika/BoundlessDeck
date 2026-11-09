//! 焦点 for agents: list them, keep and change them and the lines through
//! them, and play a line in the window, a stop at a time, to walk a person
//! through the canvas. Playing is the window's: a tool says where to go
//! (`Ctx::play`), the window glides there.

use serde_json::{Value, json};

use super::{Args, Ctx, arg_bool, arg_int, arg_num, arg_str, arg_strings, round};
use crate::model::{Focus, MAX_SCALE, MIN_SCALE};

/// Where playing is: the line (none: the list itself) and the stop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playing {
    pub line: Option<String>,
    pub at: usize,
}

/// What a tool asks of playing.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayCmd {
    Go(Playing),
    Stop,
}

fn focus_json(f: &Focus, index: usize) -> Value {
    json!({"id": f.id, "name": f.name, "x": round(f.x), "y": round(f.y), "scale": round(f.s), "index": index})
}

impl Ctx<'_> {
    pub(super) fn focus_list(&self) -> Value {
        let focus: Vec<Value> = self.m.focus.iter().enumerate().map(|(i, f)| focus_json(f, i)).collect();
        let lines: Vec<Value> = self.m.focus_lines.iter().map(|l| json!({"id": l.id, "name": l.name, "stops": l.stops})).collect();
        json!({"focus": focus, "lines": lines, "playing": self.playing_json(self.playing.as_ref())})
    }

    fn playing_json(&self, p: Option<&Playing>) -> Value {
        let Some(p) = p else { return Value::Null };
        let stops = self.m.stops(p.line.as_deref());
        let line_name = p.line.as_deref().and_then(|id| self.m.focus_line(id)).map(|l| l.name.clone());
        json!({
            "line": p.line, "lineName": line_name.unwrap_or_else(|| "全部焦点".into()),
            "index": p.at, "count": stops.len(),
            "focus": stops.get(p.at).map(|f| focus_json(f, self.m.focus.iter().position(|g| g.id == f.id).unwrap_or(0))),
        })
    }

    /// Keeps, changes or lets go a focus; or, given `line`, a line.
    pub(super) fn focus_set(&mut self, a: &Args) -> Result<Value, String> {
        if let Some(line) = a.get("line") {
            if a.keys().any(|k| k != "line") {
                return Err("line 要单独给：改焦点线时不能同时改焦点".into());
            }
            return self.focus_line_set(line.as_object().ok_or("line 要是对象")?);
        }
        let id = arg_str(a, "id")?;
        if arg_bool(a, "delete")?.unwrap_or(false) {
            let id = id.ok_or("删除焦点要给 id")?;
            if !self.m.remove_focus(&id) {
                return Err(format!("没有 id 为 {id:?} 的焦点"));
            }
            self.dirty = true;
            return Ok(json!({"deleted": id, "count": self.m.focus.len()}));
        }
        // Where it looks: a block, a point, or the window's view now.
        let (vw, vh) = self.view;
        let (mut x, mut y, mut s) = match id.as_deref() {
            Some(id) => {
                let f = self.m.focus(id).ok_or_else(|| format!("没有 id 为 {id:?} 的焦点"))?;
                (f.x, f.y, f.s)
            }
            None => Focus::of_camera(self.m.cam, (vw, vh)),
        };
        let (px, py) = (arg_num(a, "x")?, arg_num(a, "y")?);
        if let Some(bid) = arg_str(a, "block")? {
            if px.is_some() || py.is_some() {
                return Err("block 与 x/y 只能给一种".into());
            }
            let b = self.block(&bid)?;
            (x, y) = (b.x + b.w / 2., b.y + b.h / 2.);
            if vw > 0. && b.w > 0. && b.h > 0. {
                s = (vw * 0.8 / b.w).min(vh * 0.8 / b.h);
            }
        }
        x = px.map_or(x, |v| v as f32);
        y = py.map_or(y, |v| v as f32);
        if let Some(v) = arg_num(a, "scale")? {
            if v <= 0. {
                return Err("scale 要是正数".into());
            }
            s = v as f32;
        }
        let s = s.clamp(MIN_SCALE, MAX_SCALE);
        let name = arg_str(a, "name")?;
        let id = match id {
            Some(id) => {
                let f = self.m.focus.iter_mut().find(|f| f.id == id).unwrap();
                (f.x, f.y, f.s) = (x, y, s);
                if let Some(n) = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
                    f.name = n;
                }
                id
            }
            None => self.m.add_focus(name, x, y, s),
        };
        if let Some(to) = arg_int(a, "index")? {
            let from = self.m.focus.iter().position(|f| f.id == id).unwrap();
            self.m.move_stop(None, from, to.max(0) as usize);
        }
        self.dirty = true;
        let i = self.m.focus.iter().position(|f| f.id == id).unwrap();
        Ok(focus_json(&self.m.focus[i], i))
    }

    fn focus_line_set(&mut self, a: &Args) -> Result<Value, String> {
        let id = arg_str(a, "id")?;
        if arg_bool(a, "delete")?.unwrap_or(false) {
            let id = id.ok_or("删除焦点线要给 id")?;
            if !self.m.remove_focus_line(&id) {
                return Err(format!("没有 id 为 {id:?} 的焦点线"));
            }
            self.dirty = true;
            return Ok(json!({"deleted": id}));
        }
        let stops = match a.get("stops") {
            Some(_) => {
                let stops = arg_strings(a, "stops")?;
                if let Some(bad) = stops.iter().find(|s| self.m.focus(s).is_none()) {
                    return Err(format!("stops 里没有 id 为 {bad:?} 的焦点"));
                }
                Some(stops)
            }
            None => None,
        };
        let name = arg_str(a, "name")?.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        let id = match id {
            Some(id) => {
                let l = self.m.focus_lines.iter_mut().find(|l| l.id == id).ok_or_else(|| format!("没有 id 为 {id:?} 的焦点线"))?;
                if let Some(n) = name {
                    l.name = n;
                }
                if let Some(s) = stops {
                    l.stops = s;
                }
                id
            }
            None => {
                let id = self.m.add_focus_line(name);
                self.m.focus_lines.last_mut().unwrap().stops = stops.unwrap_or_default();
                id
            }
        };
        self.dirty = true;
        let l = self.m.focus_line(&id).unwrap();
        Ok(json!({"id": l.id, "name": l.name, "stops": l.stops}))
    }

    /// Plays in the window: starts a line, steps it, goes to a stop, or stops.
    pub(super) fn focus_play(&mut self, a: &Args) -> Result<Value, String> {
        let action = arg_str(a, "action")?.unwrap_or_else(|| "start".into());
        let index = arg_int(a, "index")?;
        let now = self.playing.clone();
        let line = match action.as_str() {
            "start" => match arg_str(a, "line")?.filter(|l| !l.is_empty()) {
                Some(l) if self.m.focus_line(&l).is_none() => return Err(format!("没有 id 为 {l:?} 的焦点线")),
                l => l,
            },
            "stop" => {
                self.play = Some(PlayCmd::Stop);
                return Ok(json!({"playing": Value::Null, "stopped": now.is_some()}));
            }
            "next" | "prev" | "goto" => match &now {
                Some(p) => p.line.clone(),
                None if action == "goto" => arg_str(a, "line")?.filter(|l| !l.is_empty()),
                None => return Err("还没有在放映：先用 action=start".into()),
            },
            other => return Err(format!("没有动作 {other:?}：可用 start、next、prev、goto、stop")),
        };
        let count = self.m.stops(line.as_deref()).len();
        if count == 0 {
            return Err(if line.is_some() { "这条焦点线还没有焦点".into() } else { "还没有焦点：先用 focus_set 存几个".into() });
        }
        let at = now.as_ref().filter(|p| p.line == line).map_or(0, |p| p.at.min(count - 1));
        let to = match action.as_str() {
            "next" => (at + 1).min(count - 1),
            "prev" => at.saturating_sub(1),
            _ => match index {
                Some(i) if i < 0 || i as usize >= count => return Err(format!("index 要在 0 到 {} 之间", count - 1)),
                Some(i) => i as usize,
                None if action == "goto" => return Err("goto 要给 index".into()),
                None => 0,
            },
        };
        let p = Playing { line, at: to };
        let out = json!({"playing": self.playing_json(Some(&p)), "atEnd": to + 1 == count, "atStart": to == 0});
        self.play = Some(PlayCmd::Go(p));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::Here;
    use crate::model::{Block, BlockKind, Model};

    fn call(c: &mut Ctx, name: &str, v: Value) -> Result<Value, String> {
        c.call(name, v.as_object().unwrap())
    }

    #[test]
    fn agents_keep_foci_and_lines() {
        let mut m = Model::default();
        let mut b = Block::new(BlockKind::Note);
        (b.id, b.x, b.y, b.w, b.h) = ("b000001".into(), 100., 100., 400., 200.);
        m.add_block(b);
        let mut c = Ctx { m: &mut m, view: (1000., 800.), mcp: Value::Null, here: Here::default(), dirty: false, memos: vec![], reload: vec![], view_from: None, shot: None, process: None, playing: None, play: None };

        let here = call(&mut c, "focus_set", json!({})).unwrap();
        assert_eq!((here["name"].as_str(), here["x"].as_f64(), here["y"].as_f64()), (Some("焦点 1"), Some(500.), Some(400.)), "the view now");
        let on = call(&mut c, "focus_set", json!({"block": "b000001", "name": "笔记"})).unwrap();
        assert_eq!((on["x"].as_f64(), on["y"].as_f64(), on["scale"].as_f64()), (Some(300.), Some(200.), Some(2.)));
        let first = here["id"].as_str().unwrap().to_string();
        let second = on["id"].as_str().unwrap().to_string();
        call(&mut c, "focus_set", json!({"id": second, "index": 0, "scale": 0.5})).unwrap();
        assert_eq!(c.m.focus[0].id, second);
        assert_eq!(c.m.focus[0].s, 0.5);
        assert!(c.dirty);

        let line = call(&mut c, "focus_set", json!({"line": {"name": "讲解", "stops": [first, second, first]}})).unwrap();
        assert_eq!(line["stops"].as_array().unwrap().len(), 3);
        assert!(call(&mut c, "focus_set", json!({"line": {"stops": ["nope"]}})).is_err());
        assert!(call(&mut c, "focus_set", json!({"line": {}, "name": "x"})).is_err(), "a line is set on its own");

        let list = call(&mut c, "focus_list", json!({})).unwrap();
        assert_eq!((list["focus"].as_array().unwrap().len(), list["lines"][0]["name"].as_str()), (2, Some("讲解")));

        call(&mut c, "focus_set", json!({"id": first, "delete": true})).unwrap();
        assert_eq!(c.m.focus_lines[0].stops, std::slice::from_ref(&second), "gone from the line too");
    }

    #[test]
    fn playing_steps_through_a_line_and_stops() {
        let mut m = Model::default();
        let a = m.add_focus(None, 0., 0., 1.);
        let b = m.add_focus(None, 100., 0., 1.);
        let l = m.add_focus_line(None);
        m.focus_lines[0].stops = vec![b.clone(), a.clone(), b.clone()];
        let mut c = Ctx { m: &mut m, view: (1000., 800.), mcp: Value::Null, here: Here::default(), dirty: false, memos: vec![], reload: vec![], view_from: None, shot: None, process: None, playing: None, play: None };

        assert!(call(&mut c, "focus_play", json!({"action": "next"})).is_err(), "not playing yet");
        let out = call(&mut c, "focus_play", json!({"line": l})).unwrap();
        assert_eq!((out["playing"]["count"].as_u64(), out["playing"]["focus"]["id"].as_str()), (Some(3), Some(b.as_str())));
        let Some(PlayCmd::Go(p)) = c.play.take() else { panic!() };
        assert_eq!(p, Playing { line: Some(l.clone()), at: 0 });

        // The window plays on from where the tool said.
        c.playing = Some(Playing { line: Some(l.clone()), at: 2 });
        let out = call(&mut c, "focus_play", json!({"action": "next"})).unwrap();
        assert_eq!((out["playing"]["index"].as_u64(), out["atEnd"].as_bool()), (Some(2), Some(true)), "it stays at the end");
        call(&mut c, "focus_play", json!({"action": "prev"})).unwrap();
        assert_eq!(c.play, Some(PlayCmd::Go(Playing { line: Some(l.clone()), at: 1 })));
        assert!(call(&mut c, "focus_play", json!({"action": "goto", "index": 3})).is_err());
        call(&mut c, "focus_play", json!({"action": "stop"})).unwrap();
        assert_eq!(c.play, Some(PlayCmd::Stop));
        assert!(!c.dirty, "playing changes nothing saved");
    }
}
