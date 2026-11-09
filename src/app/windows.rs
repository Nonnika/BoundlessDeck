//! The app's small windows: 键盘快捷键 (read from the commands table) and
//! 设置 (the accent, the welcome again, the global MCP per agent, the
//! project-level config switch).

use std::path::PathBuf;

use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, Focusable, FontWeight, Global, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, TitlebarOptions,
    WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, div, prelude::FluentBuilder, px, size,
};

use crate::app::Root;
use crate::app::commands::{self, Group};
use crate::mcp::config::{self, Agent, State};
use crate::app::recents::short_path;
use crate::ui::theme::Theme;

/// The main window's root, for the small windows to reach the deck open.
pub struct RootHandle(pub WeakEntity<Root>);
impl Global for RootHandle {}

#[derive(Default)]
struct OpenWindows {
    shortcuts: Option<WindowHandle<ShortcutsView>>,
    settings: Option<WindowHandle<SettingsView>>,
}
impl Global for OpenWindows {}

pub(crate) fn options(title: &str, w: f32, h: f32, cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(w), px(h)), cx))),
        titlebar: Some(TitlebarOptions { title: Some(title.to_string().into()), appears_transparent: false, traffic_light_position: None }),
        is_minimizable: false,
        window_min_size: Some(size(px(380.), px(320.))),
        ..Default::default()
    }
}

fn green(t: &Theme) -> Hsla {
    if t.dark { gpui::rgb(0x3fbf7a).into() } else { gpui::rgb(0x1f9d55).into() }
}

fn amber(t: &Theme) -> Hsla {
    if t.dark { gpui::rgb(0xe0a63a).into() } else { gpui::rgb(0xb7791f).into() }
}

/// Esc closes a small window.
fn close_on_escape(ev: &KeyDownEvent, window: &mut Window, _: &mut App) {
    if ev.keystroke.key == "escape" {
        window.remove_window();
    }
}

// ---- 键盘快捷键 ----

pub struct ShortcutsView {
    focus: FocusHandle,
    groups: Vec<Group>,
}

impl Focusable for ShortcutsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn show_shortcuts(cx: &mut App) {
    let open = cx.default_global::<OpenWindows>().shortcuts;
    if let Some(w) = open
        && w.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let opts = options("键盘快捷键", 460., 640., cx);
    if let Ok(w) = cx.open_window(opts, |window, cx| {
        let v = cx.new(|cx| ShortcutsView { focus: cx.focus_handle(), groups: commands::shortcut_groups() });
        window.focus(&v.focus_handle(cx));
        v
    }) {
        cx.default_global::<OpenWindows>().shortcuts = Some(w);
    }
}

fn keycap(text: SharedString, t: &Theme) -> impl IntoElement {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(5.))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .text_size(px(12.))
        .text_color(t.text)
        .child(text)
}

impl Render for ShortcutsView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        div()
            .id("shortcuts")
            .track_focus(&self.focus)
            .on_key_down(close_on_escape)
            .size_full()
            .bg(t.background)
            .font_family(".SystemUIFont")
            .overflow_y_scroll()
            .child(div().flex().flex_col().gap(px(22.)).px(px(24.)).pt(px(18.)).pb(px(28.)).children(
                self.groups.iter().map(|(name, rows)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child(name.clone()))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .rounded(px(10.))
                                .border_1()
                                .border_color(t.border)
                                .bg(t.surface)
                                .overflow_hidden()
                                .children(rows.iter().enumerate().map(|(i, (what, how))| {
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(12.))
                                        .px(px(14.))
                                        .py(px(8.))
                                        .when(i > 0, |d| d.border_t_1().border_color(t.border.opacity(0.6)))
                                        .child(div().flex_1().text_size(px(13.)).text_color(t.text).child(what.clone()))
                                        .child(div().flex().flex_row().gap(px(4.)).children(how.iter().map(|h| keycap(h.clone(), &t))))
                                })),
                        )
                }),
            ))
    }
}

