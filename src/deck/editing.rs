//! Editing a block's text in place: starting and stopping the editor.

use std::time::Instant;

use gpui::{AppContext, Context, Entity, Focusable, Window};

use super::{DeckView, EditPart, Editing, browser};
use crate::ui::editor::{Editor, EditorEvent};
use crate::content::memo;
use crate::model::BlockKind;

impl DeckView {
    pub(super) fn start_editing(&mut self, id: &str, part: EditPart, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.as_ref().is_some_and(|e| e.id == id && e.part == part) {
            return;
        }
        self.stop_editing(cx);
        let Some(b) = self.m.block(id) else { return };
        let kind = b.kind;
        let text = match (part, kind) {
            (EditPart::Title, _) => b.title.clone(),
            (EditPart::Address, _) => b.url.clone(),
            (EditPart::Body, BlockKind::Memo) => self.rts.get(id).map(|r| r.memo.text.clone()).unwrap_or_default(),
            (EditPart::Body, _) => b.text.clone(),
        };
        let from_title = b.title.clone();
        let multiline = part == EditPart::Body;
        let select_all = part != EditPart::Body;
        let editor = cx.new(|cx| {
            let mut e = Editor::new(text, multiline, window, cx);
            e.placeholder = match (part, kind) {
                (EditPart::Title, _) => "标题".into(),
                (EditPart::Address, _) => "输入网址或搜索 · 回车打开".into(),
                (_, BlockKind::Note) => "支持 Markdown · Esc 或 ⌘↵ 完成".into(),
                (_, BlockKind::Chart) => "名称\t数值 · 每行一项".into(),
                _ => "Markdown · Esc 或 ⌘↵ 完成".into(),
            };
            if select_all {
                e.select_all_text();
            }
            // 笔记 and QuickLook edit Typora-style: rendered but for the lines being edited.
            e.markdown = part == EditPart::Body && matches!(kind, BlockKind::Memo | BlockKind::Note);
            e
        });
        let eid = id.to_string();
        let sub = cx.subscribe(&editor, move |d: &mut DeckView, ed, ev: &EditorEvent, cx| match ev {
            EditorEvent::Changed => {
                let text = ed.read(cx).text.clone();
                let Some(e) = d.editing.as_ref().filter(|e| e.id == eid) else { return };
                let part = e.part;
                let is_memo = d.m.block(&eid).is_some_and(|b| b.kind == BlockKind::Memo);
                match part {
                    EditPart::Address => {} // taken as the edit ends
                    EditPart::Title => {
                        if let Some(b) = d.m.block_mut(&eid) {
                            b.title = text;
                        }
                    }
                    EditPart::Body if is_memo => {
                        let rt = d.rts.entry(eid.clone()).or_default();
                        rt.memo.text = text;
                        rt.memo.edited = Some(Instant::now());
                        d.schedule_memo_save(&eid, cx);
                    }
                    EditPart::Body => {
                        if let Some(b) = d.m.block_mut(&eid) {
                            b.text = text;
                        }
                    }
                }
                cx.notify();
            }
            EditorEvent::Done | EditorEvent::Blurred => {
                if d.editing.as_ref().is_some_and(|e| e.id == eid) {
                    // Done typing, the block stays the one worked in.
                    d.active = Some(eid.clone());
                    d.stop_editing(cx);
                    cx.notify();
                }
            }
        });
        let f = editor.focus_handle(cx);
        crate::platform::webfocus::release();
        window.focus(&f);
        self.editing = Some(Editing { id: id.to_string(), part, editor, from_title, _sub: sub });
        cx.notify();
    }

    /// Keeps what was typed and shows the block as it reads again.
    pub(super) fn stop_editing(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.editing.take() else { return };
        let root = self.m.root.clone();
        let typed = e.editor.read(cx).text.clone();
        let Some(b) = self.m.block_mut(&e.id) else { return };
        match e.part {
            EditPart::Address => {
                if let Ok(url) = browser::address_to_url(&typed)
                    && url != b.url
                {
                    b.url = url.clone();
                    b.ensure_tabs();
                    self.web_go(&e.id, &url);
                }
            }
            EditPart::Title => {
                b.title = b.title.trim().to_string();
                if b.kind == BlockKind::Memo && b.title != e.from_title && !b.title.is_empty() {
                    let (rel, title) = (b.path.clone(), b.title.clone());
                    match memo::rename_file(&root, &rel, &title) {
                        Ok(next) => {
                            b.title = memo::title_of(&next);
                            b.path = next;
                            b.named = false;
                        }
                        Err(err) => {
                            b.title = e.from_title.clone();
                            self.rts.entry(e.id.clone()).or_default().memo.save_err = Some(format!("没能改名：{err}"));
                        }
                    }
                } else if b.title.is_empty() {
                    b.title = b.default_title();
                    b.named = false;
                } else if b.title != e.from_title {
                    b.named = true;
                }
            }
            EditPart::Body => {
                if b.kind == BlockKind::Memo {
                    self.save_memo(&e.id, cx);
                }
            }
        }
        // A QuickLook whose time ran out while it was written goes now.
        self.request_save(cx);
        cx.notify();
    }

    pub(super) fn editing(&self, id: &str, part: EditPart) -> Option<&Entity<Editor>> {
        self.editing.as_ref().filter(|e| e.id == id && e.part == part).map(|e| &e.editor)
    }
}
