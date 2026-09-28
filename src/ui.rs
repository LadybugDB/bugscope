//! GPUI view tree — port of `App.tsx` + `HeaderBar` + `Sidebar` +
//! `QueryBox` + `GraphView` (layout, interaction, overlay).

use crate::backend::{self, DatabaseInfo, GraphData, GraphNode};
use crate::cli::CliOptions;
use crate::model::{node_size, Camera, GraphModel, Vec2};
use crate::theme::{edge_color, highlight, node_color, Theme};
use gpui::*;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone)]
struct Snapshot {
    nodes: Vec<SnapNode>,
    links: Vec<SnapLink>,
    camera: Camera,
    hovered: Option<usize>,
    selected: Option<usize>,
    theme: Theme,
}

#[derive(Clone)]
struct SnapLink {
    source: usize,
    target: usize,
    label: String,
}

#[derive(Clone)]
struct SnapNode {
    x: f32,
    y: f32,
    degree: usize,
    color: usize,
    name: String,
}

pub struct RootView {
    databases: Vec<DatabaseInfo>,
    selected_db: Option<usize>,
    db_dir: PathBuf,
    full: GraphData,
    model: GraphModel,
    camera: Camera,
    status: String,
    query: String,
    /// Byte offset of the text cursor inside `query` (always a char boundary).
    query_cursor: usize,
    search_results: Vec<GraphNode>,
    focused: Option<String>,
    hovered: Option<usize>,
    selected: Option<usize>,
    schema_mode: bool,
    sidebar_open: bool,
    /// `Some(true)` = force dark, `Some(false)` = light, `None` = follow OS.
    dark_override: Option<bool>,
    /// Effective dark state from the last render — the theme toggle flips
    /// relative to this, so the OS menu action needs no window handle.
    theme_dark: bool,
    limit: usize,
    running: bool,
    query_focus: FocusHandle,
    still_ticks: u32,
    drag_node: Option<usize>,
    panning: Option<Point<Pixels>>,
    canvas_origin: Rc<Cell<Point<Pixels>>>,
    canvas_size: Rc<Cell<Size<Pixels>>>,
}

impl RootView {
    pub fn query_focus_handle(&self) -> FocusHandle {
        self.query_focus.clone()
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_cli(&CliOptions::default(), cx)
    }

