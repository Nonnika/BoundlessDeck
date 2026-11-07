//! The browser block: a native WKWebView (through wry) laid over the
//! block's body, under a bar of back/forward/reload and the address.
//!
//! A native view always draws over the canvas, so the page shows only
//! while nothing of the canvas should be over it: not while another block,
//! the chrome or an overlay covers it, not while a zoom goes on (WebKit's
//! page zoom lags behind the block). Meanwhile its body says why. Once the
//! zoom rests, the page takes it (`WebView::zoom`), crisp at any scale.

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, svg,
};
use wry::dpi::{LogicalPosition, LogicalSize};

use super::{DeckView, Drag, EditPart, TITLE_H};
use crate::icons;
use crate::model::{Block, BlockKind, LOD_SCALE, new_block_of};
use crate::theme::Theme;

/// The bar's height at 100%.
pub(super) const BAR_H: f32 = 34.0;

pub(super) struct WebRt {
    view: wry::WebView,
    /// The page zoom it was last given.
    zoom: f32,
    bounds: (f32, f32, f32, f32),
    visible: bool,
    pub loading: bool,
}

pub(super) enum WebEvent {
    Title(String, String),
    Loading(String, bool, String),
    NewWindow(String, String),
    /// A link to what a browser does not show: the system opens it.
    External(String),
}

/// The schemes a page may go to; others are the system's.
fn web_scheme(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http:", "https:", "about:", "file:", "blob:", "data:"].iter().any(|s| lower.starts_with(s))
}

/// What the address bar's text means: an address, or words to search for.
pub fn address_to_url(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(String::new());
    }
    // Scripts in the address bar are refused, never searched for.
    crate::mcp::tools::normalize_url(t)?;
    let looks_like_address = t.contains("://")
        || t.to_ascii_lowercase().starts_with("about:")
        || (!t.contains(char::is_whitespace) && (t.contains('.') || t.starts_with("localhost")));
    if looks_like_address {
        return crate::mcp::tools::normalize_url(t);
    }
    let q: String = t
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    Ok(format!("https://www.bing.com/search?q={q}"))
}

