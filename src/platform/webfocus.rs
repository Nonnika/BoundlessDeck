//! A web page takes the keyboard only when the user clicks into it. WebKit
//! makes a page the window's first responder by itself when the page focuses
//! an input (autofocus, a script): a browser block just made would then take
//! the keys typed on the canvas. So the page last clicked into is noted, and
//! any other page found holding the keyboard (each frame of the deck, and
//! before each key goes out) hands it back to GPUI's view, its parent.
//! Likewise the wheel: over a page not clicked into, two-finger scrolling
//! moves the canvas, as over any other block, instead of the page. And a
//! press on a block's resize handle, which the page lies over, goes to the
//! deck with the drag that follows it.
//!
//! The keyboard must never be left with no one: a page holding it that is
//! hidden, closed or gone leaves it to the window itself, and the canvas then
//! gets no keys until the next click. So the keyboard also goes back to
//! GPUI's view when the window holds it, or a page out of sight does, and
//! when the deck gives the keyboard to something of its own (`release`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The WKWebView the user last pressed in, as a pointer; 0 when the last
/// press was elsewhere.
static CLICKED: AtomicUsize = AtomicUsize::new(0);

/// The resize handles the deck drew last, as x, y, w, h in window points
/// from the top-left.
static CORNERS: Mutex<Vec<[f32; 4]>> = Mutex::new(Vec::new());

/// Notes where the resize handles are; the deck calls it each frame.
pub fn set_corners(corners: Vec<[f32; 4]>) {
    if let Ok(mut c) = CORNERS.lock() {
        *c = corners;
    }
}

