//! 焦点: views of the canvas kept to come back to, played in order like
//! slides. A focus is where a view is centered, in world coordinates, and
//! its scale; kept by its center, it lands the same in a window of any size.
//!
//! The deck's foci in their list's order are its first line, played when no
//! other is chosen. A line of its own (焦点线) goes through some of them in
//! an order of its own, and may come back to one (the whole, a detail, the
//! whole again). deck.json keeps them at its top level, as `focus` and
//! `focusLines`; the Go build leaves them be.

use serde::{Deserialize, Serialize};

use super::{Camera, MAX_SCALE, MIN_SCALE, Model};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Focus {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The view's center, in world coordinates.
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default = "one")]
    pub s: f32,
}

fn one() -> f32 {
    1.
}

impl Focus {
    /// The camera that shows this focus in a view vw by vh.
    pub fn camera(&self, (vw, vh): (f32, f32)) -> Camera {
        let s = self.s.clamp(MIN_SCALE, MAX_SCALE);
        Camera { x: self.x - vw / 2. / s, y: self.y - vh / 2. / s, s }
    }

    /// The focus a camera shows in a view vw by vh: where its center is, and its scale.
    pub fn of_camera(cam: Camera, (vw, vh): (f32, f32)) -> (f32, f32, f32) {
        let (x, y) = cam.to_world(vw / 2., vh / 2.);
        (x, y, cam.s)
    }
}

/// A line through the foci: their IDs, in the order played.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusLine {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub stops: Vec<String>,
}

impl Model {
    pub fn focus(&self, id: &str) -> Option<&Focus> {
        self.focus.iter().find(|f| f.id == id)
    }

    pub fn focus_line(&self, id: &str) -> Option<&FocusLine> {
        self.focus_lines.iter().find(|l| l.id == id)
    }

    /// A new ID, `prefix` and 6 hex digits, used by no focus or line.
    fn new_focus_id(&self, prefix: char) -> String {
        loop {
            let id = format!("{prefix}{:06x}", rand::random::<u32>() & 0xff_ffff);
            if self.focus(&id).is_none() && self.focus_line(&id).is_none() {
                return id;
            }
        }
    }

