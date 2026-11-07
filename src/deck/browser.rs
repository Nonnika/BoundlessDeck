//! The browser block: a native WKWebView (through wry) laid over the
//! block's body, under a bar of back/forward/reload and the address.
//! It has tabs: each its own page, made when first opened and kept while
//! the deck is open; only the open tab's shows. A link that asks for a new
//! window opens a tab beside the one it was in.
//!
//! A native view always draws over the canvas, so the page shows only
//! while nothing of the canvas should be over it: not while another block,
//! the chrome or an overlay covers it, not while a zoom goes on (WebKit's
//! page zoom lags behind the block). Meanwhile its body says why. Once the
//! zoom rests, the page takes it (`WebView::zoom`), crisp at any scale,
//! fading in over its picture (see `sharpen`).

use std::time::Instant;

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, StyledImage, Window, div, prelude::FluentBuilder, px, svg,
};
use wry::dpi::{LogicalPosition, LogicalSize};

use super::{DeckView, EditPart, TITLE_H};
use crate::icons;
use crate::model::{Block, LOD_SCALE};
use crate::theme::Theme;

/// The bar's height at 100%.
pub(super) const BAR_H: f32 = 34.0;
/// The tab strip's height at 100%; it shows once there are two tabs.
pub(super) const TABS_H: f32 = 30.0;

pub(super) struct WebRt {
    view: wry::WebView,
    /// A picture of the page, shown in its place while it is hidden.
    pub snap: Option<std::sync::Arc<gpui::Image>>,
    snapping: bool,
    /// The page zoom it was last given.
    zoom: f32,
    /// The view's alpha as last set: below 1 while it fades in over its picture.
    alpha: f32,
    bounds: (f32, f32, f32, f32),
    visible: bool,
    pub loading: bool,
}

/// What a page says, with the block and the tab (by key) it is in.
pub(super) enum WebEvent {
    Title(String, u64, String),
    Loading(String, u64, bool, String),
    NewWindow(String, String),
    /// A link to what a browser does not show: the system opens it.
    External(String),
    /// A picture of the page, as PNG.
    Snapshot(String, u64, Vec<u8>),
}

/// Asks WebKit for a picture of the page as it shows now; it comes as an event.
#[cfg(target_os = "macos")]
fn take_snapshot(view: &wry::WebView, id: &str, key: u64, tx: &smol::channel::Sender<WebEvent>) {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use wry::WebViewExtMacOS;
    let (tx, id) = (tx.clone(), id.to_string());
    let handler = block2::RcBlock::new(move |img: *mut NSImage, _err: *mut NSError| {
        let Some(img) = (unsafe { img.as_ref() }) else { return };
        let png = img
            .TIFFRepresentation()
            .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
            .and_then(|rep| unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) });
        if let Some(png) = png {
            let _ = tx.try_send(WebEvent::Snapshot(id.clone(), key, png.to_vec()));
        }
    });
    let wk = view.webview();
    unsafe { wk.takeSnapshotWithConfiguration_completionHandler(None, &handler) };
}

/// Sets how opaque the page's native view is, from 0 to 1.
#[cfg(target_os = "macos")]
fn set_alpha(view: &wry::WebView, alpha: f32) {
    use wry::WebViewExtMacOS;
    view.webview().setAlphaValue(f64::from(alpha));
}

#[cfg(not(target_os = "macos"))]
fn set_alpha(_: &wry::WebView, _: f32) {}