// ---- 设置 ----

pub struct SettingsView {
    focus: FocusHandle,
    exe: Result<String, String>,
    agents: Vec<Agent>,
    status: Vec<State>,
    note: Option<(&'static str, String, bool)>,
    active: Option<String>,
    project_root: Option<PathBuf>,
    project_has_us: bool,
    project_on: bool,
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn show_settings(cx: &mut App) {
    let open = cx.default_global::<OpenWindows>().settings;
    if let Some(w) = open
        && w
            .update(cx, |v, window, cx| {
                v.refresh(cx);
                window.activate_window();
            })
            .is_ok()
    {
        return;
    }
    let opts = options("设置", 560., 640., cx);
    if let Ok(w) = cx.open_window(opts, |window, cx| {
        let v = cx.new(|cx| {
            let mut v = SettingsView {
                focus: cx.focus_handle(),
                exe: config::install_exe(),
                agents: config::user_agents(),
                status: Vec::new(),
                note: None,
                active: None,
                project_root: None,
                project_has_us: false,
                project_on: false,
            };
            v.refresh(cx);
            v
        });
        window.focus(&v.focus_handle(cx));
        v
    }) {
        cx.default_global::<OpenWindows>().settings = Some(w);
    }
}

/// Tells an open settings window that the deck changed. The root calls this
/// while it is being updated, and the refresh reads the root, so it waits
/// until that update is over.
pub fn refresh_settings(cx: &mut App) {
    cx.defer(|cx| {
        if let Some(w) = cx.try_global::<OpenWindows>().and_then(|o| o.settings) {
            let _ = w.update(cx, |v, _, cx| v.refresh(cx));
        }
    });
}

pub(crate) fn root(cx: &App) -> Option<gpui::Entity<Root>> {
    cx.try_global::<RootHandle>().and_then(|h| h.0.upgrade())
}

impl SettingsView {
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let exe = self.exe.clone().unwrap_or_default();
        self.status = self.agents.iter().map(|a| a.status(&exe)).collect();
        self.active = crate::mcp::read_active().ok().map(|a| a.workspace);
        let (deck_root, on) = root(cx).map(|r| r.read(cx)).map(|r| (r.deck_root(cx), r.settings.project_mcp)).unwrap_or((None, false));
        self.project_has_us = deck_root.as_deref().is_some_and(config::project_has_us);
        self.project_root = deck_root;
        self.project_on = on;
        cx.notify();
    }

    fn agent_action(&mut self, i: usize, install: bool, cx: &mut Context<Self>) {
        let ag = self.agents[i].clone();
        let r = match (&self.exe, install) {
            (Ok(exe), true) => ag.install(exe),
            (Err(e), true) => Err(e.clone()),
            (_, false) => ag.uninstall(),
        };
        self.note = Some(match r {
            Err(e) => ("global", format!("{}：{e}", ag.label), true),
            Ok(()) if install => ("global", format!("已装到 {}。重启它（或在 Claude Code 里用 /mcp 重连）后生效。", ag.label), false),
            Ok(()) => ("global", format!("已从 {} 移除。", ag.label), false),
        });
        self.refresh(cx);
    }

    fn switch_project(&mut self, cx: &mut Context<Self>) {
        let on = !self.project_on;
        let r = root(cx).map(|r| r.update(cx, |r, cx| r.set_project_mcp(on, cx))).unwrap_or(Ok(()));
        self.note = r.err().map(|e| ("project", format!("项目级配置：{e}"), true));
        self.refresh(cx);
    }

    fn remove_project(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.project_root.clone() else { return };
        if let Some(r) = root(cx) {
            r.update(cx, |r, cx| r.set_project_mcp(false, cx)).ok();
        }
        let name = crate::model::file_name(&dir.to_string_lossy());
        self.note = Some(match config::remove_project_configs(&dir) {
            Ok(()) => ("project", format!("已从「{name}」移除项目级 MCP 配置。"), false),
            Err(e) => ("project", e, true),
        });
        self.refresh(cx);
    }

    fn note_view(&self, at: &str, t: &Theme) -> Option<impl IntoElement> {
        let (where_, text, err) = self.note.as_ref()?;
        (*where_ == at).then(|| {
            div().text_size(px(12.)).text_color(if *err { t.danger } else { green(t) }).child(text.clone())
        })
    }
}

fn section(title: &str, about: &str, t: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child(title.to_string()))
        .when(!about.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(t.muted).line_height(px(17.)).child(about.to_string())))
}

