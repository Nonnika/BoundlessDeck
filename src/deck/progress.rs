//! The task capsule: what an agent says of its work with the MCP tool
//! `process`, shown at the bottom middle of the window. 准备中 and 进行中
//! show a stepping spinner (and a bar when the agent says how far it is);
//! 已完成 shows a check for a moment, then the capsule goes.
//!
//! A task is the caller's, one per terminal it runs in; while several are
//! under way, the capsule shows the one told of last and how many others
//! there are. A task not told of for a long while is let go: its agent is
//! likely gone.
//!
//! The spinner steps, a few times a second, from a timer of its own: smooth
//! motion would draw the whole canvas every frame for as long as an agent
//! works. Only coming in and going out run at full frame rate.

use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, Hsla, IntoElement, ParentElement, Styled, canvas, div, point, prelude::FluentBuilder, px, relative, svg};

use super::{DeckView, sharpen};
use crate::ui::icons;
use crate::mcp::tools::{Report, Stage};
use crate::ui::theme::{Paint, Theme, paint};

/// How long a finished task stays, then how long it takes to go.
const LINGER: Duration = Duration::from_millis(1800);
const LEAVE: Duration = Duration::from_millis(260);
/// How long the capsule takes to come in.
const ENTER: Duration = Duration::from_millis(240);
/// A task not told of in this long is let go.
const STALE: Duration = Duration::from_secs(15 * 60);
/// The spinner's spokes, and how often it steps.
const SPOKES: usize = 8;
const STEP: Duration = Duration::from_millis(110);
/// The capsule's place: up from the window's bottom, its height, its widest.
const BOTTOM: f32 = 14.;
const HEIGHT: f32 = 36.;
const MAX_W: f32 = 520.;

pub(super) struct Task {
    key: String,
    from: Option<String>,
    stage: Stage,
    title: String,
    detail: String,
    progress: Option<f32>,
    /// When it was first shown, last told of, and finished.
    shown: Instant,
    told: Instant,
    done: Option<Instant>,
}

impl Task {
    /// How far in it is, 0 to 1, as it comes in and goes out.
    fn presence(&self, now: Instant) -> f32 {
        let enter = ((now - self.shown).as_secs_f32() / ENTER.as_secs_f32()).min(1.);
        let leave = self.done.map_or(1., |d| 1. - ((now - d).saturating_sub(LINGER).as_secs_f32() / LEAVE.as_secs_f32()).min(1.));
        enter.min(leave)
    }

    fn gone(&self, now: Instant) -> bool {
        self.done.is_some_and(|d| now - d >= LINGER + LEAVE) || (self.done.is_none() && now - self.told >= STALE)
    }
}

/// The tasks the window knows of, last told of last.
#[derive(Default)]
pub(super) struct Tasks {
    list: Vec<Task>,
    /// The spinner's timer is running.
    ticking: bool,
    /// Where the capsule covers the window this frame, for web pages under it to give way.
    pub(super) rect: Option<(f32, f32, f32, f32)>,
}

/// A report on a task the window may already know: what it leaves out stays as it was.
fn apply(list: &mut Vec<Task>, r: Report, now: Instant) {
    let at = list.iter().position(|t| t.key == r.key);
    // A task told of again after it finished is a new one.
    let fresh = at.is_none_or(|i| list[i].done.is_some());
    let mut t = match at {
        Some(i) => list.remove(i),
        None => Task {
            key: r.key.clone(),
            from: None,
            stage: r.stage,
            title: String::new(),
            detail: String::new(),
            progress: None,
            shown: now,
            told: now,
            done: None,
        },
    };
    if fresh && at.is_some() {
        t.shown = now;
        t.detail.clear();
        t.progress = None;
    }
    t.from = r.from.or(t.from);
    t.stage = r.stage;
    if let Some(title) = r.title {
        t.title = title;
    }
    if let Some(d) = r.detail {
        t.detail = d;
    }
    t.progress = match r.stage {
        Stage::Done => Some(1.),
        _ => r.progress.or(t.progress),
    };
    t.told = now;
    t.done = (r.stage == Stage::Done).then_some(now);
    list.push(t);
}