    /// A name `{base} N` that no other has, N counting on from how many there are.
    fn next_name<'a>(base: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
        let mut n = taken.clone().count() + 1;
        loop {
            let name = format!("{base} {n}");
            if !taken.clone().any(|t| t == name) {
                return name;
            }
            n += 1;
        }
    }

    /// Keeps a view as a new focus, last in the list; with no name, `焦点 N`. Returns its ID.
    pub fn add_focus(&mut self, name: Option<String>, x: f32, y: f32, s: f32) -> String {
        let id = self.new_focus_id('f');
        let name = name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| Self::next_name("焦点", self.focus.iter().map(|f| f.name.as_str())));
        self.focus.push(Focus { id: id.clone(), name, x, y, s: s.clamp(MIN_SCALE, MAX_SCALE) });
        id
    }

    /// Lets a focus go, and its stops on every line.
    pub fn remove_focus(&mut self, id: &str) -> bool {
        let before = self.focus.len();
        self.focus.retain(|f| f.id != id);
        for l in &mut self.focus_lines {
            l.stops.retain(|s| s != id);
        }
        self.focus.len() != before
    }

    /// Moves the stop at `from` of a line (none: the list itself) to `to`, the others making way.
    pub fn move_stop(&mut self, line: Option<&str>, from: usize, to: usize) -> bool {
        fn shift<T>(v: &mut Vec<T>, from: usize, to: usize) -> bool {
            if from >= v.len() {
                return false;
            }
            let to = to.min(v.len() - 1);
            if from == to {
                return false;
            }
            let x = v.remove(from);
            v.insert(to, x);
            true
        }
        match line {
            None => shift(&mut self.focus, from, to),
            Some(id) => self.focus_lines.iter_mut().find(|l| l.id == id).is_some_and(|l| shift(&mut l.stops, from, to)),
        }
    }

    /// A new, empty line, `焦点线 N` unless named. Returns its ID.
    pub fn add_focus_line(&mut self, name: Option<String>) -> String {
        let id = self.new_focus_id('l');
        let name = name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| Self::next_name("焦点线", self.focus_lines.iter().map(|l| l.name.as_str())));
        self.focus_lines.push(FocusLine { id: id.clone(), name, stops: vec![] });
        id
    }

    pub fn remove_focus_line(&mut self, id: &str) -> bool {
        let before = self.focus_lines.len();
        self.focus_lines.retain(|l| l.id != id);
        self.focus_lines.len() != before
    }

    /// A line's foci in the order played; none is the list itself.
    pub fn stops(&self, line: Option<&str>) -> Vec<&Focus> {
        match line.and_then(|id| self.focus_line(id)) {
            Some(l) => l.stops.iter().filter_map(|id| self.focus(id)).collect(),
            None if line.is_some() => vec![],
            None => self.focus.iter().collect(),
        }
    }

    /// After reading deck.json: stops on foci that are not there are let go.
    pub(super) fn prune_focus(&mut self) {
        let ids: std::collections::HashSet<String> = self.focus.iter().map(|f| f.id.clone()).collect();
        for l in &mut self.focus_lines {
            l.stops.retain(|s| ids.contains(s));
        }
        for f in &mut self.focus {
            if !(f.s.is_finite() && f.s > 0.) {
                f.s = 1.;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foci_are_kept_named_and_reordered() {
        let mut m = Model::default();
        let a = m.add_focus(None, 0., 0., 1.);
        let b = m.add_focus(Some(" 细节 ".into()), 500., 300., 2.);
        let c = m.add_focus(None, -100., 0., 100.);
        let names: Vec<&str> = m.focus.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["焦点 1", "细节", "焦点 3"]);
        assert_eq!(m.focus(&c).unwrap().s, MAX_SCALE, "kept within the scales the camera has");
        assert!(m.move_stop(None, 2, 0));
        assert_eq!(m.stops(None).iter().map(|f| f.id.clone()).collect::<Vec<_>>(), [c.clone(), a.clone(), b.clone()]);
        assert!(!m.move_stop(None, 5, 0));

        // A line goes through some of them, coming back to one.
        let l = m.add_focus_line(None);
        m.focus_lines[0].stops = vec![a.clone(), b.clone(), a.clone()];
        assert_eq!(m.focus_lines[0].name, "焦点线 1");
        assert_eq!(m.stops(Some(&l)).len(), 3);
        assert!(m.move_stop(Some(&l), 0, 1));
        assert_eq!(m.focus_lines[0].stops, [b.clone(), a.clone(), a.clone()]);
        assert!(m.stops(Some("nope")).is_empty());

        // A focus let go leaves every line.
        assert!(m.remove_focus(&a));
        assert_eq!(m.focus_lines[0].stops, std::slice::from_ref(&b));
        assert!(m.remove_focus_line(&l) && m.focus_lines.is_empty());
    }

    #[test]
    fn foci_are_saved_at_the_top_and_what_is_unknown_stays() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deck.json");
        std::fs::write(
            &file,
            r#"{"version":1,"root":"","camX":0,"camY":0,"camS":1,"blocks":[],"later":{"k":1},
               "focus":[{"id":"f000001","name":"全局","x":10,"y":20,"s":0.5},{"id":"f000002","x":1,"y":2,"s":0}],
               "focusLines":[{"id":"l000001","name":"讲解","stops":["f000001","gone","f000002","f000001"]}]}"#,
        )
        .unwrap();
        let mut m = Model::new(dir.path());
        m.file = Some(file.clone());
        m.load().unwrap();
        assert_eq!(m.focus.len(), 2);
        assert_eq!(m.focus[1].s, 1., "a scale that cannot be is 1");
        assert_eq!(m.focus_lines[0].stops, ["f000001", "f000002", "f000001"], "stops on foci not there go");
        m.save().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(v["later"]["k"], 1, "the top level's unknown fields are written back");
        assert_eq!(v["version"], 1);
        assert_eq!(v["focus"][0]["name"], "全局");
        assert_eq!(v["focusLines"][0]["stops"].as_array().unwrap().len(), 3);

        // With none, deck.json says nothing of them.
        m.focus.clear();
        m.focus_lines.clear();
        m.save().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(v.get("focus").is_none() && v.get("focusLines").is_none());
    }

    #[test]
    fn a_focus_lands_the_same_in_any_window() {
        let f = Focus { id: "f1".into(), name: String::new(), x: 1000., y: 500., s: 2. };
        for view in [(1280., 800.), (800., 600.)] {
            let cam = f.camera(view);
            assert_eq!(cam.to_world(view.0 / 2., view.1 / 2.), (1000., 500.));
            assert_eq!(Focus::of_camera(cam, view), (1000., 500., 2.));
        }
    }
}