#[cfg(not(target_os = "macos"))]
fn take_snapshot(_: &wry::WebView, _: &str, _: u64, _: &smol::channel::Sender<WebEvent>) {}

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
            WebEvent::Title(id, key, title) => {
                if let Some(b) = self.m.block_mut(&id)
                    && let Some(i) = b.tab_of(key)
                {
                    b.tabs[i].title = title.clone();
                    if i == b.tab {
                        b.auto_title(&title);
                    }
                }
            }
            WebEvent::Loading(id, key, on, url) => {
                let tx = self.web_tx.clone();
                if let Some(web) = self.rts.get_mut(&id).and_then(|r| r.webs.get_mut(&key)) {
                    web.loading = on;
                    // Loaded out of sight: a picture to show for it meanwhile.
                    if !on && !web.visible && !web.snapping {
                        web.snapping = true;
                        take_snapshot(&web.view, &id, key, &tx);
                    }
                }
                if !on
                    && !url.is_empty()
                    && let Some(b) = self.m.block_mut(&id)
                    && let Some(i) = b.tab_of(key)
                    && b.tabs[i].url != url
                {
                    b.tabs[i].url = url.clone();
                    if i == b.tab {
                        b.url = url;
                    }
                    self.request_save(cx);
                }
            }
            WebEvent::NewWindow(id, url) => {
                // A link to a new window opens a tab beside the open one.
                let Some(b) = self.m.block_mut(&id) else { return };
                b.open_tab(url, true);
                self.request_save(cx);
            }
            WebEvent::External(url) => cx.open_url(&url),
            WebEvent::Snapshot(id, key, png) => {
                if let Some(web) = self.rts.get_mut(&id).and_then(|r| r.webs.get_mut(&key)) {
                    web.snapping = false;
                    web.snap = Some(std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png)));
                }
            }
        }
        cx.notify();
    }

    fn make_webview(&self, id: &str, key: u64, url: &str, window: &Window) -> Result<wry::WebView, String> {
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
                let _ = t3.try_send(WebEvent::Title(i2.clone(), key, title));
            })
            .with_on_page_load_handler(move |ev, url| {
                let on = matches!(ev, wry::PageLoadEvent::Started);
                let _ = t4.try_send(WebEvent::Loading(i3.clone(), key, on, url));
            })
            .build_as_child(window)
            .map_err(|e| e.to_string())
    }

    /// The page of the block's open tab, if it was made.
    fn open_web(&self, id: &str) -> Option<&WebRt> {
        let key = self.m.block(id).and_then(|b| b.tabs.get(b.tab))?.key;
        self.rts.get(id)?.webs.get(&key)
    }

    /// Navigates the open tab's page, as the address bar or an agent asks.
    pub(super) fn web_go(&mut self, id: &str, url: &str) {
        let key = self.m.block(id).and_then(|b| b.tabs.get(b.tab)).map(|t| t.key);
        if let Some(web) = key.and_then(|k| self.rts.get_mut(id)?.webs.get_mut(&k)) {
            if url.is_empty() {
                let _ = web.view.load_url("about:blank");
            } else {
                let _ = web.view.load_url(url);
            }
        }
    }

    /// Why the page cannot show now over its body, if it cannot.
    fn web_hidden(&self, id: &str, r: (f32, f32, f32, f32)) -> Option<&'static str> {
        if self.preview.is_some() || self.preview_leaving.is_some() || self.popup.is_some() || self.pluck.as_ref().is_some_and(|p| p.active) {
            return Some("被浮层遮住");
        }
        if self.gesture_covers(r) {
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
        let b = &match self.m.block_mut(&b.id) {
            Some(m) => {
                m.ensure_tabs();
                m.clone()
            }
            None => return div().into_any_element(),
        };
        let key = b.tabs[b.tab].key;
        let s = self.m.cam.s;
        let (sx, sy) = self.m.cam.to_screen(b.x, b.y);
        let strip_h = if b.tabs.len() > 1 { TABS_H } else { 0. };
        let top = TITLE_H + strip_h + BAR_H;
        let page = (sx + 1., sy + top * s, b.w * s - 2., (b.h - top) * s - 1.);
        let rt = self.rts.entry(b.id.clone()).or_default();
        let loading = rt.webs.get(&key).is_some_and(|w| w.loading);
        // The open tab's page: made once there is an address, shown where nothing is over it.
        let mut note: Option<String> = None;
        let mut snap: Option<std::sync::Arc<gpui::Image>> = None;
        let tx = self.web_tx.clone();
        if b.url.is_empty() {
            rt.webs.remove(&key);
            note = Some("在地址栏输入网址或要搜索的词".into());
        } else if s < LOD_SCALE {
            note = Some(String::new());
        } else {
            if self.rts.get(&b.id).is_some_and(|r| !r.webs.contains_key(&key) && r.web_err.is_none()) {
                let made = self.make_webview(&b.id, key, &b.url, window);
                let rt = self.rts.entry(b.id.clone()).or_default();
                match made {
                    Ok(view) => {
                        rt.webs.insert(key, WebRt {
                            view,
                            snap: None,
                            snapping: false,
                            zoom: 1.,
                            alpha: 1.,
                            bounds: (0., 0., 0., 0.),
                            visible: false,
                            loading: true,
                        });
                    }
                    Err(e) => rt.web_err = Some(format!("网页视图没能创建：{e}")),
                }
            }
            let hidden = self.web_hidden(&b.id, page);
            let zooming = self.zooming();
            let rt = self.rts.entry(b.id.clone()).or_default();
            let now = Instant::now();
            if let Some(e) = &rt.web_err {
                note = Some(e.clone());
            } else if let Some(web) = rt.webs.get_mut(&key) {
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
                        // Coming into sight: its picture stays under while the live page fades in.
                        if !web.visible && web.snap.is_some() {
                            rt.sharpen.begin(now);
                        }
                        let fading = rt.sharpen.tick(now);
                        let alpha = rt.sharpen.alpha(now);
                        if (web.alpha - alpha).abs() > 1e-3 {
                            set_alpha(&web.view, alpha);
                            web.alpha = alpha;
                        }
                        if fading {
                            snap = web.snap.clone();
                            self.animating = true;
                        }
                        if !web.visible {
                            let _ = web.view.set_visible(true);
                            web.visible = true;
                        }
                        self.web_shown.insert(key);
                    }
                    Some(why) => {
                        // Going out of sight: a picture of it takes its place.
                        if web.visible && !web.snapping && (!zooming || web.snap.is_none()) {
                            web.snapping = true;
                            take_snapshot(&web.view, &b.id, key, &tx);
                        }
                        snap = web.snap.clone();
                        note = Some(why.to_string());
                    }
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
        let (i1, i2, i3, i4) = (id.clone(), id.clone(), id.clone(), id.clone());
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
                    if let Some(w) = d.open_web(&i1) {
                        let _ = w.view.evaluate_script("history.back()");
                    }
                },
            )))
            .child(bar_button(SharedString::from(format!("fwd:{id}")), "arrow_right", s, t).on_click(cx.listener(
                move |d, _, _, _| {
                    if let Some(w) = d.open_web(&i2) {
                        let _ = w.view.evaluate_script("history.forward()");
                    }
                },
            )))
            .child(bar_button(SharedString::from(format!("reload:{id}")), if loading { "x" } else { "reload" }, s, t).on_click(
                cx.listener(move |d, _, _, _| {
                    if let Some(w) = d.open_web(&i3) {
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
            )
            .child(bar_button(SharedString::from(format!("newtab:{id}")), "plus", s, t).on_click(cx.listener(
                move |d, _, window, cx| {
                    if let Some(b) = d.m.block_mut(&i4) {
                        b.open_tab(String::new(), true);
                        d.request_save(cx);
                        d.start_editing(&i4, EditPart::Address, window, cx);
                    }
                },
            )));
        let strip = (b.tabs.len() > 1).then(|| self.tab_strip(b, t, s, cx));
        let backdrop: Hsla = if t.dark { gpui::rgb(0x1a1b1f).into() } else { gpui::rgb(0xf3f4f6).into() };
        if let Some(img) = snap {
            // The page's picture, scaled with the block; why it is a picture, faintly.
            let why = note.filter(|n| !n.is_empty());
            return div()
                .size_full()
                .flex()
                .flex_col()
                .children(strip)
                .child(bar)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .w_full()
                        .bg(backdrop)
                        .child(gpui::img(gpui::ImageSource::Image(img)).size_full().object_fit(gpui::ObjectFit::Fill))
                        .when_some(why, |d, why| {
                            d.child(
                                div().absolute().bottom(px(8. * s)).left_0().right_0().flex().justify_center().child(
                                    div()
                                        .px(px(9. * s))
                                        .py(px(3. * s))
                                        .rounded_full()
                                        .bg(t.surface.opacity(0.9))
                                        .text_size(px(10. * s))
                                        .text_color(t.muted)
                                        .child(why),
                                ),
                            )
                        }),
                )
                .into_any_element();
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(strip)
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

    /// The tabs, over the bar: click one to open it, × to close it.
    fn tab_strip(&self, b: &Block, t: &Theme, s: f32, cx: &mut Context<Self>) -> AnyElement {
        let tabs = b.tabs.iter().enumerate().map(|(i, tab)| {
            let open = i == b.tab;
            let (pick, shut) = (b.id.clone(), b.id.clone());
            div()
                .id(SharedString::from(format!("tab:{}:{}", b.id, tab.key)))
                .flex_1()
                .min_w_0()
                .max_w(px(200. * s))
                .h(px((TABS_H - 6.) * s))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2. * s))
                .pl(px(9. * s))
                .pr(px(3. * s))
                .rounded(px(6. * s))
                .text_size(px(11.5 * s))
                .when(open, |d| d.bg(t.surface_hover).text_color(t.text))
                .when(!open, |d| d.text_color(t.muted).cursor_pointer().hover(|d| d.bg(t.surface_hover.opacity(0.6))))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(tab.label()))
                .child(
                    div()
                        .id(SharedString::from(format!("tabx:{}:{}", b.id, tab.key)))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(16. * s))
                        .rounded(px(4. * s))
                        .cursor_pointer()
                        .hover(|d| d.bg(t.border))
                        .child(svg().path(icons::path("x")).size(px(10. * s)).text_color(t.muted))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |d, _, _, cx| {
                            cx.stop_propagation();
                            d.close_tab(&shut, i, cx);
                        })),
                )
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |d, _, _, cx| {
                    if let Some(b) = d.m.block_mut(&pick)
                        && b.tab != i
                    {
                        b.select_tab(i);
                        d.request_save(cx);
                        cx.notify();
                    }
                }))
        });
        div()
            .h(px(TABS_H * s))
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3. * s))
            .px(px(6. * s))
            .border_b_1()
            .border_color(t.border)
            .children(tabs)
            .into_any_element()
    }

    /// Closes a block's tab i and lets its page go.
    fn close_tab(&mut self, id: &str, i: usize, cx: &mut Context<Self>) {
        let Some(gone) = self.m.block_mut(id).and_then(|b| b.close_tab(i)) else { return };
        if let Some(rt) = self.rts.get_mut(id) {
            rt.webs.remove(&gone.key);
        }
        self.request_save(cx);
        cx.notify();
    }

    /// Hides the pages this frame did not show: out of view, covered, gone, or in a tab not open.
    pub(super) fn hide_unshown_pages(&mut self) {
        let shown = std::mem::take(&mut self.web_shown);
        for rt in self.rts.values_mut() {
            for (key, web) in rt.webs.iter_mut() {
                if web.visible && !shown.contains(key) {
                    let _ = web.view.set_visible(false);
                    web.visible = false;
                }
            }
        }
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