impl DeckView {
    /// A report from `process`: the capsule shows it, and keeps time while there is a task.
    pub(super) fn report_task(&mut self, r: Report, cx: &mut Context<Self>) {
        apply(&mut self.tasks.list, r, Instant::now());
        self.start_task_timer(cx);
        cx.notify();
    }

    /// Redraws a few times a second while there is a task: the spinner steps, and finished ones go.
    fn start_task_timer(&mut self, cx: &mut Context<Self>) {
        if self.tasks.ticking {
            return;
        }
        self.tasks.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STEP).await;
                let go_on = this
                    .update(cx, |d, cx| {
                        let any = !d.tasks.list.is_empty();
                        d.tasks.ticking = any;
                        if any {
                            cx.notify();
                        }
                        any
                    })
                    .unwrap_or(false);
                if !go_on {
                    break;
                }
            }
        })
        .detach();
    }

    /// Before the blocks are drawn: lets finished and forgotten tasks go, and
    /// says where the capsule will be.
    pub(super) fn tasks_tick(&mut self) {
        let now = Instant::now();
        self.tasks.list.retain(|t| !t.gone(now));
        self.tasks.rect = None;
        let Some(t) = self.tasks.list.last() else { return };
        let p = t.presence(now);
        if p < 1. && !sharpen::reduce_motion() {
            self.animating = true;
        }
        let (vw, vh) = self.view;
        let w = MAX_W.min(vw - 32.);
        self.tasks.rect = Some(((vw - w) / 2., vh - BOTTOM - HEIGHT, w, HEIGHT));
    }

    /// The capsule, for the task told of last.
    pub(super) fn task_capsule(&self, t: &Theme) -> Option<AnyElement> {
        let task = self.tasks.list.last()?;
        let now = Instant::now();
        let p = if sharpen::reduce_motion() { if task.presence(now) > 0. { 1. } else { 0. } } else { task.presence(now) };
        let e = 1. - (1. - p).powi(3);
        let others = self.tasks.list.iter().rev().skip(1).filter(|o| o.done.is_none()).count();
        let green = match paint("green", false, t) {
            Paint::Color(c) => c,
            _ => t.accent,
        };
        let done = task.stage == Stage::Done;
        let lead: AnyElement = if done {
            div()
                .size(px(18.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(green)
                .child(svg().path(icons::path("check")).size(px(12.)).text_color(gpui::white()))
                .into_any_element()
        } else {
            let color = if task.stage == Stage::Running { t.accent } else { t.muted };
            let head = ((now - task.shown).as_millis() / STEP.as_millis()) as usize % SPOKES;
            spinner(head, color).into_any_element()
        };
        let title = if task.title.is_empty() { task.from.clone().unwrap_or_else(|| "Agent 任务".into()) } else { task.title.clone() };
        let stage_color = if done { green } else if task.stage == Stage::Running { t.accent } else { t.muted };
        let (vw, _) = self.view;
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(BOTTOM - (1. - e) * 18.))
                .flex()
                .justify_center()
                .opacity(e)
                .child(
                    div()
                        .relative()
                        .h(px(HEIGHT))
                        .max_w(px(MAX_W.min(vw - 32.)))
                        .min_w(px(200.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.))
                        .pl(px(10.))
                        .pr(px(14.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .shadow(vec![gpui::BoxShadow {
                            color: t.shade,
                            offset: point(px(0.), px(4.)),
                            blur_radius: px(16.),
                            spread_radius: px(0.),
                        }])
                        .text_size(px(12.5))
                        .whitespace_nowrap()
                        // How far along: a bar along the straight part of the capsule's bottom edge
                        // (what it holds is cut square, not to its round ends).
                        .when_some(task.progress.filter(|_| !done), |d, f| {
                            d.child(
                                div()
                                    .absolute()
                                    .left(px(HEIGHT / 2.))
                                    .right(px(HEIGHT / 2.))
                                    .bottom_0()
                                    .h(px(2.))
                                    .child(div().h_full().w(relative(f.clamp(0., 1.))).rounded_full().bg(t.accent.opacity(0.8))),
                            )
                        })
                        .child(lead)
                        .child(div().flex_none().font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child(title))
                        .when(!task.detail.is_empty(), |d| {
                            d.child(div().min_w_0().flex_shrink().overflow_hidden().text_ellipsis().text_color(t.muted).child(task.detail.clone()))
                        })
                        .child(div().flex_1())
                        .when_some(task.progress.filter(|_| !done), |d, f| {
                            d.child(div().flex_none().text_color(t.muted).child(format!("{:.0}%", f * 100.)))
                        })
                        .child(div().flex_none().text_size(px(11.5)).text_color(stage_color).child(task.stage.label()))
                        .when(others > 0, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .px(px(6.))
                                    .rounded_full()
                                    .bg(t.surface_hover)
                                    .text_size(px(11.))
                                    .text_color(t.muted)
                                    .child(format!("+{others}")),
                            )
                        }),
                )
                .into_any_element(),
        )
    }
}