/// Whether (x, y), in window points from the top-left, is on a resize handle.
fn on_corner(x: f32, y: f32) -> bool {
    CORNERS.lock().is_ok_and(|c| c.iter().any(|r| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]))
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ptr::NonNull;

    use objc2::rc::Retained;
    use objc2::runtime::NSObjectProtocol;
    use objc2::{ClassType, MainThreadMarker, Message};
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventType, NSView, NSWindow};
    use objc2_web_kit::WKWebView;

    use std::cell::RefCell;

    use super::{CLICKED, Ordering, on_corner};

    thread_local! {
        /// GPUI's view, while a press on a resize handle over a page is
        /// being dragged: its drag and release go there too.
        static PRESSED: RefCell<Option<Retained<NSView>>> = const { RefCell::new(None) };
        /// The deck's GPUI view, as it drew last: where the keyboard goes back to.
        static CANVAS: RefCell<Option<Retained<NSView>>> = const { RefCell::new(None) };
    }

    pub fn set_canvas(view: &NSView) {
        CANVAS.with(|c| {
            let same = c.borrow().as_ref().is_some_and(|v| std::ptr::eq(&**v, view));
            if !same {
                *c.borrow_mut() = Some(view.retain());
            }
        });
    }

    /// The deck's view, if it lies in win.
    fn canvas_in(win: &NSWindow) -> Option<Retained<NSView>> {
        CANVAS.with(|c| c.borrow().clone()).filter(|v| v.window().is_some_and(|w| std::ptr::eq(&*w, win)))
    }

    /// The web view v is, or lies in.
    fn web_of(v: Retained<NSView>) -> Option<Retained<NSView>> {
        let mut v = Some(v);
        while let Some(view) = v {
            if view.isKindOfClass(WKWebView::class()) {
                return Some(view);
            }
            v = unsafe { view.superview() };
        }
        None
    }

    /// Hands the keyboard back to the canvas (GPUI's view, the pages'
    /// parent) unless the page the user clicked into holds it and shows.
    /// With no page holding it, the window itself may: then too.
    pub fn guard_window(win: &NSWindow) {
        let Some(canvas) = canvas_in(win) else { return };
        let view = win.firstResponder().and_then(|fr| fr.downcast::<NSView>().ok());
        let keep = match view {
            // The window itself, or nothing: the keys would go nowhere.
            None => false,
            Some(view) => match web_of(view) {
                Some(web) => {
                    Retained::as_ptr(&web) as usize == CLICKED.load(Ordering::Relaxed)
                        && !web.isHiddenOrHasHiddenAncestor()
                        && web.window().is_some()
                }
                // GPUI's view, or a view of the system's.
                None => true,
            },
        };
        if !keep {
            win.makeFirstResponder(Some(&canvas));
        }
    }

    pub fn install() {
        let handler = block2::RcBlock::new(|ev: NonNull<NSEvent>| -> *mut NSEvent {
            let e = unsafe { ev.as_ref() };
            let Some(mtm) = MainThreadMarker::new() else { return ev.as_ptr() };
            let Some(win) = e.window(mtm) else { return ev.as_ptr() };
            let kind = e.r#type();
            if kind == NSEventType::KeyDown {
                guard_window(&win);
                return ev.as_ptr();
            }
            if kind == NSEventType::LeftMouseDragged || kind == NSEventType::LeftMouseUp {
                let to = PRESSED.with(|p| if kind == NSEventType::LeftMouseUp { p.borrow_mut().take() } else { p.borrow().clone() });
                let Some(canvas) = to else { return ev.as_ptr() };
                if kind == NSEventType::LeftMouseUp { canvas.mouseUp(e) } else { canvas.mouseDragged(e) }
                return std::ptr::null_mut();
            }
            let Some(content) = win.contentView() else { return ev.as_ptr() };
            let hit = content.hitTest(e.locationInWindow()).and_then(web_of);
            if e.r#type() == NSEventType::ScrollWheel {
                // Over a page not clicked into, scrolling moves the canvas:
                // the event goes to GPUI's view, the page's parent, instead.
                if let Some(web) = hit
                    && Retained::as_ptr(&web) as usize != CLICKED.load(Ordering::Relaxed)
                    && let Some(canvas) = unsafe { web.superview() }
                {
                    canvas.scrollWheel(e);
                    return std::ptr::null_mut();
                }
                return ev.as_ptr();
            }
            // A press on a resize handle the page lies over: the deck's.
            if kind == NSEventType::LeftMouseDown
                && let Some(web) = &hit
                && let Some(canvas) = unsafe { web.superview() }
            {
                let p = e.locationInWindow();
                let top = content.frame().size.height - p.y;
                if on_corner(p.x as f32, top as f32) {
                    CLICKED.store(0, Ordering::Relaxed);
                    canvas.mouseDown(e);
                    PRESSED.with(|p| *p.borrow_mut() = Some(canvas));
                    return std::ptr::null_mut();
                }
            }
            // A press: in a page, that page may take the keyboard and the wheel.
            CLICKED.store(hit.map_or(0, |w| Retained::as_ptr(&w) as usize), Ordering::Relaxed);
            ev.as_ptr()
        });
        let mask = NSEventMask::LeftMouseDown
            | NSEventMask::RightMouseDown
            | NSEventMask::OtherMouseDown
            | NSEventMask::KeyDown
            | NSEventMask::ScrollWheel
            | NSEventMask::LeftMouseDragged
            | NSEventMask::LeftMouseUp;
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) };
        // The monitor lives as long as the app.
        std::mem::forget(monitor);
        std::mem::forget(handler);
    }
}

/// Installs the monitor; call once, on the main thread.
pub fn install() {
    #[cfg(target_os = "macos")]
    mac::install();
}

/// The deck gave the keyboard to something of its own (the canvas, an
/// editor, a terminal): a page clicked into before gives it back.
pub fn release() {
    CLICKED.store(0, Ordering::Relaxed);
}

/// Takes the keyboard back from a page that took it unclicked, or that
/// left it to no one; each frame.
pub fn guard(window: &gpui::Window) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSView;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
        let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
        let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
        mac::set_canvas(view);
        if let Some(win) = view.window() {
            mac::guard_window(&win);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
    let _ = &CLICKED;
}
