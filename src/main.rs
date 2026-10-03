use bugscope::cli::parse_cli;
use bugscope::ui::{RootView, ViewMode};
use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;

actions!(
    bugscope,
    [
        OpenFile,
        ReloadGraph,
        ToggleSchema,
        ToggleLayout,
        ToggleTheme,
        ToggleSidebar,
        ToggleInsights,
        ViewGraph,
        ViewTreemap,
        ViewSunburst,
        ResetView,
        OpenPreferences,
        Quit
    ]
);

/// Handle to the live root view so OS menu actions can reach it.
struct ActiveView(WeakEntity<RootView>);
impl Global for ActiveView {}

fn with_view(cx: &mut App, f: impl FnOnce(&mut RootView, &mut Context<RootView>)) {
    let Some(entity) = cx.try_global::<ActiveView>().and_then(|h| h.0.upgrade()) else {
        return;
    };
    entity.update(cx, f);
}

fn main() {
    // Windows: broaden the DLL search path before anything backend-adjacent
    // loads (LOAD EXTENSION + transitive arrow/bz2/brotli/lz4 deps). This
    // only helps *runtime* loads — load-time DLLs need the flat zip layout
    // from scripts/stage_windows_bundle.sh, resolved before main runs.
    #[cfg(windows)]
    bugscope::windows_dll::init();

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
        cx.on_action(|_: &OpenFile, cx| with_view(cx, |v, cx| v.open_file_dialog(cx)));
        cx.on_action(|_: &ReloadGraph, cx| with_view(cx, |v, cx| v.load_graph(cx)));
        cx.on_action(|_: &ToggleSchema, cx| with_view(cx, |v, cx| v.toggle_schema(cx)));
        cx.on_action(|_: &ToggleLayout, cx| with_view(cx, |v, cx| v.toggle_layout(cx)));
        cx.on_action(|_: &ToggleTheme, cx| with_view(cx, |v, cx| v.toggle_theme(cx)));
        cx.on_action(|_: &ToggleSidebar, cx| with_view(cx, |v, cx| v.toggle_sidebar(cx)));
        cx.on_action(|_: &ToggleInsights, cx| with_view(cx, |v, cx| v.toggle_right_pane(cx)));
        cx.on_action(|_: &ViewGraph, cx| {
            with_view(cx, |v, cx| v.set_view_mode(ViewMode::Graph, cx))
        });
        cx.on_action(|_: &ViewTreemap, cx| {
            with_view(cx, |v, cx| v.set_view_mode(ViewMode::Treemap, cx))
        });
        cx.on_action(|_: &ViewSunburst, cx| {
            with_view(cx, |v, cx| v.set_view_mode(ViewMode::Sunburst, cx))
        });
        cx.on_action(|_: &ResetView, cx| with_view(cx, |v, cx| v.reset_view(cx)));
        cx.on_action(|_: &OpenPreferences, cx| with_view(cx, |v, cx| v.toggle_preferences(cx)));
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-o", OpenFile, None),
            KeyBinding::new("cmd-r", ReloadGraph, None),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new("cmd-i", ToggleInsights, None),
            KeyBinding::new("cmd-1", ViewGraph, None),
            KeyBinding::new("cmd-2", ViewTreemap, None),
            KeyBinding::new("cmd-3", ViewSunburst, None),
            KeyBinding::new("cmd-,", OpenPreferences, None),
        ]);
        cx.set_menus(vec![
            Menu {
                name: "Bugscope".into(),
                items: vec![
                    MenuItem::action("Preferences…", OpenPreferences),
                    MenuItem::separator(),
                    MenuItem::action("Quit", Quit),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Open Database…", OpenFile),
                    MenuItem::action("Reload Graph", ReloadGraph),
                    MenuItem::separator(),
                    MenuItem::action("Toggle Schema View", ToggleSchema),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Pause/Resume Layout", ToggleLayout),
                    MenuItem::action("Toggle Light/Dark Theme", ToggleTheme),
                    MenuItem::action("Toggle Sidebar", ToggleSidebar),
                    MenuItem::action("Toggle Insights Pane", ToggleInsights),
                    MenuItem::separator(),
                    MenuItem::action("Graph View", ViewGraph),
                    MenuItem::action("Treemap (Leiden) View", ViewTreemap),
                    MenuItem::action("Sunburst (Leiden) View", ViewSunburst),
                    MenuItem::separator(),
                    MenuItem::action("Reset View", ResetView),
                ],
            },
        ]);
        let holder: Rc<RefCell<Option<Entity<RootView>>>> = Rc::new(RefCell::new(None));
        let capture = holder.clone();
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
                    *capture.borrow_mut() = Some(view.clone());
                    view
                }
            },
        )
        .expect("failed to open window");
        if let Some(view) = holder.borrow().clone() {
            cx.set_global(ActiveView(view.downgrade()));
        }
        cx.activate(true);
    });
}