/// The stepping spinner: its spokes fainter the further behind `head` they are.
fn spinner(head: usize, color: Hsla) -> impl IntoElement {
    canvas(|_, _, _| (), move |b, _, window, _| {
        let (cx, cy) = (f32::from(b.center().x), f32::from(b.center().y));
        let r = f32::from(b.size.width) / 2.;
        for i in 0..SPOKES {
            let behind = (head + SPOKES - i) % SPOKES;
            let a = i as f32 / SPOKES as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let (s, c) = a.sin_cos();
            let mut p = gpui::PathBuilder::stroke(px(1.8));
            p.move_to(point(px(cx + c * r * 0.42), px(cy + s * r * 0.42)));
            p.line_to(point(px(cx + c * r * 0.92), px(cy + s * r * 0.92)));
            if let Ok(p) = p.build() {
                window.paint_path(p, color.opacity(1. - behind as f32 / SPOKES as f32 * 0.85));
            }
        }
    })
    .size(px(16.))
    .flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(key: &str, stage: Stage, title: Option<&str>, detail: Option<&str>) -> Report {
        Report { key: key.into(), from: None, stage, title: title.map(Into::into), detail: detail.map(Into::into), progress: None }
    }

    #[test]
    fn a_task_goes_from_ready_to_done_and_then_away() {
        let t0 = Instant::now();
        let mut list = Vec::new();
        apply(&mut list, report("b1", Stage::Preparing, Some("整理笔记"), None), t0);
        apply(&mut list, Report { progress: Some(0.4), ..report("b1", Stage::Running, None, Some("读文件")) }, t0);
        assert_eq!(list.len(), 1, "one task per caller");
        let t = &list[0];
        assert_eq!((t.stage, t.title.as_str(), t.detail.as_str(), t.progress), (Stage::Running, "整理笔记", "读文件", Some(0.4)));

        apply(&mut list, report("b2", Stage::Running, Some("另一个"), None), t0);
        assert_eq!(list.last().unwrap().key, "b2", "the one told of last shows");

        apply(&mut list, report("b1", Stage::Done, None, None), t0);
        let t = list.last().unwrap();
        assert_eq!((t.key.as_str(), t.stage, t.progress), ("b1", Stage::Done, Some(1.)));
        assert!(!t.gone(t0 + LINGER) && t.gone(t0 + LINGER + LEAVE), "it stays a moment, then goes");
        assert!(t.presence(t0 + LINGER + LEAVE / 2) < 1.);

        // Told of again, a finished task starts over.
        apply(&mut list, report("b1", Stage::Preparing, Some("下一项"), None), t0 + LINGER);
        let t = list.last().unwrap();
        assert_eq!((t.done, t.title.as_str(), t.detail.as_str(), t.progress), (None, "下一项", "", None));
    }

    #[test]
    fn a_task_not_told_of_for_long_is_let_go() {
        let t0 = Instant::now();
        let mut list = Vec::new();
        apply(&mut list, report("agent", Stage::Running, Some("x"), None), t0);
        assert!(!list[0].gone(t0 + STALE / 2));
        assert!(list[0].gone(t0 + STALE));
    }
}
