//! BoundlessDeck: pictures of what an element painted, to show in its place.
//!
//! A frame can ask for what was painted inside some bounds to be kept as a
//! picture (`Window::paint_snapshot_capture`): the renderer draws those
//! primitives again into a texture of their own, after the frame. Later
//! frames can show that picture, stretched into where the element is now
//! (`Window::paint_snapshot`), instead of laying it out and painting it: a
//! canvas gliding between views shows its blocks' text as pictures and lays
//! it out again only where it comes to rest.
//!
//! A picture holds only the part of the element that was in the window when
//! it was taken (`visible`); the rest of its bounds stays bare. Pictures are
//! kept by the caller's key, with the caller's version of the content: a
//! picture of another version is not shown.

use crate::{Bounds, DevicePixels, Hsla, Pixels, ScaledPixels, Size};
use std::{cell::RefCell, ops::Range};

/// Texture indices from here up name pictures, not the sprite atlas's textures.
pub(crate) const SNAPSHOT_SLOT_BASE: u32 = 0x4000_0000;

/// How many pictures are kept; the least recently shown go first.
const MAX_SNAPSHOTS: usize = 96;

/// What a frame asked to be kept, for the renderer.
#[derive(Clone, Debug)]
pub(crate) struct SnapshotRequest {
    pub key: u64,
    pub version: u64,
    /// The scene's paint operations painted inside the bounds.
    pub ops: Range<usize>,
    /// The element's bounds, in device pixels.
    pub bounds: Bounds<ScaledPixels>,
    /// The part of them in the window, in device pixels.
    pub visible: Bounds<ScaledPixels>,
    /// What shows under the content. A see-through one (alpha below 1) is
    /// not drawn: the picture holds the content alone, its colors multiplied
    /// by its coverage, to lay over what is drawn live under it (a gradient).
    pub background: Hsla,
}

/// What a kept picture is of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapshotInfo {
    /// The caller's version of the content it shows.
    pub version: u64,
    /// The element's bounds when it was taken, in window pixels.
    pub bounds: Bounds<Pixels>,
    /// The part of them it holds, in window pixels.
    pub visible: Bounds<Pixels>,
}

pub(crate) struct Snapshot {
    pub version: u64,
    pub bounds: Bounds<ScaledPixels>,
    pub visible: Bounds<ScaledPixels>,
    pub slot: u32,
    pub size: Size<DevicePixels>,
    /// It holds the content alone, its colors multiplied by its coverage.
    pub premultiplied: bool,
    pub last_used: u64,
    #[cfg(target_os = "macos")]
    pub texture: metal::Texture,
}

#[derive(Default)]
pub(crate) struct Registry {
    pub snapshots: collections::FxHashMap<u64, Snapshot>,
    /// Keys whose content of a version cannot be pictured (paths, surfaces in
    /// it), by that version: the caller draws them as they are.
    pub refused: collections::FxHashMap<u64, u64>,
    next_slot: u32,
    clock: u64,
}

impl Registry {
    pub fn next_slot(&mut self) -> u32 {
        self.next_slot = self.next_slot.wrapping_add(1) % (u32::MAX - SNAPSHOT_SLOT_BASE);
        SNAPSHOT_SLOT_BASE + self.next_slot
    }

    pub fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// Drops the least recently used pictures past `MAX_SNAPSHOTS`.
    pub fn trim(&mut self) {
        while self.snapshots.len() > MAX_SNAPSHOTS {
            let Some(oldest) = self.snapshots.iter().min_by_key(|(_, s)| s.last_used).map(|(k, _)| *k) else {
                break;
            };
            self.snapshots.remove(&oldest);
        }
    }

    #[cfg(target_os = "macos")]
    pub fn texture_for_slot(&self, slot: u32) -> Option<metal::Texture> {
        self.snapshots.values().find(|s| s.slot == slot).map(|s| s.texture.clone())
    }
}

thread_local! {
    /// The pictures kept, for the windows of this (the main) thread.
    pub(crate) static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Forgets the picture kept for `key` (its element is gone).
pub fn forget_snapshot(key: u64) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.snapshots.remove(&key);
        r.refused.remove(&key);
    });
}

/// Notes that `key`'s content of `version` cannot be pictured.
pub(crate) fn refuse_snapshot(key: u64, version: u64) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.snapshots.remove(&key);
        if r.refused.len() > 4 * MAX_SNAPSHOTS {
            r.refused.clear();
        }
        r.refused.insert(key, version);
    });
}

/// Paints `child`, keeping what it painted as a picture under `key`, of the
/// caller's `version` of it, over `background` (`Window::paint_snapshot_capture`);
/// a see-through `background` keeps the content alone, for a live one under it.
pub fn snapshot_capture(key: u64, version: u64, background: crate::Hsla, child: impl crate::IntoElement) -> SnapshotCapture {
    SnapshotCapture { key, version, background, child: Some(child.into_any_element()) }
}

/// An element that paints its child and keeps what it painted as a picture.
pub struct SnapshotCapture {
    key: u64,
    version: u64,
    background: crate::Hsla,
    child: Option<crate::AnyElement>,
}

impl crate::Element for SnapshotCapture {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<crate::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&crate::GlobalElementId>,
        _inspector_id: Option<&crate::InspectorElementId>,
        window: &mut crate::Window,
        cx: &mut crate::App,
    ) -> (crate::LayoutId, ()) {
        (self.child.as_mut().unwrap().request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&crate::GlobalElementId>,
        _inspector_id: Option<&crate::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut crate::Window,
        cx: &mut crate::App,
    ) {
        self.child.as_mut().unwrap().prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&crate::GlobalElementId>,
        _inspector_id: Option<&crate::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut crate::Window,
        cx: &mut crate::App,
    ) {
        let mut child = self.child.take().unwrap();
        window.paint_snapshot_capture(self.key, self.version, bounds, self.background, |window| child.paint(window, cx));
    }
}

impl crate::IntoElement for SnapshotCapture {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
