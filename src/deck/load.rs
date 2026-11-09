//! Loading what blocks show off the UI thread: text files, documents,
//! Word files, PDF pages, and a terminal's shell.

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, AppContext, Context, InteractiveElement, IntoElement, MouseButton, ObjectFit, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, StyledImage, div, img, prelude::FluentBuilder, px,
};

use super::{DeckView, TITLE_H, WordDoc, image_format, message, pdf_backdrop, sharpen};
use crate::model::Block;
use crate::content::preview;
use crate::content::terminal::{self, Terminal, TerminalEvent};
use crate::ui::theme::Theme;

impl DeckView {
    pub(super) fn load_lines(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.lines.is_some() {
            return;
        }
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { preview::read_lines(&path) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                rt.lines = Some(r.map(Arc::new));
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn load_doc(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.doc.is_some() {
            return;
        }
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { preview::read_doc(&path) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                rt.doc = Some(r.map(Arc::new));
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn load_word(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.word.is_some() {
            return;
        }
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move {
                    crate::content::word::read(&path).map(|d| WordDoc {
                        markdown: d.markdown,
                        media: d
                            .media
                            .into_iter()
                            .map(|(name, bytes)| {
                                let img = Arc::new(gpui::Image::from_bytes(image_format(&name), bytes));
                                (name, img)
                            })
                            .collect(),
                    })
                })
                .await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                rt.word = Some(r.map(Arc::new));
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_pdf(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.pdf.is_some() {
            return;
        }
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { crate::content::pdf::Doc::open(&path) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                rt.pdf = Some(r.map(Arc::new));
                cx.notify();
            });
        })
        .detach();
    }

    /// Renders the page the block shows, unless it shows already or one renders.
    pub(super) fn render_pdf(&mut self, id: &str, page: usize, width: usize, cx: &mut Context<Self>) {
        let Some(rt) = self.rts.get_mut(id) else { return };
        let Some(Ok(doc)) = rt.pdf.clone() else { return };
        if rt.pdf_rendering || rt.pdf_img.as_ref().is_some_and(|(p, w, _)| *p == page && *w == width) {
            return;
        }
        rt.pdf_rendering = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move {
                    doc.render(page, width).and_then(|(w, h, px)| {
                        let buf = image::RgbaImage::from_raw(w as u32, h as u32, px).ok_or("无法绘制这一页")?;
                        Ok(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(buf)])))
                    })
                })
                .await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.pdf_rendering = false;
                match r {
                    Ok(img) => {
                        // The sharper page fades in over the one it replaces.
                        rt.pdf_prev = rt.pdf_img.take().map(|(_, _, old)| old);
                        rt.pdf_img = Some((page, width, img));
                        rt.sharpen.begin(Instant::now());
                    }
                    Err(e) => rt.pdf = Some(Err(e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn pdf_body(&mut self, b: &Block, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
        self.open_pdf(&b.id, cx);
        let doc = match self.rts.get(&b.id).and_then(|r| r.pdf.clone()) {
            None => return message("加载中…".into(), t.muted, s),
            Some(Err(e)) => return message(e, t.danger, s),
            Some(Ok(doc)) => doc,
        };
        let pages = doc.pages();
        let page = (b.page.max(0) as usize).min(pages - 1);
        let has_img = self.rts.get(&b.id).is_some_and(|r| r.pdf_img.is_some());
        if !self.zooming() || !has_img {
            let render_s = if self.lay.s > 0.0 { self.lay.s } else { s };
            self.render_pdf(&b.id, page, crate::content::pdf::width_for(b.w * render_s), cx);
        }
        let now = Instant::now();
        let rt = self.rts.entry(b.id.clone()).or_default();
        let fading = rt.sharpen.tick(now);
        if !fading {
            rt.pdf_prev = None;
        }
        let alpha = rt.sharpen.alpha(now);
        let pic = |im| img(gpui::ImageSource::Render(im)).size_full().object_fit(ObjectFit::Contain).into_any_element();
        let img_el = rt.pdf_img.clone().map(|(_, _, im)| {
            if fading { sharpen::crossfade(rt.pdf_prev.clone().map(pic), pic(im), alpha).into_any_element() } else { pic(im) }
        });
        self.animating |= fading;
        let ps = s.min(1.5);
        let turn = |key: &str, glyph: &'static str, to: Option<usize>, cx: &mut Context<Self>| {
            let id = b.id.clone();
            let can = to.is_some_and(|p| p < pages);
            div()
                .id(SharedString::from(format!("{key}:{}", b.id)))
                .px(px(9. * ps))
                .py(px(1. * ps))
                .text_size(px(15. * ps))
                .text_color(if can { t.text } else { t.muted.opacity(0.35) })
                .when(can, |d| d.cursor_pointer().hover(|d| d.bg(t.surface_hover)))
                .child(glyph)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |d, _, _, cx| {
                    if let (true, Some(to)) = (can, to)
                        && let Some(b) = d.m.block_mut(&id)
                    {
                        b.page = to as i32;
                        d.request_save(cx);
                        cx.notify();
                    }
                }))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(pdf_backdrop(t))
            .child(div().flex_1().min_h_0().w_full().p(px(8. * s)).children(img_el))
            .when(pages > 1, |d| {
                d.child(
                    div().w_full().flex().justify_center().pb(px(8. * ps)).child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2. * ps))
                            .p(px(2. * ps))
                            .bg(t.surface.opacity(0.92))
                            .border_1()
                            .border_color(t.border)
                            .occlude()
                            .child(turn("pdf-prev", "‹", page.checked_sub(1), cx))
                            .child(
                                div()
                                    .px(px(4. * ps))
                                    .text_size(px(12. * ps))
                                    .text_color(t.muted)
                                    .child(format!("{} / {pages}", page + 1)),
                            )
                            .child(turn("pdf-next", "›", Some(page + 1), cx)),
                    ),
                )
            })
            .into_any_element()
    }

    /// Starts the block's shell in the deck's folder.
    pub(super) fn start_terminal(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.term.is_some() || rt.term_err.is_some() {
            return;
        }
        let (cols, rows) = terminal::grid_for(b.w, b.h - TITLE_H);
        let term = match Terminal::spawn(&self.m.root, cols, rows, id) {
            Ok(make) => cx.new(make),
            Err(e) => {
                rt.term_err = Some(format!("终端没能启动：{e}"));
                return;
            }
        };
        let eid = id.to_string();
        let sub = cx.subscribe(&term, move |d: &mut DeckView, _, ev: &TerminalEvent, cx| {
            match ev {
                TerminalEvent::Title(t) => {
                    if let Some(b) = d.m.block_mut(&eid) {
                        b.auto_title(t);
                    }
                }
                TerminalEvent::Exited(code) => {
                    if let Some(b) = d.m.block_mut(&eid) {
                        b.title = format!("{} — 已退出 ({code})", b.title);
                    }
                }
            }
            cx.notify();
        });
        rt.term = Some(term);
        rt._term_sub = Some(sub);
    }
}
