use bugscope_gpui::ui::RootView;
use gpui::*;

fn main() {
    Application::new().run(|cx: &mut App| {
        // Menu + window chrome come from GPUI itself — no Tauri webview, no
        // titlebar plugin, no capability files.
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1400.0), px(900.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Bugscope — native graph explorer".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(RootView::new);
                // Autofocus the search box so typing works immediately.
                window.focus(&view.read(cx).query_focus_handle());
                view
            },
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
