//! A 笔记's Markdown file: read when it shows, written back after edits,
//! and read again when it changes on disk.

use std::time::{Duration, Instant};

use gpui::{AppContext, Context};

use super::DeckView;
use crate::memo;
use crate::model::{Block, BlockKind};

impl DeckView {
    pub(super) fn load_memo(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        if b.path.is_empty() {
            return;
        }
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.memo.loaded || rt.memo.err.is_some() {
            return;
        }
        rt.loading = true;
        let path = self.m.path_of(&b.path);
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { memo::read(&path) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                match r {
                    Ok((text, modified)) => {
                        rt.memo.text = text.clone();
                        rt.memo.saved = text;
                        rt.memo.loaded = true;
                        rt.memo.modified = Some(modified);
                        rt.memo.checked = Some(Instant::now());
                    }
                    Err(e) => rt.memo.err = Some(e),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn schedule_memo_save(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(memo::SAVE_IDLE).await;
            let _ = this.update(cx, |d, cx| {
                let rested = d
                    .rts
                    .get(&id)
                    .and_then(|r| r.memo.edited)
                    .is_some_and(|t| t.elapsed() >= memo::SAVE_IDLE - Duration::from_millis(20));
                if rested {
                    d.save_memo(&id, cx);
                }
            });
        }).detach();
    }

    /// Writes what was typed to the note's file, unless the file holds it already.
    pub(super) fn save_memo(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id).filter(|b| b.kind == BlockKind::Memo && !b.path.is_empty()) else {
            return;
        };
        let path = self.m.path_of(&b.path);
        let Some(rt) = self.rts.get_mut(id) else { return };
        if !rt.memo.loaded || rt.memo.text == rt.memo.saved || rt.memo.saving {
            return;
        }
        rt.memo.saving = true;
        let text = rt.memo.text.clone();
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let t2 = text.clone();
            let r = cx.background_spawn(async move { memo::write(&path, &t2) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id.clone()).or_default();
                rt.memo.saving = false;
                match r {
                    Ok(m) => {
                        rt.memo.saved = text;
                        rt.memo.modified = Some(m);
                        rt.memo.save_err = None;
                    }
                    Err(e) => rt.memo.save_err = Some(format!("没能保存：{e}")),
                }
                // Typed on meanwhile: write that too.
                if rt.memo.text != rt.memo.saved {
                    d.save_memo(&id, cx);
                }
                cx.notify();
            });
        }).detach();
    }

    /// Writes what was typed in the note, here and now: it pauses, leaves, or the deck closes.
    /// Takes the block as it is or was, as one that left the deck is no longer in the model.
    pub(super) fn flush_memo(&mut self, b: &Block) {
        if b.kind != BlockKind::Memo || b.path.is_empty() {
            return;
        }
        let path = self.m.path_of(&b.path);
        if let Some(rt) = self.rts.get_mut(&b.id)
            && rt.memo.loaded
            && rt.memo.text != rt.memo.saved
            && memo::write(&path, &rt.memo.text).is_ok()
        {
            rt.memo.saved = rt.memo.text.clone();
        }
    }

    /// Reads the note's file again when something else changed it, at most every 2 s.
    pub(super) fn check_memo(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let Some(rt) = self.rts.get_mut(id) else { return };
        let m = &mut rt.memo;
        if !m.loaded || rt.loading || m.saving || b.path.is_empty() || m.text != m.saved {
            return;
        }
        if m.checked.is_some_and(|t| t.elapsed() < memo::POLL) {
            return;
        }
        m.checked = Some(Instant::now());
        let Some(known) = m.modified else { return };
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { memo::changed_since(&path, known) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                match r {
                    Err(e) => rt.memo.err = Some(e),
                    Ok(Some((text, m))) if rt.memo.text == rt.memo.saved => {
                        rt.memo.text = text.clone();
                        rt.memo.saved = text;
                        rt.memo.modified = Some(m);
                    }
                    _ => {}
                }
                cx.notify();
            });
        }).detach();
    }
}
