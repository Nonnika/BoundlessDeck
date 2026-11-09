//! The places the app talks to macOS itself: the Colors panel and the
//! eyedropper, the trackpad's pinch, window pictures, and who holds the
//! keyboard between GPUI and web pages.

pub(crate) mod nativepanel;
pub(crate) mod pinch;
pub(crate) mod screenshot;
pub(crate) mod webfocus;
