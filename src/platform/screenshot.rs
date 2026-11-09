//! A picture of the deck window, for agents: what the camera shows, taken
//! as WindowServer composites it.
//!
//! ScreenCaptureKit takes the window by its window number. The deck's own
//! window needs no screen recording consent: macOS 14.4 and later list the
//! calling process's windows without it; older systems fall back to the
//! general list, where the window may be missing until consent is given.
//! The callbacks come on ScreenCaptureKit's own queues, so the UI thread
//! may wait on them.

/// A window's picture, as PNG, with its size in pixels.
#[derive(Clone, Debug)]
pub struct Shot {
    pub png: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

#[cfg(target_os = "macos")]
pub fn capture(window: &gpui::Window) -> Result<Shot, String> {
    mac::capture(window)
}

#[cfg(not(target_os = "macos"))]
pub fn capture(_: &gpui::Window) -> Result<Shot, String> {
    Err("截图只在 macOS 上可用".into())
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::mpsc;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::AllocAnyThread;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey};
    use objc2_core_graphics::CGImage;
    use objc2_foundation::{
        NSArray, NSDictionary, NSError, NSInteger, NSOperatingSystemVersion, NSProcessInfo,
    };
    use objc2_screen_capture_kit::{
        SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::Shot;

    /// Long enough for a big window; a call should not hang the UI thread longer.
    const TIMEOUT: Duration = Duration::from_secs(5);

    /// Whether the system is at least this version.
    fn at_least(major: NSInteger, minor: NSInteger) -> bool {
        let v = NSOperatingSystemVersion { majorVersion: major, minorVersion: minor, patchVersion: 0 };
        NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(v)
    }

    /// The window number of GPUI's window: what ScreenCaptureKit names it by.
    fn window_number(window: &gpui::Window) -> Result<usize, String> {
        let handle = HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?;
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return Err("截图只认 AppKit 窗口".into());
        };
        let view: &objc2_app_kit::NSView = unsafe { h.ns_view.cast().as_ref() };
        let window = view.window().ok_or_else(|| "窗口还没有出现".to_string())?;
        Ok(window.windowNumber() as usize)
    }

    fn why(err: *mut NSError) -> String {
        match unsafe { err.as_ref() } {
            Some(e) => format!("系统没有给出画面：{}", e.localizedDescription()),
            None => "系统没有给出画面".to_string(),
        }
    }

    /// Re-encodes a captured CGImage as PNG, with its pixel size.
    fn png_of(image: &CGImage) -> Option<(Vec<u8>, usize, usize)> {
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
        let props: Retained<NSDictionary<NSBitmapImageRepPropertyKey, AnyObject>> = NSDictionary::new();
        let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
        let width = CGImage::width(Some(image));
        let height = CGImage::height(Some(image));
        Some((data.to_vec(), width, height))
    }

    pub fn capture(window: &gpui::Window) -> Result<Shot, String> {
        if !at_least(14, 0) {
            return Err("截图需要 macOS 14 或更新版本".into());
        }
        let id = window_number(window)?;
        let (tx, rx) = mpsc::channel::<Result<Shot, String>>();
        let tx_outer = tx.clone();
        let share = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
            let Some(content) = (unsafe { content.as_ref() }) else {
                let _ = tx_outer.send(Err(why(err)));
                return;
            };
            let windows: Retained<NSArray<SCWindow>> = unsafe { content.windows() };
            let Some(win) = windows.iter().find(|w| unsafe { w.windowID() as usize == id }) else {
                let _ = tx_outer.send(Err(
                    "窗口不在可截图列表里：可能被最小化了，或系统要求「屏幕录制」权限（系统设置 › 隐私与安全性）".into(),
                ));
                return;
            };
            let filter = unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &win) };
            let cfg = unsafe { SCStreamConfiguration::new() };
            let frame = unsafe { win.frame() };
            let scale = f64::from(unsafe { filter.pointPixelScale() });
            let w = (frame.size.width * scale).round().max(1.) as usize;
            let h = (frame.size.height * scale).round().max(1.) as usize;
            unsafe {
                cfg.setWidth(w);
                cfg.setHeight(h);
                cfg.setShowsCursor(false);
            }
            let tx = tx_outer.clone();
            let done = RcBlock::new(move |image: *mut CGImage, err: *mut NSError| {
                let Some(image) = (unsafe { image.as_ref() }) else {
                    let _ = tx.send(Err(why(err)));
                    return;
                };
                let _ = tx.send(png_of(image).map(|(png, width, height)| Shot { png, width, height }).ok_or_else(|| "截到的画面编不成 PNG".to_string()));
            });
            unsafe {
                SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &cfg, Some(&done));
            }
        });
        if at_least(14, 4) {
            unsafe { SCShareableContent::getCurrentProcessShareableContentWithCompletionHandler(&share) };
        } else {
            unsafe {
                SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(false, true, &share);
            }
        }
        match rx.recv_timeout(TIMEOUT) {
            Ok(shot) => shot,
            Err(_) => Err("截图超时：窗口可能被最小化，或被系统挡住了".into()),
        }
    }
}