    /// Constructor honoring parsed command-line options: `--dir` replaces
    /// the scanned directory, positional files are appended (first valid one
    /// is selected), plus `--limit` / `--schema` overrides.
    pub fn with_cli(opts: &CliOptions, cx: &mut Context<Self>) -> Self {
        let db_dir = opts.db_dir.clone().unwrap_or_else(backend::default_db_dir);
        let mut databases = backend::scan_for_databases(&db_dir);
        for file in &opts.files {
            if !file.is_file() {
                eprintln!("bugscope: skipping {file:?}: not a file");
                continue;
            }
            let info = backend::database_info_for_path(file);
            if !databases.iter().any(|d| d.path == info.path) {
                databases.push(info);
            }
        }
        // Match scan_for_databases id assignment for appended entries.
        for (i, db) in databases.iter_mut().enumerate() {
            db.id = i;
        }
        let selected_db = opts
            .files
            .iter()
            .filter_map(|f| {
                let p = f.to_string_lossy().into_owned();
                databases.iter().position(|d| d.path == p)
            })
            .next()
            .or(if databases.is_empty() { None } else { Some(0) });
        let query_focus = cx.focus_handle();
        let mut this = Self {
            databases,
            selected_db,
            db_dir,
            full: GraphData::default(),
            model: GraphModel::default(),
            camera: Camera::default(),
            status: "Pick a database to begin.".to_string(),
            query: String::new(),
            query_cursor: 0,
            search_results: Vec::new(),
            focused: None,
            hovered: None,
            selected: None,
            schema_mode: opts.schema_mode,
            sidebar_open: false,
            dark_override: None,
            theme_dark: false,
            limit: opts.limit.unwrap_or(backend::EDGE_SCAN_LIMIT),
            running: false,
            query_focus,
            still_ticks: 0,
            drag_node: None,
            panning: None,
            canvas_origin: Rc::new(Cell::new(point(px(0.), px(0.)))),
            canvas_size: Rc::new(Cell::new(size(px(900.), px(600.)))),
        };
        if this.selected_db.is_some() {
            this.load_graph(cx);
        }
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(33))
                .await;
            let alive = this
                .update(cx, |view, cx| {
                    if view.running && !view.model.nodes.is_empty() {
                        view.model.tick();
                        // Settle: stop ticking once the layout goes quiet so
                        // framed content stays centered instead of drifting.
                        let peak = view
                            .model
                            .nodes
                            .iter()
                            .map(|n| n.vel.x.abs().max(n.vel.y.abs()))
                            .fold(0.0f32, f32::max);
                        if peak < 0.05 {
                            view.still_ticks += 1;
                            if view.still_ticks > 30 {
                                view.running = false;
                            }
                        } else {
                            view.still_ticks = 0;
                        }
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();
        this
    }

    fn set_status(&mut self, s: impl Into<String>, cx: &mut Context<Self>) {
        self.status = s.into();
        cx.notify();
    }

    pub fn load_graph(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_db else { return };
        let Some(db) = self.databases.get(id) else {
            return;
        };
        let path = db.path.clone();
        let name = db.name.clone();
        let schema_mode = self.schema_mode;
        match backend::open_connection(&path).and_then(|conn| {
            if schema_mode {
                backend::collect_schema_graph(&conn)
            } else {
                backend::collect_edge_graph(&conn, self.limit)
            }
        }) {
            Ok(data) => {
                let msg = format!(
                    "{name}: {} nodes, {} edges",
                    data.nodes.len(),
                    data.links.len()
                );
                self.full = data.clone();
                self.model.load(&data);
                self.model.settle();
                self.frame_initial(None);
                self.focused = None;
                self.selected = None;
                self.running = !self.schema_mode;
                self.set_status(msg, cx);
            }
            Err(e) => self.set_status(format!("Load failed: {e:#}"), cx),
        }
    }

    pub fn open_file_dialog(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => return,
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                let mut info = backend::database_info_for_path(&path);
                if let Some(existing) = view.databases.iter().position(|d| d.path == info.path) {
                    view.selected_db = Some(existing);
                } else {
                    info.id = view.databases.len();
                    view.databases.push(info);
                    view.selected_db = Some(view.databases.len() - 1);
                }
                view.load_graph(cx);
            });
        })
        .detach();
    }

    /// OS menu actions — one-liners around the old header buttons.
    pub fn toggle_schema(&mut self, cx: &mut Context<Self>) {
        self.schema_mode = !self.schema_mode;
        self.load_graph(cx);
    }

    pub fn toggle_layout(&mut self, cx: &mut Context<Self>) {
        self.running = !self.running;
        cx.notify();
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.dark_override = Some(!self.theme_dark);
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.query.clear();
        self.query_cursor = 0;
        self.search_results.clear();
        self.load_graph(cx);
    }

    /// Keep the cursor on a char boundary and inside the text.
    fn clamp_cursor(&mut self) {
        while !self.query.is_char_boundary(self.query_cursor) && self.query_cursor > 0 {
            self.query_cursor -= 1;
        }
        self.query_cursor = self.query_cursor.min(self.query.len());
    }

    fn query_insert(&mut self, text: &str) {
        self.clamp_cursor();
        self.query.insert_str(self.query_cursor, text);
        self.query_cursor += text.len();
    }

    /// Backspace: delete the char before the cursor (⌘⌫ clears to start).
    fn query_backspace(&mut self, to_start: bool) {
        self.clamp_cursor();
        if to_start {
            self.query.drain(..self.query_cursor);
            self.query_cursor = 0;
            return;
        }
        if self.query_cursor == 0 {
            return;
        }
        let mut start = self.query_cursor - 1;
        while !self.query.is_char_boundary(start) && start > 0 {
            start -= 1;
        }
        self.query.drain(start..self.query_cursor);
        self.query_cursor = start;
    }

    /// Forward delete: remove the char under the cursor.
    fn query_delete_fwd(&mut self) {
        self.clamp_cursor();
        if self.query_cursor >= self.query.len() {
            return;
        }
        let mut end = self.query_cursor + 1;
        while !self.query.is_char_boundary(end) && end < self.query.len() {
            end += 1;
        }
        self.query.drain(self.query_cursor..end);
    }

    fn query_move(&mut self, dir: i8) {
        self.clamp_cursor();
        if dir < 0 {
            if self.query_cursor > 0 {
                let mut c = self.query_cursor - 1;
                while !self.query.is_char_boundary(c) && c > 0 {
                    c -= 1;
                }
                self.query_cursor = c;
            }
        } else {
            self.query_cursor = self
                .query
                .char_indices()
                .map(|(i, _)| i)
                .find(|&i| i > self.query_cursor)
                .unwrap_or(self.query.len());
        }
    }

    /// Enter in the query bar: `/foo` searches node names/properties,
    /// anything else runs as a Cypher query and replaces the graph.
    fn run_query(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_db else { return };
        let Some(db) = self.databases.get(id) else {
            return;
        };
        let raw = self.query.clone();
        let raw = raw.trim().to_string();
        if raw.is_empty() {
            return;
        }
        let is_search = raw.starts_with('/');
        let mut term = raw.trim_start_matches('/').trim().to_string();
        // Accept both `/rdf` and `/search rdf` (the old hint text
        // suggested the latter).
        if let Some(rest) = term.strip_prefix("search") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                term = rest.trim().to_string();
            }
        }
        if term.is_empty() {
            return;
        }
        let path = db.path.clone();
        let work = if is_search {
            // Search mode: reuse the substring scan, presented as matches.
            backend::open_connection(&path).and_then(|conn| {
                backend::search_nodes(&conn, &term).map(|nodes| GraphData {
                    nodes,
                    links: vec![],
                })
            })
        } else {
            backend::open_connection(&path).and_then(|conn| backend::run_cypher(&conn, &raw))
        };
        match work {
            Ok(data) => {
                if is_search {
                    let n = data.nodes.len();
                    self.search_results = data.nodes;
                    if n > 0 {
                        // Matches only render in the sidebar — open it so
                        // the search visibly does something.
                        self.sidebar_open = true;
                    }
                    self.set_status(format!("{n} matches for {term} — click to focus"), cx);
                } else {
                    self.search_results.clear();
                    self.focused = None;
                    self.selected = None;
                    self.full = data.clone();
                    self.model.load(&data);
                    self.model.settle();
                    self.frame_initial(None);
                    self.running = !data.nodes.is_empty() && !self.schema_mode;
                    self.set_status(
                        format!("Cypher: {} nodes, {} edges", data.nodes.len(), data.links.len()),
                        cx,
                    );
                }
            }
            Err(e) => self.set_status(format!("Query failed: {e:#}"), cx),
        }
    }

    fn focus_node(&mut self, node_id: &str, cx: &mut Context<Self>) {
        let view = backend::neighborhood(&self.full, node_id);
        if view.nodes.is_empty() {
            self.set_status(format!("Node {node_id} not in loaded graph"), cx);
            return;
        }
        self.focused = Some(node_id.to_string());
        self.model.load(&view);
        self.model.settle();
        self.frame_initial(Some(node_id));
        self.running = true;
        self.set_status(format!("Neighborhood of {node_id}"), cx);
    }

    /// Default framing: zoomed in on the densest viewport with labels on.
    /// Centering a single highest-degree hub fails on large graphs — the hub
    /// can sit far from the bulk, leaving an almost-empty viewport. Instead
    /// pick the zoom first, then center the window position holding the most
    /// graph mass (degree-weighted). Callers must `settle` the model first
    /// so density reflects topology, not the initial spiral.
    fn frame_initial(&mut self, focus: Option<&str>) {
        let s = self.canvas_size.get();
        let vw = f32::from(s.width).max(50.0);
        let vh = f32::from(s.height).max(50.0);
        self.camera.fit(&self.model, vw, vh);
        self.camera.zoom = self.camera.zoom.clamp(0.9, 2.0);
        let zoom = self.camera.zoom;
        // World-space half extents of the viewport at this zoom.
        let hw = (vw as f64 / zoom / 2.0) as f32;
        let hh = (vh as f64 / zoom / 2.0) as f32;
        // Focused node wins outright; otherwise score candidates by the
        // degree-weighted mass visible around them.
        if let Some(id) = focus {
            if let Some(a) = self.model.nodes.iter().find(|n| n.id == id) {
                self.camera.center_x = a.pos.x as f64;
                self.camera.center_y = a.pos.y as f64;
                self.still_ticks = 0;
                return;
            }
        }
        // Candidates: degree-weighted centroid + top hubs by degree.
        let mut order: Vec<usize> = (0..self.model.nodes.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.model.nodes[i].degree));
        let mut cx = 0.0f64;
        let mut cy = 0.0f64;
        let mut cw = 0.0f64;
        for n in &self.model.nodes {
            let w = 1.0 + n.degree as f64;
            cx += n.pos.x as f64 * w;
            cy += n.pos.y as f64 * w;
            cw += w;
        }
        let mut best = (cx / cw.max(1.0), cy / cw.max(1.0), -1.0f64);
        for &i in order.iter().take(200) {
            let p = &self.model.nodes[i].pos;
            let mut mass = 0.0f64;
            for n in &self.model.nodes {
                if (n.pos.x - p.x).abs() <= hw && (n.pos.y - p.y).abs() <= hh {
                    mass += 1.0 + n.degree as f64;
                }
            }
            if mass > best.2 {
                best = (p.x as f64, p.y as f64, mass);
            }
        }
        self.camera.center_x = best.0;
        self.camera.center_y = best.1;
        self.still_ticks = 0;
    }

    fn snapshot(&self, theme: Theme) -> Snapshot {
        Snapshot {
            theme,
            nodes: self
                .model
                .nodes
                .iter()
                .map(|n| SnapNode {
                    x: n.pos.x,
                    y: n.pos.y,
                    degree: n.degree,
                    color: n.color,
                    name: truncate_label(&n.name),
                })
                .collect(),
            links: self
                .model
                .links
                .iter()
                .map(|l| SnapLink {
                    source: l.source,
                    target: l.target,
                    label: truncate_label(&l.label),
                })
                .collect(),
            camera: self.camera,
            hovered: self.hovered,
            selected: self.selected,
        }
    }

    /// Theme with the manual light/dark override applied on top of the
    /// OS appearance.
    fn theme_for(&self, window: &Window) -> Theme {
        match self.dark_override {
            Some(true) => Theme::tokyo_night(),
            Some(false) => Theme::flexoki_light(),
            None => Theme::current(window),
        }
    }

    fn render_header(&mut self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let db_name = self
            .selected_db
            .and_then(|id| self.databases.get(id))
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "no database".to_string());
        let status = self.status.clone();
        // The theme button flips between the two palettes; the glyph shows
        // what you get when you click (sun on dark, moon on light).
        let theme_button = if theme.dark { "☀" } else { "☾" };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .bg(theme.surface)
            .text_color(theme.foreground)
            .child(div().font_weight(FontWeight::BOLD).child("Bugscope"))
            .child(div().text_sm().child(format!("{db_name} · {status}")))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme.selection)
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, window, cx| {
                                    let current = view
                                        .dark_override
                                        .unwrap_or_else(|| Theme::current(window).dark);
                                    view.dark_override = Some(!current);
                                    cx.notify();
                                }),
                            )
                            .child(theme_button),
                    ),
            )
    }

    /// Divider gutter between the sidebar and the canvas: a 1px line split
    /// into top/bottom segments with the collapse chevron straddling its
    /// vertical middle. Always rendered so the bar can be reopened when
    /// hidden. Pure flexbox — no absolute positioning needed.
    fn render_divider(&mut self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // Chevron points the way the bar will go.
        let side_glyph = if self.sidebar_open { "«" } else { "»" };
        let line = || div().flex_1().w(px(1.)).bg(theme.border);
        div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(18.))
            .h_full()
            .py_1()
            .child(line())
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .rounded_md()
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .text_color(theme.secondary)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.selection))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| view.toggle_sidebar(cx)),
                    )
                    .child(side_glyph),
            )
            .child(line())
    }

    fn render_sidebar(&mut self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .w(px(260.))
            .h_full()
            .id("sidebar")
            .overflow_y_scroll()
            .bg(theme.surface)
            .text_color(theme.foreground)
            .text_sm()
            .child(div().font_weight(FontWeight::BOLD).child("Databases"))
            .child(
                div()
                    .text_xs()
                    .child(self.db_dir.to_string_lossy().to_string()),
            );
        for d in self.databases.clone() {
            let id = d.id;
            let active = Some(id) == self.selected_db;
            col = col.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(if active { theme.accent } else { theme.surface })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.selected_db = Some(id);
                            view.load_graph(cx);
                        }),
                    )
                    .child(d.name.clone()),
            );
        }
        col = col
            .child(div().pt_2().font_weight(FontWeight::BOLD).child("Matches"))
            .child(div().text_xs().child(format!(
                "{} nodes · {} edges{}",
                self.model.nodes.len(),
                self.model.links.len(),
                self.focused
                    .as_ref()
                    .map(|f| format!(" · focus {f}"))
                    .unwrap_or_default()
            )));
        for node in self.search_results.clone().into_iter().take(30) {
            let id = node.id.clone();
            let label = format!("{} · {}", node.name, node.label);
            col = col.child(
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.selection))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.focus_node(&id.clone(), cx);
                        }),
                    )
                    .child(label),
            );
        }
        if let Some(sel) = self.selected.and_then(|i| self.model.nodes.get(i)).cloned() {
            let id = sel.id.clone();
            col = col
                .child(div().pt_2().font_weight(FontWeight::BOLD).child("Selected"))
                .child(
                    div()
                        .text_xs()
                        .child(format!("{} · {}", sel.name, sel.label)),
                )
                .child(div().text_xs().child(format!("id {}", sel.id)))
                .child(div().text_xs().child(format!("degree {}", sel.degree)))
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .mt_1()
                        .rounded_md()
                        .bg(highlight(&theme))
                        .text_color(theme.background)
                        .cursor_pointer()
                        .text_sm()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, _, cx| {
                                view.focus_node(&id.clone(), cx);
                            }),
                        )
                        .child("Expand neighborhood"),
                );
        }
        col
    }

    /// Window coords → canvas-local coords using the origin captured at prepaint.
    fn to_local(&self, p: Point<Pixels>) -> (f32, f32) {
        let o = self.canvas_origin.get();
        (f32::from(p.x - o.x), f32::from(p.y - o.y))
    }

    fn canvas_size(&self) -> (f32, f32) {
        let s = self.canvas_size.get();
        (f32::from(s.width), f32::from(s.height))
    }

    fn render_graph_canvas(&mut self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let snap = self.snapshot(theme);
        let origin_cell = self.canvas_origin.clone();
        let size_cell = self.canvas_size.clone();
        div()
            .flex_1()
            .h_full()
            .bg(theme.inset)
            // Structural clip: everything the canvas paints — discs, strokes,
            // and especially the labels, which paint outside paint_layer — is
            // cut at this box, so zoomed content can never cover the search
            // box, title, or sidebar no matter what the label math does.
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        origin_cell.set(bounds.origin);
                        size_cell.set(bounds.size);
                        (snap, bounds)
                    },
                    move |_bounds, (snap, bounds), window, cx| {
                        paint_graph(&snap, bounds, window, cx);
                    },
                )
                .flex_1()
                .h_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, ev: &MouseDownEvent, _, cx| {
                    let (lx, ly) = view.to_local(ev.position);
                    let (vw, vh) = view.canvas_size();
                    if ev.click_count >= 2 {
                        let world = view.camera.screen_to_world(lx, ly, vw, vh);
                        if let Some(i) = view.model.pick(world, 6.0) {
                            let id = view.model.nodes[i].id.clone();
                            view.focus_node(&id, cx);
                        }
                        return;
                    }
                    view.panning = Some(ev.position);
                    let world = view.camera.screen_to_world(lx, ly, vw, vh);
                    if let Some(i) = view.model.pick(world, 6.0) {
                        view.drag_node = Some(i);
                        view.selected = Some(i);
                        view.panning = None;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.drag_node = None;
                    view.panning = None;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _, cx| {
                let (lx, ly) = view.to_local(ev.position);
                let (vw, vh) = view.canvas_size();
                if let Some(i) = view.drag_node {
                    if let Some(n) = view.model.nodes.get_mut(i) {
                        n.pos = view.camera.screen_to_world(lx, ly, vw, vh);
                        n.vel = Vec2 { x: 0.0, y: 0.0 };
                    }
                    // Dragging wakes the layout back up; it re-settles after.
                    view.running = true;
                    view.still_ticks = 0;
                    cx.notify();
                    return;
                }
                if let Some(last) = view.panning {
                    let dx = ev.position.x - last.x;
                    let dy = ev.position.y - last.y;
                    view.camera.center_x -= f64::from(dx) / view.camera.zoom;
                    view.camera.center_y -= f64::from(dy) / view.camera.zoom;
                    view.panning = Some(ev.position);
                    cx.notify();
                    return;
                }
                let world = view.camera.screen_to_world(lx, ly, vw, vh);
                let h = view.model.pick(world, 6.0);
                if h != view.hovered {
                    view.hovered = h;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|view, ev: &ScrollWheelEvent, _, cx| {
                let (lx, ly) = view.to_local(ev.position);
                let (vw, vh) = view.canvas_size();
                let dy: f32 = match ev.delta {
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                    ScrollDelta::Lines(p) => p.y * 20.0,
                };
                let factor = if dy < 0.0 { 1.12 } else { 1.0 / 1.12 };
                view.camera.zoom_by(factor, lx, ly, vw, vh);
                cx.notify();
            }))
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme_for(window);
        self.theme_dark = theme.dark;
        // The search box is a real focusable element: click focuses it,
        // keystrokes land on it (not on a root handler that never had focus
        // — the old bug), and it autofocuses when the window opens.
        let qfocus = self.query_focus.clone();
        let qfocused = self.query_focus.is_focused(window);
        // Caret as a reverse-video highlight on the char under the cursor,
        // not an inserted `▍` glyph: inserting a glyph changes the line
        // width, so moving the cursor used to shove the text around it.
        let caret_on = qfocused && !self.query.is_empty();
        let (before, cur, after) = if caret_on {
            let mut at = self.query_cursor.min(self.query.len());
            if !self.query.is_char_boundary(at) {
                at = self.query.len();
            }
            let (b, rest) = self.query.split_at(at);
            match rest.chars().next() {
                Some(c) => (
                    b.to_string(),
                    c.to_string(),
                    rest[c.len_utf8()..].to_string(),
                ),
                // End of text: highlight a trailing space so the caret
                // is still visible without moving anything before it.
                None => (b.to_string(), " ".to_string(), String::new()),
            }
        } else if self.query.is_empty() && !qfocused {
            (
                "/rdf searches nodes · anything else is Cypher — Enter".to_string(),
                String::new(),
                String::new(),
            )
        } else if self.query.is_empty() {
            ("▍".to_string(), String::new(), String::new())
        } else {
            (self.query.clone(), String::new(), String::new())
        };
        let mut cur_el = div().child(cur);
        if caret_on {
            cur_el = cur_el
                .bg(theme.accent)
                .text_color(theme.on_accent)
                .rounded_sm();
        }
        let query_content = div()
            .flex()
            .flex_row()
            .child(before)
            .child(cur_el)
            .child(after);
        let sidebar = if self.sidebar_open {
            Some(self.render_sidebar(theme, cx))
        } else {
            None
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.render_header(theme, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar)
                    .child(self.render_divider(theme, cx))
                    .child(self.render_graph_canvas(theme, cx)),
            )
            // Compact query bar at the bottom: `/foo` searches, anything
            // else runs as Cypher. Enter submits; Escape clears.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_center()
                    .px_3()
                    .py_2()
                    .bg(theme.surface)
                    .child(
                        div()
                            .id("query-box")
                            .w(px(560.))
                            .max_w_full()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme.inset)
                            .border_1()
                            .border_color(if qfocused { theme.accent } else { theme.border })
                            .text_color(if self.query.is_empty() && !qfocused {
                                theme.secondary
                            } else {
                                theme.bright
                            })
                            .text_sm()
                            .cursor_text()
                            .track_focus(&qfocus)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, window, cx| {
                                    let h = view.query_focus.clone();
                                    window.focus(&h);
                                    cx.notify();
                                }),
                            )
                            .on_key_down(cx.listener(|view, ev: &KeyDownEvent, _, cx| {
                                let mods = &ev.keystroke.modifiers;
                                let cmd = mods.platform || mods.control;
                                match ev.keystroke.key.as_str() {
                                    "backspace" => {
                                        view.query_backspace(cmd);
                                        cx.notify();
                                    }
                                    "delete" => {
                                        view.query_delete_fwd();
                                        cx.notify();
                                    }
                                    "enter" => view.run_query(cx),
                                    "escape" => {
                                        view.query.clear();
                                        view.query_cursor = 0;
                                        cx.notify();
                                    }
                                    "left" => {
                                        view.query_move(-1);
                                        cx.notify();
                                    }
                                    "right" => {
                                        view.query_move(1);
                                        cx.notify();
                                    }
                                    "home" => {
                                        view.query_cursor = 0;
                                        cx.notify();
                                    }
                                    "end" => {
                                        view.query_cursor = view.query.len();
                                        cx.notify();
                                    }
                                    // ⌘V paste (clipboard text, flattened to
                                    // one line); ⌘C copies the whole query.
                                    "v" if cmd => {
                                        if let Some(item) = cx.read_from_clipboard() {
                                            if let Some(text) = item.text() {
                                                let flat = text
                                                    .split_whitespace()
                                                    .collect::<Vec<_>>()
                                                    .join(" ");
                                                if !flat.is_empty() {
                                                    view.query_insert(&flat);
                                                    cx.notify();
                                                }
                                            }
                                        }
                                    }
                                    "c" if cmd => {
                                        if !view.query.is_empty() {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                view.query.clone(),
                                            ));
                                        }
                                    }
                                    _ => {
                                        if let Some(ch) = ev.keystroke.key_char.clone() {
                                            if !mods.control
                                                && !mods.platform
                                                && !mods.alt
                                                && ch.chars().count() == 1
                                            {
                                                view.query_insert(&ch);
                                                cx.notify();
                                            }
                                        }
                                    }
                                }
                            }))
                            .child(query_content),
                    ),
            )
    }
}

