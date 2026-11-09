//! Trackpad pinches. GPUI 0.2.2 passes no magnify events on, so an AppKit
//! local monitor catches them before any view does: each step goes to the
//! deck as (x, y, factor), x and y in the window's points from its top-left.
//! A pinch over a web page zooms the canvas, as anywhere else, not the page.

use std::sync::OnceLock;

/// Where the pinches go: the deck open takes them from here.
static PINCHES: OnceLock<smol::channel::Sender<(f32, f32, f32)>> = OnceLock::new();

/// The receiving end, made once; the deck open reads it.
pub fn channel() -> smol::channel::Receiver<(f32, f32, f32)> {
    static RX: OnceLock<smol::channel::Receiver<(f32, f32, f32)>> = OnceLock::new();
    RX.get_or_init(|| {
        let (tx, rx) = smol::channel::unbounded();
        let _ = PINCHES.set(tx);
        rx
    })
    .clone()
}

/// Installs the monitor; call once, on the main thread.
#[cfg(target_os = "macos")]
pub fn install() {
    use std::ptr::NonNull;

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSEventMask};

    let _ = channel();
    let handler = block2::RcBlock::new(|ev: NonNull<NSEvent>| -> *mut NSEvent {
        let e = unsafe { ev.as_ref() };
        let Some(mtm) = MainThreadMarker::new() else { return ev.as_ptr() };
        let Some(win) = e.window(mtm) else { return ev.as_ptr() };
        // The main window's title ends in BoundlessDeck; the small windows' do not.
        if !win.title().to_string().ends_with("BoundlessDeck") {
            return ev.as_ptr();
        }
        let Some(view) = win.contentView() else { return ev.as_ptr() };
        let h = view.frame().size.height;
        let p = e.locationInWindow();
        let k = 1. + e.magnification();
        if let Some(tx) = PINCHES.get() {
            let _ = tx.try_send((p.x as f32, (h - p.y) as f32, k as f32));
        }
        std::ptr::null_mut() // the canvas's: no view zooms by itself
    });
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::Magnify, &handler) };
    // The monitor lives as long as the app.
    std::mem::forget(monitor);
    std::mem::forget(handler);
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}
