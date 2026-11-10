//! BoundlessDeck, in Rust: an infinite-canvas workbench drawn natively with
//! GPUI. Each deck is a folder: the home screen opens one, and the deck is
//! saved inside it, in the same deck.json the Go build reads.

mod app;
mod content;
mod deck;
mod mcp;
mod model;
mod platform;
mod ui;

use app::{commands, welcome, windows};
use platform::{pinch, webfocus};
use ui::{editor, font, icons};

use gpui::{
    AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};

fn main() {
    // The same binary is the MCP's stdio bridge: agents start it as
    // `boundlessdeck mcp-stdio [--workspace DIR]`, and it relays to the app.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = mcp::bridge::main(&args) {
        std::process::exit(code);
    }
    // `boundlessdeck <folder>` opens the folder's deck at once.
    let open = args.first().map(std::path::PathBuf::from);
    Application::new().with_assets(icons::Assets).run(|cx| {
        font::init(cx);
        editor::bind_keys(cx);
        pinch::install();
        webfocus::install();
        commands::bind_keys(cx);
        cx.set_menus(commands::menus());
        cx.on_action(|_: &app::Quit, cx| cx.quit());
        cx.on_action(|_: &app::About, _| {});
        cx.on_action(|_: &app::OpenSettings, cx| windows::show_settings(cx));
        cx.on_action(|_: &app::ShowShortcuts, cx| windows::show_shortcuts(cx));
        cx.on_action(|_: &app::ShowWelcome, cx| welcome::show(cx));

        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("BoundlessDeck".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(19.))),
                    }),
                    window_min_size: Some(size(px(680.), px(420.))),
                    ..Default::default()
                },
                move |_, cx| cx.new(|cx| app::Root::new(open, cx)),
            )
            .expect("打开窗口");

        // Closing the window saves the deck open and hangs its shells up.
        window
            .update(cx, |_, window, cx| {
                let root = cx.entity();
                cx.set_global(windows::RootHandle(root.downgrade()));
                window.on_window_should_close(cx, move |_, cx| {
                    root.update(cx, |r, cx| r.close_deck(cx));
                    true
                });
            })
            .ok();
        // The main window closed: the app is done; a small window closing is not.
        cx.on_window_closed(move |cx| {
            if window.update(cx, |_, _, _| ()).is_err() {
                cx.quit();
            }
        })
        .detach();
        cx.on_app_quit(move |cx| {
            window.update(cx, |r, _, cx| r.close_deck(cx)).ok();
            async {}
        })
        .detach();
        set_dock_icon();
        cx.activate(true);
        // The first time the app starts, the welcome window tells what it does.
        if welcome::due(cx) {
            welcome::show(cx);
        }
    });
}

/// The app's icon in the Dock. A bundled .app takes it from its Info.plist,
/// but `cargo run` starts a bare binary, which would show the generic one.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(include_bytes!("../resources/icon.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else { return };
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon() {}