/// Inline labels, legible at any zoom: short names on a pill behind the text.
fn truncate_label(name: &str) -> String {
    const MAX: usize = 26;
    if name.chars().count() <= MAX {
        return name.to_string();
    }
    let kept: String = name.chars().take(MAX - 1).collect();
    format!("{kept}…")
}

/// Screen-space radius for a node — shared by discs, edge trimming and labels.
fn disc_radius(degree: usize, zoom: f64) -> f32 {
    (node_size(degree) * zoom as f32).clamp(3.0, 26.0)
}

fn paint_graph(snap: &Snapshot, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    // Everything below is clipped to the canvas box: zooming never spills
    // geometry over the sidebar, header or query bar. (Text is clamped by
    // construction in paint_labels, which needs `cx` the layer can't take.)
    window.paint_layer(bounds, |window| {
        paint_edges(snap, bounds, window);
        paint_nodes(snap, bounds, window);
    });
    let _ = (vw, vh);
    paint_labels(snap, bounds, window, cx);
    paint_edge_labels(snap, bounds, window, cx);
}

/// Edge labels at the segment midpoint, tiered with the node labels:
/// hot edges always, the rest once zoomed near 1x, longest first so the
/// cap keeps labels that fit. Hidden with the discs when zoomed out.
fn paint_edge_labels(snap: &Snapshot, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let zoom = snap.camera.zoom;
    if snap.links.len() > 5000 {
        return;
    }
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let font_size = px(10.);
    let pad_x = 3.0;
    let pad_y = 1.0;
    struct Cand {
        hot: bool,
        len: f32,
        mx: f32,
        my: f32,
        label: String,
    }
    let mut cands: Vec<Cand> = Vec::new();
    for l in &snap.links {
        if l.label.is_empty() {
            continue;
        }
        let (Some(a), Some(b)) = (snap.nodes.get(l.source), snap.nodes.get(l.target)) else {
            continue;
        };
        let hot = Some(l.source) == snap.selected
            || Some(l.target) == snap.selected
            || Some(l.source) == snap.hovered
            || Some(l.target) == snap.hovered;
        if !hot && zoom < 0.9 {
            continue;
        }
        let (x0, y0) = snap.camera.world_to_screen(a.x, a.y, vw, vh);
        let (x1, y1) = snap.camera.world_to_screen(b.x, b.y, vw, vh);
        let mx = (x0 + x1) / 2.0;
        let my = (y0 + y1) / 2.0;
        let len = (x1 - x0).hypot(y1 - y0);
        if len < 48.0 || mx < 0.0 || my < 0.0 || mx > vw || my > vh {
            continue;
        }
        cands.push(Cand {
            hot,
            len,
            mx,
            my,
            label: l.label.clone(),
        });
    }
    cands.sort_by(|a, b| {
        b.hot
            .cmp(&a.hot)
            .then_with(|| b.len.partial_cmp(&a.len).unwrap())
    });
    cands.truncate(120);
    for c in cands {
        let text: SharedString = c.label.into();
        let run = TextRun {
            len: text.len(),
            font: font(".SystemUIFont"),
            color: snap.theme.secondary,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line(text, font_size, &[run], None);
        let w = f32::from(shaped.width);
        let h = f32::from(shaped.ascent + shaped.descent);
        let mut lx = c.mx - w / 2.0 - pad_x;
        lx = lx.clamp(2.0, (vw - w - pad_x * 2.0 - 2.0).max(2.0));
        let mut ly = c.my - h / 2.0 - pad_y;
        ly = ly.clamp(2.0, (vh - h - pad_y * 2.0 - 2.0).max(2.0));
        window.paint_quad(PaintQuad {
            bounds: Bounds::new(
                point(px(ox + lx), px(oy + ly)),
                size(px(w + pad_x * 2.0), px(h + pad_y * 2.0)),
            ),
            corner_radii: Corners::all(px(3.)),
            background: snap.theme.inset.opacity(0.88).into(),
            border_widths: Edges::default(),
            border_color: transparent_black(),
            border_style: BorderStyle::Solid,
        });
        let _ = shaped.paint(
            point(px(ox + lx + pad_x), px(oy + ly + pad_y)),
            px(13.),
            window,
            cx,
        );
    }
}

/// Conventional directed-graph edges: thin straight 1px strokes trimmed to
/// the rims of both endpoint discs, with a small arrowhead at the target.
/// Edges touching the selected/hovered node draw in amber, 1.5px.
fn paint_edges(snap: &Snapshot, bounds: Bounds<Pixels>, window: &mut Window) {
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let zoom = snap.camera.zoom;
    let mut plain = PathBuilder::stroke(px(1.));
    let mut hot = PathBuilder::stroke(px(1.5));
    let mut heads = PathBuilder::fill();
    let draw_heads = snap.links.len() <= 3000 && zoom >= 0.5;
    for l in &snap.links {
        let (s, t) = (l.source, l.target);
        let (Some(a), Some(b)) = (snap.nodes.get(s), snap.nodes.get(t)) else {
            continue;
        };
        let (x0, y0) = snap.camera.world_to_screen(a.x, a.y, vw, vh);
        let (x1, y1) = snap.camera.world_to_screen(b.x, b.y, vw, vh);
        if (x0 < -60.0 || x0 > vw + 60.0) && (x1 < -60.0 || x1 > vw + 60.0) {
            continue;
        }
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1.0 {
            continue;
        }
        let ux = dx / len;
        let uy = dy / len;
        let r0 = disc_radius(a.degree, zoom);
        let r1 = disc_radius(b.degree, zoom);
        // Trim to disc rims so strokes never cross a node's fill.
        let sx = ox + x0 + ux * r0;
        let sy = oy + y0 + uy * r0;
        let mut ex = ox + x1 - ux * (r1 + if draw_heads { 7.0 } else { 1.0 });
        let mut ey = oy + y1 - uy * (r1 + if draw_heads { 7.0 } else { 1.0 });
        if (ex - sx).hypot(ey - sy) < 2.0 {
            ex = ox + x1 - ux * r1;
            ey = oy + y1 - uy * r1;
        }
        let from = point(px(sx), px(sy));
        let to = point(px(ex), px(ey));
        let is_hot = Some(s) == snap.selected
            || Some(t) == snap.selected
            || Some(s) == snap.hovered
            || Some(t) == snap.hovered;
        if is_hot {
            hot.move_to(from);
            hot.line_to(to);
        } else {
            plain.move_to(from);
            plain.line_to(to);
        }
        if draw_heads {
            // Arrowhead: small filled triangle pointing along (ux, uy).
            let tip = point(px(ox + x1 - ux * r1), px(oy + y1 - uy * r1));
            let base = (r1 + 7.0, r1 + 1.5);
            let bx = ox + x1 - ux * base.0;
            let by = oy + y1 - uy * base.0;
            let w = 3.2;
            heads.move_to(tip);
            heads.line_to(point(px(bx - uy * w), px(by + ux * w)));
            heads.line_to(point(px(bx + uy * w), px(by - ux * w)));
            heads.line_to(tip);
        }
    }
    if let Ok(path) = plain.build() {
        window.paint_path(path, edge_color(&snap.theme));
    }
    if let Ok(path) = hot.build() {
        window.paint_path(path, highlight(&snap.theme));
    }
    if draw_heads {
        if let Ok(path) = heads.build() {
            window.paint_path(path, edge_color(&snap.theme));
        }
    }
}

/// Node discs with a thin inset outline; selection/hover read as a 2px
/// amber/bright ring, not a full recolor — the hue keeps meaning "label".
fn paint_nodes(snap: &Snapshot, bounds: Bounds<Pixels>, window: &mut Window) {
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    for (i, node) in snap.nodes.iter().enumerate() {
        let (sx, sy) = snap.camera.world_to_screen(node.x, node.y, vw, vh);
        if sx < -50.0 || sy < -50.0 || sx > vw + 50.0 || sy > vh + 50.0 {
            continue;
        }
        let r = disc_radius(node.degree, snap.camera.zoom);
        let (ring_width, ring_color) = if Some(i) == snap.selected {
            (px(2.), highlight(&snap.theme))
        } else if Some(i) == snap.hovered {
            (px(2.), snap.theme.bright)
        } else {
            (px(1.), snap.theme.inset.opacity(0.6))
        };
        let rad = Corners::all(px(r));
        window.paint_quad(PaintQuad {
            bounds: Bounds::new(
                point(px(ox + sx - r), px(oy + sy - r)),
                size(px(r * 2.0), px(r * 2.0)),
            ),
            corner_radii: rad,
            background: node_color(&snap.theme, node.color).into(),
            border_widths: Edges::all(ring_width),
            border_color: ring_color,
            border_style: BorderStyle::Solid,
        });
    }
}

/// Zoom-tiered inline labels, drawn after the discs so text stays on top:
/// hovered/selected always; hubs once zoomed near 1x; everything at 2x+.
/// Each label sits on a theme-inset pill so it reads over edges and nodes.
fn paint_labels(snap: &Snapshot, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let zoom = snap.camera.zoom;
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let font_size = px(11.);
    let pad_x = 4.0;
    let pad_y = 2.0;

    // Collect (index, screen_x, screen_y, radius) candidates on screen.
    let mut candidates: Vec<(usize, f32, f32, f32)> = Vec::new();
    for (i, node) in snap.nodes.iter().enumerate() {
        let (sx, sy) = snap.camera.world_to_screen(node.x, node.y, vw, vh);
        if sx < -80.0 || sy < -40.0 || sx > vw + 80.0 || sy > vh + 40.0 {
            continue;
        }
        let r = disc_radius(node.degree, zoom);
        candidates.push((i, sx, sy, r));
    }
    if candidates.is_empty() {
        return;
    }
    // Highest degree first so the cap keeps the most informative labels.
    candidates.sort_by_key(|(i, _, _, _)| std::cmp::Reverse(snap.nodes[*i].degree));
    let cap = if zoom >= 2.0 {
        220
    } else if zoom >= 0.9 {
        80
    } else {
        0
    };
    let min_degree = if zoom >= 2.0 {
        0
    } else {
        snap.nodes
            .iter()
            .map(|n| n.degree)
            .max()
            .unwrap_or(0)
            .max(2)
            / 2
            + 1
    };
    let mut labelled = 0;
    for (i, sx, sy, r) in candidates {
        let node = &snap.nodes[i];
        let pinned = Some(i) == snap.selected || Some(i) == snap.hovered;
        if !pinned && (labelled >= cap || node.degree < min_degree) {
            continue;
        }
        labelled += 1;
        // Shape FIRST, then clamp with the real measured width: clamping
        // with a character-count estimate let pills and text spill past
        // the canvas edge.
        let text: SharedString = node.name.clone().into();
        let run = TextRun {
            len: text.len(),
            font: font(".SystemUIFont"),
            color: snap.theme.bright,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line(text, font_size, &[run], None);
        let w = f32::from(shaped.width);
        let h = f32::from(shaped.ascent + shaped.descent);
        let pill_w = w + pad_x * 2.0;
        let pill_h = h + pad_y * 2.0;
        if pill_w > vw - 4.0 || pill_h > vh - 4.0 {
            continue;
        }
        let lx = (sx - pill_w / 2.0).clamp(2.0, vw - pill_w - 2.0);
        let mut ly = sy + r + 3.0;
        if ly + pill_h > vh - 2.0 {
            ly = sy - r - 3.0 - pill_h;
        }
        if ly < 2.0 {
            continue;
        }
        let pill = Bounds::new(
            point(px(ox + lx), px(oy + ly)),
            size(px(w + pad_x * 2.0), px(h + pad_y * 2.0)),
        );
        window.paint_quad(PaintQuad {
            bounds: pill,
            corner_radii: Corners::all(px(4.)),
            background: snap.theme.inset.opacity(0.88).into(),
            border_widths: Edges::default(),
            border_color: transparent_black(),
            border_style: BorderStyle::Solid,
        });
        let _ = shaped.paint(
            point(px(ox + lx + pad_x), px(oy + ly + pad_y)),
            px(14.),
            window,
            cx,
        );
    }
}
