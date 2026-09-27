use bugscope::cli::parse_cli;
use bugscope::ui::RootView;
use gpui::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opts = match parse_cli(&args) {
        Ok(None) => return, // --help / --version already printed
        Ok(Some(opts)) => opts,
        Err(e) => {
            eprint!("{e}");
            std::process::exit(2);
        }
    };
    Application::new().run(move |cx: &mut App| {
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
                    title: Some("Bugscope".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            {
                let opts = opts.clone();
                move |window, cx| {
                    let view = cx.new(|cx| RootView::with_cli(&opts, cx));
                    // Autofocus the search box so typing works immediately.
                    window.focus(&view.read(cx).query_focus_handle());
                    view
                }
            },
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