fn bar_button(id: SharedString, icon: &'static str, s: f32, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(24. * s))
        .rounded(px(6. * s))
        .hover(|d| d.bg(t.surface_hover))
        .cursor_pointer()
        .child(svg().path(icons::path(icon)).size(px(14. * s)).text_color(t.muted))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

impl DeckView {
    pub(super) fn on_web_event(&mut self, ev: WebEvent, cx: &mut Context<Self>) {
        match ev {
            WebEvent::Title(id, title) => {
                if let Some(b) = self.m.block_mut(&id) {
                    b.auto_title(&title);
                }
            }
            WebEvent::Loading(id, on, url) => {
                if let Some(rt) = self.rts.get_mut(&id).and_then(|r| r.web.as_mut()) {
                    rt.loading = on;
                }
                if !on
                    && !url.is_empty()
                    && let Some(b) = self.m.block_mut(&id)
                    && b.url != url
                {
                    b.url = url;
                    self.request_save(cx);
                }
            }
            WebEvent::NewWindow(id, url) => {
                // A link to a new window opens a browser block beside its own.
                let Some(src) = self.m.block(&id).cloned() else { return };
                let mut b = new_block_of(BlockKind::Browser, self.m.now_ms());
                b.url = url;
                (b.x, b.y) = self.m.free_spot(src.x + src.w + 28., src.y, b.w, b.h);
                self.m.add_block(b);
                self.after_edit(cx);
            }
            WebEvent::External(url) => cx.open_url(&url),
        }
        cx.notify();
    }

    fn make_webview(&self, id: &str, url: &str, window: &Window) -> Result<wry::WebView, String> {
        let tx = self.web_tx.clone();
        let (t1, t2, t3, t4) = (tx.clone(), tx.clone(), tx.clone(), tx);
        let (i1, i2, i3) = (id.to_string(), id.to_string(), id.to_string());
        wry::WebViewBuilder::new()
            .with_url(url)
            .with_bounds(wry::Rect {
                position: LogicalPosition::new(0., 0.).into(),
                size: LogicalSize::new(10., 10.).into(),
            })
            .with_visible(false)
            .with_accept_first_mouse(true)
            .with_devtools(true)
            .with_navigation_handler(move |url| {
                if web_scheme(&url) {
                    return true;
                }
                let _ = t1.try_send(WebEvent::External(url));
                false
            })
            .with_new_window_req_handler(move |url, _| {
                let _ = t2.try_send(WebEvent::NewWindow(i1.clone(), url));
                wry::NewWindowResponse::Deny
            })
            .with_document_title_changed_handler(move |title| {
                let _ = t3.try_send(WebEvent::Title(i2.clone(), title));
            })
            .with_on_page_load_handler(move |ev, url| {
                let on = matches!(ev, wry::PageLoadEvent::Started);
                let _ = t4.try_send(WebEvent::Loading(i3.clone(), on, url));
            })
            .build_as_child(window)
            .map_err(|e| e.to_string())
    }

    /// Navigates the block's page, as the address bar or an agent asks.
    pub(super) fn web_go(&mut self, id: &str, url: &str) {
        if let Some(web) = self.rts.get_mut(id).and_then(|r| r.web.as_mut()) {
            if url.is_empty() {
                let _ = web.view.load_url("about:blank");
            } else {
                let _ = web.view.load_url(url);
            }
        }
    }

    /// Whether a zoom goes on, or has just rested and still glides.
    fn zooming(&self) -> bool {
        self.zoomed_at.is_some_and(|t| t.elapsed() < Duration::from_millis(400))
    }

    /// Why the page cannot show now over its body, if it cannot.
    fn web_hidden(&self, id: &str, r: (f32, f32, f32, f32)) -> Option<&'static str> {
        if self.preview.is_some() || self.popup.is_some() || self.pluck.as_ref().is_some_and(|p| p.active) {
            return Some("被浮层遮住");
        }
        if matches!(self.drag, Some(Drag::Marquee { .. }) | Some(Drag::Pan { .. })) {
            return Some("");
        }
        if self.zooming() {
            return Some("");
        }
        let over = |a: (f32, f32, f32, f32)| a.0 < r.0 + r.2 && a.0 + a.2 > r.0 && a.1 < r.1 + r.3 && a.1 + a.3 > r.1;
        // The blocks drawn after it lie over it.
        let mut after = false;
        for (other, rect) in &self.frame_rects {
            if other == id {
                after = true;
                continue;
            }
            if after && over(*rect) {
                return Some("被其他 block 遮住 · 点它的胶带把它放到最上面");
            }
        }
        // The chrome: the workspace pill, the toolbar, the status bar.
        let (vw, vh) = self.view;
        for chrome in [(80., 0., 300., 56.), (vw - 560., 0., 560., 56.), (0., vh - 48., 520., 48.)] {
            if over(chrome) {
                return Some("被工具栏遮住 · 平移画布让它露出来");
            }
        }
        None
    }

    pub(super) fn browser_body(&mut self, b: &Block, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        let (sx, sy) = self.m.cam.to_screen(b.x, b.y);
        let page = (sx + 1., sy + (TITLE_H + BAR_H) * s, b.w * s - 2., (b.h - TITLE_H - BAR_H) * s - 1.);
        let rt = self.rts.entry(b.id.clone()).or_default();
        let loading = rt.web.as_ref().is_some_and(|w| w.loading);
        // The page: made once there is an address, shown where nothing is over it.
        let mut note: Option<String> = None;
        if b.url.is_empty() {
            rt.web = None;
            note = Some("在地址栏输入网址或要搜索的词".into());
        } else if s < LOD_SCALE {
            note = Some(String::new());
        } else {
            if self.rts.get(&b.id).is_some_and(|r| r.web.is_none() && r.web_err.is_none()) {
                let made = self.make_webview(&b.id, &b.url, window);
                let rt = self.rts.entry(b.id.clone()).or_default();
                match made {
                    Ok(view) => {
                        rt.web = Some(WebRt { view, zoom: 1., bounds: (0., 0., 0., 0.), visible: false, loading: true })
                    }
                    Err(e) => rt.web_err = Some(format!("网页视图没能创建：{e}")),
                }
            }
            let hidden = self.web_hidden(&b.id, page);
            let zooming = self.zooming();
            let rt = self.rts.entry(b.id.clone()).or_default();
            if let Some(e) = &rt.web_err {
                note = Some(e.clone());
            } else if let Some(web) = rt.web.as_mut() {
                match hidden {
                    None => {
                        if web.bounds != page {
                            let _ = web.view.set_bounds(wry::Rect {
                                position: LogicalPosition::new(page.0 as f64, page.1 as f64).into(),
                                size: LogicalSize::new(page.2.max(1.) as f64, page.3.max(1.) as f64).into(),
                            });
                            web.bounds = page;
                        }
                        if !zooming && (web.zoom - s).abs() > 1e-3 {
                            let _ = web.view.zoom(s as f64);
                            web.zoom = s;
                        }
                        if !web.visible {
                            let _ = web.view.set_visible(true);
                            web.visible = true;
                        }
                        self.web_shown.insert(b.id.clone());
                    }
                    Some(why) => note = Some(why.to_string()),
                }
            }
        }

        let id = b.id.clone();
        let editing = self.editing(&b.id, EditPart::Address).cloned();
        let address: AnyElement = match editing {
            Some(ed) => {
                ed.update(cx, |e, _| {
                    e.font_size = 12. * s;
                    e.color = t.text;
                    e.muted = t.muted;
                    e.accent = t.accent;
                });
                div().flex_1().min_w_0().child(ed).into_any_element()
            }
            None => {
                let shown = if b.url.is_empty() { "输入网址或搜索".to_string() } else { b.url.clone() };
                let (i, muted) = (id.clone(), b.url.is_empty());
                div()
                    .id(SharedString::from(format!("address:{}", b.id)))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .cursor_text()
                    .text_color(if muted { t.muted } else { t.text })
                    .child(shown)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |d, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            d.m.sel.clear();
                            d.start_editing(&i, EditPart::Address, window, cx);
                        }),
                    )
                    .into_any_element()
            }
        };
        let (i1, i2, i3) = (id.clone(), id.clone(), id.clone());
        let bar = div()
            .h(px(BAR_H * s))
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4. * s))
            .px(px(6. * s))
            .border_b_1()
            .border_color(t.border)
            .child(bar_button(SharedString::from(format!("back:{id}")), "arrow_left", s, t).on_click(cx.listener(
                move |d, _, _, _| {
                    if let Some(w) = d.rts.get(&i1).and_then(|r| r.web.as_ref()) {
                        let _ = w.view.evaluate_script("history.back()");
                    }
                },
            )))
            .child(bar_button(SharedString::from(format!("fwd:{id}")), "arrow_right", s, t).on_click(cx.listener(
                move |d, _, _, _| {
                    if let Some(w) = d.rts.get(&i2).and_then(|r| r.web.as_ref()) {
                        let _ = w.view.evaluate_script("history.forward()");
                    }
                },
            )))
            .child(bar_button(SharedString::from(format!("reload:{id}")), if loading { "x" } else { "reload" }, s, t).on_click(
                cx.listener(move |d, _, _, _| {
                    if let Some(w) = d.rts.get(&i3).and_then(|r| r.web.as_ref()) {
                        let _ = if w.loading { w.view.evaluate_script("window.stop()") } else { w.view.reload() };
                    }
                }),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(24. * s))
                    .flex()
                    .items_center()
                    .px(px(10. * s))
                    .rounded(px(12. * s))
                    .bg(t.surface_hover)
                    .text_size(px(12. * s))
                    .child(address),
            );
        let backdrop: Hsla = if t.dark { gpui::rgb(0x1a1b1f).into() } else { gpui::rgb(0xf3f4f6).into() };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(6. * s))
                    .bg(backdrop)
                    .text_size(px(12. * s))
                    .text_color(t.muted)
                    .when_some(note.filter(|n| !n.is_empty()), |d, n| {
                        d.child(svg().path(icons::path("globe")).size(px(28. * s)).text_color(t.muted)).child(n)
                    }),
            )
            .into_any_element()
    }

    /// Hides the pages this frame did not show: out of view, covered, gone.
    pub(super) fn hide_unshown_pages(&mut self) {
        let shown = std::mem::take(&mut self.web_shown);
        for (id, rt) in self.rts.iter_mut() {
            if let Some(web) = rt.web.as_mut()
                && web.visible
                && !shown.contains(id)
            {
                let _ = web.view.set_visible(false);
                web.visible = false;
            }
        }
    }

    /// Notes that the canvas zoomed now, for `zooming`; a frame comes once the zoom rests.
    pub(super) fn note_zoom(&mut self, cx: &mut Context<Self>) {
        self.zoomed_at = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(450)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_bar_reads_addresses_and_searches() {
        assert_eq!(address_to_url("example.com").unwrap(), "https://example.com");
        assert_eq!(address_to_url("http://x").unwrap(), "http://x");
        assert_eq!(address_to_url("localhost:3000").unwrap(), "https://localhost:3000");
        assert_eq!(address_to_url("rust gpui").unwrap(), "https://www.bing.com/search?q=rust+gpui");
        assert!(address_to_url("你好").unwrap().contains("q=%E4%BD%A0"));
        assert!(address_to_url("javascript:alert(1)").is_err());
        assert!(web_scheme("https://a") && !web_scheme("mailto:a@b"));
    }
}
