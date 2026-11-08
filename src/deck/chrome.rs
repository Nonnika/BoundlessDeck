//! The chrome around the canvas: the toolbar, the home pill, popups and
//! the status bar.


use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    Stateful, StatefulInteractiveElement, Styled, div, px, svg,
};

use super::{CHROME_LEFT, CHROME_TOP, DeckEvent, DeckView, Popup, shadow};
use crate::icons;
use crate::lifespan::LIFE_CHOICES;
use crate::model::{self, BlockKind};
use crate::recents::short_path;
use crate::theme::Theme;

impl DeckView {
    pub(super) fn toolbar(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let item = |id: &'static str, icon: &'static str, label: &'static str, kind: BlockKind, cx: &mut Context<Self>| {
            tool_button(id, icon, label, t).on_click(cx.listener(move |d, _, window, cx| {
                let (x, y) = (d.view.0 / 2.0, d.view.1 / 2.0);
                d.add_kind_at(kind, x, y, window, cx);
            }))
        };
        div()
            .id("toolbar")
            .absolute()
            .top(px(CHROME_TOP))
            .right(px(12.))
            .flex()
            .flex_row()
            .gap(px(2.))
            .p(px(5.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(px(12.))
            .shadow(vec![shadow(t)])
            .occlude()
            .child(item("new-note", "note", "QuickLook", BlockKind::Note, cx))
            .child(item("new-memo", "memo", "笔记", BlockKind::Memo, cx))
            .child(item("new-file", "file", "文件", BlockKind::File, cx))
            .child(item("new-term", "terminal", "终端", BlockKind::Terminal, cx))
            .child(item("new-browser", "globe", "浏览器", BlockKind::Browser, cx))
            .child(item("new-chart", "chart", "图表", BlockKind::Chart, cx))
            .child(item("new-stack", "stack", "块堆", BlockKind::Stack, cx))
            .child(item("new-text", "type", "文字", BlockKind::Text, cx))
            .child(item("new-shape", "shapes", "图形", BlockKind::Shape, cx))
            .into_any_element()
    }

    pub(super) fn home_pill(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let name = model::file_name(&self.m.root.to_string_lossy());
        div()
            .id("home-pill")
            .absolute()
            .top(px(CHROME_TOP))
            .left(px(CHROME_LEFT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .p(px(5.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(px(12.))
            .shadow(vec![shadow(t)])
            .occlude()
            .child(
                div()
                    .id("go-home")
                    .px(px(7.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .hover(|d| d.bg(t.surface_hover))
                    .cursor_pointer()
                    .child(svg().path(icons::path("back")).size(px(15.)).text_color(t.text))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DeckEvent::Home))),
            )
            .child(
                div()
                    .id("workspace")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .hover(|d| d.bg(t.surface_hover))
                    .cursor_pointer()
                    .child(svg().path(icons::path("folder")).size(px(15.)).text_color(t.text))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(name))
                    .child(svg().path(icons::path("chevron_down")).size(px(13.)).text_color(t.muted))
                    .on_click(cx.listener(|d, _, _, cx| {
                        d.popup = match d.popup {
                            Some(Popup::Workspaces) => None,
                            _ => Some(Popup::Workspaces),
                        };
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(super) fn popup_view(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let popup = self.popup.clone()?;
        let menu = |x: f32, y: f32| {
            div()
                .id("popup")
                .absolute()
                .left(px(x))
                .top(px(y))
                .min_w(px(180.))
                .flex()
                .flex_col()
                .p(px(4.))
                .bg(t.surface)
                .border_1()
                .border_color(t.border)
                .rounded(px(8.))
                .shadow(vec![shadow(t)])
                .occlude()
                .text_size(px(13.))
                .text_color(t.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let row = |id: SharedString, label: String, checked: bool| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .gap(px(8.))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(5.))
                .hover(|d| d.bg(t.accent).text_color(t.accent_text))
                .cursor_pointer()
                .child(div().w(px(12.)).child(if checked { "✓" } else { "" }))
                .child(label)
        };
        Some(match popup {
            Popup::Workspaces => {
                let root = self.m.root.to_string_lossy().into_owned();
                let mut paths = self.workspaces.clone();
                if !paths.contains(&root) {
                    paths.insert(0, root.clone());
                }
                let mut m = menu(CHROME_LEFT, CHROME_TOP + 44.);
                for (i, p) in paths.into_iter().take(12).enumerate() {
                    let label = format!(
                        "{}    {}",
                        model::file_name(&p),
                        short_path(&std::path::Path::new(&p).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default())
                    );
                    let here = p == root;
                    m = m.child(row(SharedString::from(format!("ws:{i}")), label, here).on_click(cx.listener(
                        move |d, _, _, cx| {
                            d.popup = None;
                            if !here {
                                cx.emit(DeckEvent::Open(p.clone()));
                            }
                            cx.notify();
                        },
                    )));
                }
                m.child(div().h(px(1.)).my(px(4.)).bg(t.border))
                    .child(row("ws:pick".into(), "打开文件夹…".into(), false).on_click(cx.listener(|d, _, _, cx| {
                        d.popup = None;
                        cx.emit(DeckEvent::Pick);
                        cx.notify();
                    })))
                    .into_any_element()
            }
            Popup::Life { id, at } => {
                let expires = self.m.block(&id).map(|b| b.expires).unwrap_or(0);
                let mut m = menu(at.0 - 90., at.1);
                for (label, span) in LIFE_CHOICES {
                    let id = id.clone();
                    m = m.child(
                        row(SharedString::from(format!("life:{label}")), label.into(), span == 0 && expires == 0)
                            .on_click(cx.listener(move |d, _, _, cx| {
                                let now = d.m.now_ms();
                                if let Some(b) = d.m.block_mut(&id) {
                                    b.expires = if span <= 0 { 0 } else { now + span };
                                }
                                d.popup = None;
                                d.request_save(cx);
                                cx.notify();
                            })),
                    );
                }
                m.into_any_element()
            }
            Popup::ChartType { id, at } => {
                let now = self.m.block(&id).map(|b| crate::chart::of(&b.chart));
                let mut m = menu(at.0 - 60., at.1);
                for (kind, name, label, _) in crate::chart::TYPES {
                    let id = id.clone();
                    m = m.child(
                        row(SharedString::from(format!("chart:{name}")), label.into(), now == Some(kind)).on_click(
                            cx.listener(move |d, _, _, cx| {
                                if let Some(b) = d.m.block_mut(&id) {
                                    b.chart = name.to_string();
                                }
                                d.popup = None;
                                d.request_save(cx);
                                cx.notify();
                            }),
                        ),
                    );
                }
                m.into_any_element()
            }
        })
    }

    pub(super) fn status_bar(&self, t: &Theme) -> AnyElement {
        div()
            .absolute()
            .left(px(12.))
            .bottom(px(10.))
            .flex()
            .flex_row()
            .gap(px(10.))
            .px(px(8.))
            .py(px(4.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(px(9.))
            .text_size(px(11.))
            .text_color(t.muted)
            .child(format!("{:.0}%", self.m.cam.s * 100.))
            .child(format!("{} 个 block · 选中 {}", self.m.blocks.len(), self.m.sel.len()))
            .child(match (&self.mcp, &self.mcp_err) {
                (Some(_), None) => div().child(format!("MCP · {} 个工具", crate::mcp::tools::tool_names().len())),
                (Some(_), Some(e)) => div().text_color(t.danger).child(format!("MCP · {e}")),
                (None, Some(e)) => div().text_color(t.danger).child(format!("MCP 未启动：{e}")),
                (None, None) => div(),
            })
            .into_any_element()
    }
}

fn tool_button(id: &'static str, icon: &'static str, label: &'static str, t: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(8.))
        .hover(|d| d.bg(t.surface_hover))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(t.text)
        .child(svg().path(icons::path(icon)).size(px(15.)).text_color(t.text))
        .child(label)
}
