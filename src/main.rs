//! BoundlessDeck, in Rust: an infinite-canvas workbench drawn natively with
//! GPUI. Each deck is a folder: the home screen opens one, and the deck is
//! saved inside it, in the same deck.json the Go build reads.

mod app;
mod chartview;
mod deck;
mod editor;
mod icons;
mod lifespan;
mod markdown;
mod memo;
mod model;
mod preview;
mod recents;
mod tape;
mod terminal;
mod termstate;
mod theme;

use gpui::{
    AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};

fn main() {
    // `boundlessdeck <folder>` opens the folder's deck at once.
    let open = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    Application::new().with_assets(icons::Assets).run(|cx| {
        editor::bind_keys(cx);
        app::bind_keys(cx);
        cx.set_menus(app::menus());
        cx.on_action(|_: &app::Quit, cx| cx.quit());
        cx.on_action(|_: &app::About, _| {});

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
                window.on_window_should_close(cx, move |_, cx| {
                    root.update(cx, |r, cx| r.close_deck(cx));
                    true
                });
            })
            .ok();
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.on_app_quit({
            let window = window;
            move |cx| {
                window.update(cx, |r, _, cx| r.close_deck(cx)).ok();
                async {}
            }
        })
        .detach();
        cx.activate(true);
    });
}
