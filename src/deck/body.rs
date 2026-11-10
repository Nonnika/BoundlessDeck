//! What a block's card holds under its title: QuickLooks, 笔记, files,
//! and the parsed Markdown and chart data cached for them.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ObjectFit, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, StyledImage, Window, div, img, prelude::FluentBuilder, px, uniform_list,
};

use super::{DOC_FONT, DeckView, EDITOR_FONT, EditPart, ImageFinder, TITLE_H, message};
use crate::model::{Block, BlockKind};
use crate::content::terminal;
use crate::ui::theme::{MdColors, Paper, Theme};
use crate::content::{markdown, preview};

impl DeckView {
    pub(super) fn parsed_markdown(&mut self, id: &str, src: &str, c: &MdColors, dark: bool) -> markdown::Parsed {
        let rt = self.rts.entry(id.to_string()).or_default();
        if let Some((cached_src, parsed)) = &rt.md_cache
            && cached_src == src
            && parsed.is_dark() == dark
        {
            return parsed.clone();
        }
        let parsed = markdown::parse_doc(src, c, dark);
        rt.md_cache = Some((src.to_string(), parsed.clone()));
        parsed
    }

    /// A 文字's words as Markdown, in its own colors, each typed line its own line.
    pub(super) fn text_markdown(&mut self, id: &str, src: &str, c: &MdColors, dark: bool) -> markdown::Parsed {
        // The colors are baked into the parse (links, code), so they are part of what it is cached by.
        // Compared in place: the text is not copied into a key each frame.
        let colors = format!("{:?}{:?}{:?}\u{0}", c.text, c.link, c.muted);
        let rt = self.rts.entry(id.to_string()).or_default();
        if let Some((cached, parsed)) = &rt.md_cache
            && cached.len() == colors.len() + src.len()
            && cached.starts_with(&colors)
            && cached[colors.len()..] == *src
            && parsed.is_dark() == dark
        {
            return parsed.clone();
        }
        let parsed = markdown::parse_lines(src, c, dark);
        rt.md_cache = Some((colors + src, parsed.clone()));
        parsed
    }

    pub(super) fn face_parsed_markdown(
        &mut self,
        b: &Block,
        c: &MdColors,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> Option<markdown::Parsed> {
        let md = self.face_markdown(b, cx);
        if md.is_empty() {
            return None;
        }
        let rt = self.rts.entry(b.id.clone()).or_default();
        if let Some((cached_src, parsed)) = &rt.face_md_cache
            && cached_src == &md
            && parsed.is_dark() == dark
        {
            return Some(parsed.clone());
        }
        let parsed = markdown::parse_doc(&md, c, dark);
        rt.face_md_cache = Some((md, parsed.clone()));
        Some(parsed)
    }

    /// How much of a block's body, from its top, can show in the window:
    /// lists of its text lay out no further. GPUI's lists fill their own
    /// height, not the window's: zoomed in on a long note, its whole height
    /// would be laid out though most of it is past the window's bottom.
    /// Bodies drawn away from the canvas (a stack's preview) are not cut.
    pub(super) fn body_room(&self, id: &str) -> Option<f32> {
        if !self.clip_bodies {
            return None;
        }
        let b = self.m.block(id)?;
        let cam = self.m.cam;
        let top = (b.y - cam.y + TITLE_H) * cam.s;
        let full = ((b.h - TITLE_H) * cam.s).max(1.);
        Some((self.view.1 - top).clamp(1., full))
    }

    /// A Markdown body as it reads, in a list of its blocks that lays out only
    /// those in sight: a long document costs what shows, also while zooming.
    /// Its scroll stays where it was when the text changes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn markdown_list(
        &mut self,
        id: &str,
        parsed: markdown::Parsed,
        size: f32,
        colors: MdColors,
        images: Rc<ImageFinder>,
        pad: [f32; 4],
        outer_w: f32,
        toggle: Option<markdown::OnToggle>,
    ) -> AnyElement {
        let rt = self.rts.entry(id.to_string()).or_default();
        let state = match &mut rt.md_list {
            Some((st, p)) if p.same(&parsed) => st.clone(),
            Some((st, p)) => {
                let top = st.logical_scroll_top();
                st.reset(parsed.len());
                st.scroll_to(gpui::ListOffset { item_ix: top.item_ix.min(parsed.len().saturating_sub(1)), offset_in_item: top.offset_in_item });
                *p = parsed.clone();
                st.clone()
            }
            None => {
                let st = gpui::ListState::new(parsed.len(), gpui::ListAlignment::Top, px(100.));
                rt.md_list = Some((st.clone(), parsed.clone()));
                st
            }
        };
        // GPUI's list keeps only its top and bottom padding (items start at its
        // left edge, as wide as it), so each item carries the sides itself.
        let (pr, pl) = (px(pad[1]), px(pad[3]));
        let room = self.body_room(id);
        let list = gpui::list(state, move |ix, _, _| {
            div().pl(pl).pr(pr).child(parsed.render_item(ix, size, &colors, &*images, toggle.as_ref())).into_any_element()
        })
        .w(px(outer_w.max(40.)))
        .pt(px(pad[0]))
        .pb(px(pad[2]));
        div()
            .size_full()
            .child(match room {
                Some(h) => list.h(px(h)),
                None => list.h_full(),
            })
            .into_any_element()
    }