fn card(t: &Theme) -> gpui::Div {
    div().flex().flex_col().rounded(px(10.)).border_1().border_color(t.border).bg(t.surface).overflow_hidden()
}

fn button(id: SharedString, label: &str, enabled: bool, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(12.))
        .py(px(5.))
        .rounded(px(7.))
        .border_1()
        .border_color(t.border)
        .text_size(px(12.))
        .text_color(if enabled { t.text } else { t.muted })
        .when(enabled, |d| d.hover(|d| d.bg(t.surface_hover)).cursor_pointer())
        .child(label.to_string())
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        let exe_ok = self.exe.is_ok();
        let mut agents = card(&t);
        for (i, ag) in self.agents.iter().enumerate() {
            let st = self.status.get(i).cloned().unwrap_or(State::Absent);
            let (text, color) = match &st {
                State::Absent => ("这台电脑上没有找到".to_string(), t.muted),
                State::NotThere => ("未安装".to_string(), t.muted),
                State::Installed => ("已安装".to_string(), green(&t)),
                State::Elsewhere(c) => (format!("已安装，但指向另一个 BoundlessDeck：{c}"), amber(&t)),
                State::Broken(e) => (format!("配置文件读不了：{e}"), t.danger),
            };
            let action = match st {
                State::NotThere => Some(("安装", true)),
                State::Installed => Some(("卸载", false)),
                State::Elsewhere(_) => Some(("改为此应用", true)),
                _ => None,
            };
            agents = agents.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(10.))
                    .when(i > 0, |d| d.border_t_1().border_color(t.border.opacity(0.6)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().text_size(px(13.)).text_color(t.text).child(ag.label))
                            .child(div().text_size(px(11.)).text_color(color).child(text)),
                    )
                    .when_some(action, |d, (label, install)| {
                        let enabled = !install || exe_ok;
                        d.child(button(SharedString::from(format!("agent:{}", ag.id)), label, enabled, &t).when(enabled, |b| {
                            b.on_click(cx.listener(move |v, _, _, cx| v.agent_action(i, install, cx)))
                        }))
                    }),
            );
        }
        let exe_line = match &self.exe {
            Ok(exe) => div().text_size(px(11.)).text_color(t.muted).child(format!("命令：{exe} mcp-stdio")),
            Err(e) => div().text_size(px(12.)).text_color(t.danger).child(format!("暂时不能安装：{e}")),
        };
        let active = card(&t).child(div().flex().flex_col().gap(px(3.)).px(px(14.)).py(px(10.)).map(|d| match &self.active {
            None => d
                .child(div().text_size(px(13.)).text_color(t.text).child("没有打开的图版"))
                .child(div().text_size(px(11.)).text_color(t.muted).child("agent 此时调用工具，会收到「先在应用里打开一个文件夹」的提示。")),
            Some(ws) => d
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(crate::model::file_name(ws)))
                .child(div().text_size(px(11.)).text_color(t.muted).child(short_path(ws))),
        }));
        let on = self.project_on;
        let switch = div()
            .id("project-switch")
            .flex_none()
            .w(px(38.))
            .h(px(22.))
            .rounded_full()
            .bg(if on { t.accent } else { t.border })
            .cursor_pointer()
            .child(
                div()
                    .size(px(18.))
                    .mt(px(2.))
                    .ml(px(if on { 18. } else { 2. }))
                    .rounded_full()
                    .bg(gpui::white())
                    .shadow_sm(),
            )
            .on_click(cx.listener(|v, _, _, cx| v.switch_project(cx)));
        let mut project = card(&t).child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().text_size(px(13.)).text_color(t.text).child("打开文件夹时写入项目级 MCP 配置"))
                        .child(div().text_size(px(11.)).text_color(t.muted).line_height(px(16.)).child(
                            "在文件夹里写 .mcp.json、.cursor/mcp.json、.vscode/mcp.json，在里面启动的 agent 直接连这个图版。文件夹是代码仓库时容易被误提交，所以默认关闭。",
                        )),
                )
                .child(switch),
        );
        if let (Some(dir), true) = (&self.project_root, self.project_has_us) {
            project = project.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(t.border.opacity(0.6))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().text_size(px(13.)).text_color(t.text).child(format!(
                                "「{}」里有项目级配置",
                                crate::model::file_name(&dir.to_string_lossy())
                            )))
                            .child(div().text_size(px(11.)).text_color(t.muted).child("只删去 BoundlessDeck 这一项，文件里你自己的 server 保留。")),
                    )
                    .child(button("project-remove".into(), "移除", true, &t).on_click(cx.listener(|v, _, _, cx| v.remove_project(cx)))),
            );
        }
        // The accents, each a dot in its color for this appearance; the one in use ringed.
        let current = crate::ui::theme::accent().id;
        let swatches = div().flex().flex_row().flex_wrap().items_center().gap(px(6.)).children(
            crate::ui::theme::ACCENTS.iter().enumerate().map(|(i, a)| {
                let on = a.id == current;
                let color: Hsla = gpui::rgb(if t.dark { a.dark } else { a.light }).into();
                let id = a.id;
                div()
                    .id(("accent", i))
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_2()
                    .border_color(if on { color } else { gpui::transparent_black() })
                    .cursor_pointer()
                    .child(div().size(px(18.)).rounded_full().bg(color))
                    .on_click(|_, _, cx| {
                        if let Some(r) = root(cx) {
                            r.update(cx, |r, cx| r.set_accent(id, cx));
                        }
                    })
            }),
        );
        let appearance = card(&t).child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .child(div().flex_1().text_size(px(13.)).text_color(t.text).child("强调色"))
                .child(div().text_size(px(12.)).text_color(t.muted).child(crate::ui::theme::accent().name))
                .child(swatches),
        );
        let welcome = card(&t).child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().text_size(px(13.)).text_color(t.text).child("欢迎引导"))
                        .child(div().text_size(px(11.)).text_color(t.muted).child("在主窗口里再看一遍主要功能的介绍。")),
                )
                .child(button("welcome-again".into(), "重新打开", true, &t).on_click(|_, _, cx| crate::app::welcome::show(cx))),
        );
        div()
            .id("settings")
            .track_focus(&self.focus)
            .on_key_down(close_on_escape)
            .size_full()
            .bg(t.background)
            .font_family(".SystemUIFont")
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(26.))
                    .px(px(24.))
                    .pt(px(20.))
                    .pb(px(28.))
                    .child(section("外观", "", &t).child(appearance))
                    .child(section("欢迎使用", "", &t).child(welcome))
                    .child(
                        section(
                            "全局 MCP",
                            "装进 agent 的用户级配置后，在任何文件夹里启动的 agent 都能操作 BoundlessDeck 当前打开的图版，项目里不用放任何配置。",
                            &t,
                        )
                        .child(agents)
                        .children(self.note_view("global", &t))
                        .child(exe_line),
                    )
                    .child(section("当前图版", "", &t).child(active))
                    .child(section("项目级配置", "", &t).child(project).children(self.note_view("project", &t))),
            )
    }
}