    /// What a click on a checkbox of block `id`'s Markdown does: flips that task.
    pub(super) fn task_toggle(&self, id: &str, cx: &mut Context<Self>) -> markdown::OnToggle {
        let this = cx.entity().downgrade();
        let id = id.to_string();
        Rc::new(move |at, _, cx| {
            let _ = this.update(cx, |d, cx| d.toggle_task(&id, at, cx));
        })
    }

    /// Ticks or unticks the task whose bracketed byte is at `at` in a 笔记's
    /// text (written to its file) or a QuickLook's (saved with the deck).
    pub(super) fn toggle_task(&mut self, id: &str, at: usize, cx: &mut Context<Self>) {
        let Some(b) = self.m.block_mut(id) else { return };
        match b.kind {
            BlockKind::Memo => {
                let Some(rt) = self.rts.get_mut(id).filter(|r| r.memo.loaded) else { return };
                let Some(next) = markdown::toggle_task(&rt.memo.text, at) else { return };
                rt.memo.text = next;
                self.save_memo(id, cx);
            }
            BlockKind::Note => {
                let Some(next) = markdown::toggle_task(&b.text, at) else { return };
                b.text = next;
                self.request_save(cx);
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn chart_data(&mut self, b: &Block) -> Arc<crate::content::chart::Data> {
        let rt = self.rts.entry(b.id.clone()).or_default();
        if let Some((cached_src, data)) = &rt.chart_cache
            && cached_src == &b.text
        {
            return data.clone();
        }
        let data = Arc::new(crate::content::chart::parse(&b.text));
        rt.chart_cache = Some((b.text.clone(), data.clone()));
        data
    }

    pub(super) fn body(&mut self, b: &Block, t: &Theme, paper: &Paper, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
        match b.kind {
            BlockKind::Note => {
                if let Some(ed) = self.editing(&b.id, EditPart::Body).cloned() {
                    ed.update(cx, |e, _| {
                        e.font_size = 14. * s;
                        e.color = paper.text;
                        e.muted = paper.muted;
                        e.accent = t.accent;
                    });
                    // Laid out at the content scale, so text wraps the same all through a zoom.
                    return div()
                        .w(px(b.w * s))
                        .h_full()
                        .flex_none()
                        .pt(px(6. * s))
                        .px(px(8. * s))
                        .pb(px(22. * s))
                        .child(ed)
                        .into_any_element();
                }
                let pad = [12. * s, 14. * s, 26. * s, 14. * s];
                if b.text.is_empty() {
                    let hint = div().text_size(px(14. * s)).text_color(paper.muted).child("点击编辑，支持 Markdown");
                    return live_preview(&b.id, hint.into_any_element(), pad, b.w * s);
                }
                let colors = MdColors::of_paper(paper);
                let parsed = self.parsed_markdown(&b.id, &b.text, &colors, t.dark);
                let toggle = self.task_toggle(&b.id, cx);
                self.markdown_list(&b.id, parsed, 14. * s, colors, Rc::new(markdown::local_image), pad, b.w * s, Some(toggle))
            }
            BlockKind::Memo => self.memo_body(b, t, cx),
            BlockKind::File => self.file_body(b, t, window, cx),
            BlockKind::Terminal => {
                // The shell starts on the block's first resume (`lifecycle`).
                let rt = self.rts.get(&b.id);
                match rt.and_then(|r| r.term.clone()) {
                    Some(term) => {
                        let (cols, rows) = terminal::grid_for(b.w, b.h - TITLE_H);
                        term.update(cx, |tm, cx| {
                            if tm.font_size != terminal::FONT_SIZE * s {
                                tm.font_size = terminal::FONT_SIZE * s;
                                cx.notify();
                            }
                            tm.resize(cols, rows);
                        });
                        div().size_full().bg(terminal::background(t.dark)).child(term).into_any_element()
                    }
                    None => message(
                        rt.and_then(|r| r.term_err.clone()).unwrap_or_else(|| "启动终端…".into()),
                        t.muted,
                        s,
                    ),
                }
            }
            BlockKind::Chart => {
                let data = self.chart_data(b);
                let chart = crate::content::chartview::render_data(data, &b.chart, s, t);
                let Some(ed) = self.editing(&b.id, EditPart::Body).cloned() else { return chart };
                ed.update(cx, |e, _| {
                    e.font_size = 12. * s;
                    e.color = t.text;
                    e.muted = t.muted;
                    e.accent = t.accent;
                    e.mono = true;
                });
                // The table beside the chart, which follows it live; under it when narrow.
                let side = b.w >= 520.;
                let table = div()
                    .flex_none()
                    .p(px(8. * s))
                    .bg(t.surface_hover)
                    .when(side, |d| d.w(px(b.w * s * 0.42)).h_full().border_r_1())
                    .when(!side, |d| d.h(px((b.h - TITLE_H) * s * 0.45)).w(px(b.w * s)).border_b_1())
                    .border_color(t.border)
                    .child(ed);
                div()
                    .size_full()
                    .flex()
                    .when(side, |d| d.flex_row())
                    .when(!side, |d| d.flex_col())
                    .child(table)
                    .child(div().flex_1().min_w_0().min_h_0().child(chart))
                    .into_any_element()
            }
            BlockKind::Stack => self.stack_body(b, t, window, cx),
            BlockKind::Browser => self.browser_body(b, t, window, cx),
            // Drawn by `bare_view`, not in a card.
            BlockKind::Text | BlockKind::Shape => div().into_any_element(),
        }
    }

    pub(super) fn memo_body(&mut self, b: &Block, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
        let size = DOC_FONT * s;
        let editing = self.editing(&b.id, EditPart::Body).cloned();
        let rt = self.rts.entry(b.id.clone()).or_default();
        if let Some(err) = &rt.memo.err {
            return message(err.clone(), t.danger, s);
        }
        if !rt.memo.loaded {
            self.load_memo(&b.id, cx);
            return message("加载中…".into(), t.muted, s);
        }
        if let Some(ed) = editing {
            // The text may have arrived after the edit began.
            let late = (ed.read(cx).text.is_empty() && !rt.memo.text.is_empty()).then(|| rt.memo.text.clone());
            ed.update(cx, |e, cx| {
                if let Some(text) = late {
                    e.set_text(text, cx);
                }
                e.font_size = size;
                e.color = t.text;
                e.muted = t.muted;
                e.accent = t.accent;
            });
            return div().w(px(b.w * s)).h_full().flex_none().py(px(8. * s)).px(px(12. * s)).child(ed).into_any_element();
        }
        let save_err = rt.memo.save_err.clone();
        self.check_memo(&b.id, cx);
        let colors = MdColors::of_theme(t);
        let pad = [12. * s, 16. * s, 12. * s, 16. * s];
        // Lent out of the runtime for the parse, not copied each frame; nothing
        // between here and its return reads it.
        let text = std::mem::take(&mut self.rts.entry(b.id.clone()).or_default().memo.text);
        let parsed = (!text.trim().is_empty()).then(|| self.parsed_markdown(&b.id, &text, &colors, t.dark));
        self.rts.entry(b.id.clone()).or_default().memo.text = text;
        if let (None, Some(parsed)) = (&save_err, &parsed) {
            let toggle = self.task_toggle(&b.id, cx);
            return self.markdown_list(&b.id, parsed.clone(), size, colors, Rc::new(markdown::local_image), pad, b.w * s, Some(toggle));
        }
        let content = div()
            .flex()
            .flex_col()
            .gap(px(6. * s))
            .when_some(save_err, |d, e| d.child(div().text_size(px(12. * s)).text_color(t.danger).child(e)))
            .child(match parsed {
                None => div().text_size(px(size)).text_color(t.muted).child("双击开始写，支持 Markdown").into_any_element(),
                Some(parsed) => markdown::render_parsed(&parsed, size, &colors),
            })
            .into_any_element();
        live_preview(&b.id, content, pad, b.w * s)
    }

    pub(super) fn file_body(&mut self, b: &Block, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
        let path = self.m.path_of(&b.path);
        if matches!(preview::of(&b.path), preview::Kind::Markdown | preview::Kind::Word | preview::Kind::Pdf | preview::Kind::Text) {
            self.check_file(&b.id, cx);
        }
        match preview::of(&b.path) {
            preview::Kind::Image => div()
                .size_full()
                .child(img(path).size_full().object_fit(ObjectFit::Contain))
                .into_any_element(),
            preview::Kind::Markdown => {
                self.load_doc(&b.id, cx);
                let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                match self.rts.get(&b.id).and_then(|r| r.doc.clone()) {
                    None => message("加载中…".into(), t.muted, s),
                    Some(Err(e)) => message(e, t.danger, s),
                    Some(Ok(text)) => {
                        let colors = MdColors::of_theme(t);
                        let parsed = self.parsed_markdown(&b.id, &text, &colors, t.dark);
                        let images = Rc::new(move |url: &str| {
                            if url.starts_with("http://") || url.starts_with("https://") {
                                return None;
                            }
                            let p = std::path::Path::new(url);
                            Some(gpui::ImageSource::from(if p.is_absolute() { p.to_path_buf() } else { base.join(p) }))
                        });
                        self.markdown_list(&b.id, parsed, DOC_FONT * s, colors, images, [12. * s, 16. * s, 12. * s, 16. * s], b.w * s, None)
                    }
                }
            }
            preview::Kind::Word => {
                self.load_word(&b.id, cx);
                match self.rts.get(&b.id).and_then(|r| r.word.clone()) {
                    None => message("加载中…".into(), t.muted, s),
                    Some(Err(e)) => message(e, t.danger, s),
                    Some(Ok(doc)) if doc.markdown.trim().is_empty() => message("（空文档）".into(), t.muted, s),
                    Some(Ok(doc)) => {
                        let d2 = doc.clone();
                        let colors = MdColors::of_theme(t);
                        let parsed = self.parsed_markdown(&b.id, &doc.markdown, &colors, t.dark);
                        let images = Rc::new(move |url: &str| {
                            let name = url.strip_prefix(crate::content::word::MEDIA_SCHEME)?;
                            d2.media.get(name).map(|im| gpui::ImageSource::Image(im.clone()))
                        });
                        self.markdown_list(&b.id, parsed, DOC_FONT * s, colors, images, [12. * s, 16. * s, 16. * s, 16. * s], b.w * s, None)
                    }
                }
            }
            preview::Kind::Pdf => self.pdf_body(b, t, cx),
            preview::Kind::Video => self.video_body(b, t, window),
            preview::Kind::Text => {
                self.load_lines(&b.id, cx);
                let rt = self.rts.entry(b.id.clone()).or_default();
                match rt.lines.clone() {
                    None => message("加载中…".into(), t.muted, s),
                    Some(Err(e)) => message(e, t.danger, s),
                    Some(Ok(lines)) => {
                        let size = EDITOR_FONT * s;
                        let lh = (size * 1.5).round().max(1.);
                        let digits = lines.len().to_string().len() as f32;
                        let (muted, text) = (t.muted, t.text);
                        let handle = rt.list.clone();
                        let room = self.body_room(&b.id);
                        uniform_list(SharedString::from(format!("text:{}", b.id)), lines.len(), move |range, _, _| {
                            range
                                .map(|i| {
                                    div()
                                        .h(px(lh))
                                        .flex()
                                        .flex_row()
                                        .whitespace_nowrap()
                                        .child(
                                            div()
                                                .w(px(size * 0.62 * digits + size))
                                                .flex_none()
                                                .text_color(muted)
                                                .child(format!("{}", i + 1)),
                                        )
                                        .child(div().text_color(text).child(lines[i].clone()))
                                })
                                .collect()
                        })
                        .track_scroll(handle)
                        .w_full()
                        .map(|d| match room {
                            Some(h) => d.h(px(h)),
                            None => d.h_full(),
                        })
                        .px(px(6. * s))
                        .py(px(4. * s))
                        .font_family(crate::ui::font::mono())
                        .text_size(px(size))
                        .into_any_element()
                    }
                }
            }
        }
    }

    /// Covers a scrolling body while it is not the block worked in, or while
    /// a scroll the canvas took goes on: the wheel then moves the canvas, and
    /// a click makes the block the one worked in.
    pub(super) fn body_cover(&self, b: &Block, cx: &mut Context<Self>) -> Option<AnyElement> {
        let scrolls = match b.kind {
            BlockKind::Terminal | BlockKind::Note | BlockKind::Memo => true,
            BlockKind::File => matches!(preview::of(&b.path), preview::Kind::Text | preview::Kind::Markdown | preview::Kind::Word),
            _ => false,
        };
        if !scrolls || self.editing(&b.id, EditPart::Body).is_some() {
            return None;
        }
        let gesture = self.canvas_scroll_until.is_some_and(|t| t > Instant::now());
        if self.active.as_deref() == Some(&b.id) && !gesture {
            return None;
        }
        let id = b.id.clone();
        Some(
            div()
                .id(SharedString::from(format!("cover:{}", b.id)))
                .absolute()
                .left_0()
                .top_0()
                .size_full()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_cover_down(&id, ev, window, cx)))
                .into_any_element(),
        )
    }
}

/// A QuickLook's or 笔记's body as it reads, in a scroller of its own.
fn live_preview(id: &str, content: AnyElement, pad: [f32; 4], outer_w: f32) -> AnyElement {
    div()
        .id(SharedString::from(format!("live:{id}")))
        .size_full()
        .overflow_y_scroll()
        .child(div().w(px(outer_w.max(40.))).pt(px(pad[0])).pr(px(pad[1])).pb(px(pad[2])).pl(px(pad[3])).child(content))
        .into_any_element()
}
