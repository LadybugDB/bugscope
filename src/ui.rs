//! GPUI view tree — port of `App.tsx` + `HeaderBar` + `Sidebar` +
//! `QueryBox` + `GraphView` (layout, interaction, overlay).

use crate::backend::{self, DatabaseInfo, GraphData, GraphNode};
use crate::cli::CliOptions;
use crate::clusters;
use crate::decorations;
use crate::model::{node_size, Camera, GraphModel, Vec2};
use crate::theme::{edge_color, highlight, node_color, Theme};
use gpui::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone)]
struct Snapshot {
    nodes: Vec<SnapNode>,
    links: Vec<SnapLink>,
    camera: Camera,
    hovered: Option<usize>,
    selected: Option<usize>,
    /// Schema multi-selection, as model indices resolved at snapshot time:
    /// selected node types + selected edge types (⌘/Ctrl-click in schema view).
    sel_nodes: Vec<usize>,
    sel_edges: Vec<usize>,
    hover_edge: Option<usize>,
    /// Canvas-local cursor position for anchoring the hover tooltip.
    hover_pos: Option<(f32, f32)>,
    /// Prebuilt attribute lines for the hovered node/edge (built at
    /// snapshot time so per-frame paint clones no property maps).
    hover_lines: Option<Vec<String>>,
    theme: Theme,
    label_font: String,
    label_font_size: f32,
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

/// Center-pane view: force-directed graph, Leiden treemap, or Leiden
/// sunburst (same communities, radial instead of rectangular).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Graph,
    Treemap,
    Sunburst,
}

/// Startup default threshold: the first graph shown with more than this
/// many edges opens in the treemap view (the Leiden overview scales better
/// than the force graph); smaller ones open in the graph view. Applied
/// only once — afterwards the view sticks and changes only via the header
/// toggle, menu, or shortcuts.
pub const TREEMAP_AUTO_EDGES: usize = 64;

/// Default view for an edge count: treemap above the threshold.
pub fn auto_view_mode(edge_count: usize) -> ViewMode {
    if edge_count > TREEMAP_AUTO_EDGES {
        ViewMode::Treemap
    } else {
        ViewMode::Graph
    }
}

/// One step of the drill-down trail: a focused 1-hop neighborhood.
/// `label` is captured at focus time so breadcrumbs stay readable even
/// after `full` is replaced (Cypher / reload).
#[derive(Clone)]
struct Crumb {
    id: String,
    label: String,
}

/// One row of the Top PageRank list in the insights pane.
#[derive(Clone)]
struct TopRank {
    id: String,
    name: String,
    label: String,
    score: f64,
}

/// Everything the treemap needs for one frame: the Leiden communities
/// (largest first, shared with hit-testing via the same `Rc`), truncated
/// names per displayed node, and the hover/selection to outline. The
/// sunburst reuses the same snapshot plus `center_label` for its root disc.
#[derive(Clone)]
struct TreeSnapshot {
    tree: Rc<Vec<clusters::Community>>,
    names: Rc<Vec<String>>,
    theme: Theme,
    hover_node: Option<usize>,
    selected_node: Option<usize>,
    selected_community: Option<usize>,
    label_font: String,
    label_font_size: f32,
    summary: String,
    /// Root of the displayed graph for the sunburst center disc: the
    /// focused node when drilled in, else the node count.
    center_label: String,
    /// Canvas-local cursor + prebuilt attribute lines for the hovered
    /// treemap/sunburst node.
    hover_pos: Option<(f32, f32)>,
    hover_lines: Option<Vec<String>>,
}

pub struct RootView {
    databases: Vec<DatabaseInfo>,
    selected_db: Option<usize>,
    db_dir: PathBuf,
    full: GraphData,
    model: GraphModel,
    camera: Camera,
    /// Center-pane view: chosen once at startup by edge count, then sticky
    /// (header toggle / menu / shortcuts only).
    view_mode: ViewMode,
    /// Whether the one-shot startup default has been applied.
    auto_view_done: bool,
    /// Currently displayed graph, in model order: full scan, Cypher result,
    /// or 1-hop neighborhood. Analytics (Leiden/PageRank) run over this.
    shown: GraphData,
    /// Leiden community per displayed node + modularity + status line.
    communities: Vec<u64>,
    cluster_modularity: f64,
    cluster_status: String,
    /// PageRank score per displayed node, and the top 10 with names.
    scores: Vec<f64>,
    top_ranks: Vec<TopRank>,
    /// Leiden communities largest-first with truncated names, shared by
    /// paint + hit-testing so both always agree.
    tree: Rc<Vec<clusters::Community>>,
    tree_names: Rc<Vec<String>>,
    /// Displayed-node index under the treemap cursor.
    treemap_hover: Option<usize>,
    /// Collapsible right insights pane (Top PageRank + Leiden summary).
    right_open: bool,
    status: String,
    query: String,
    /// Byte offset of the text cursor inside `query` (always a char boundary).
    query_cursor: usize,
    search_results: Vec<GraphNode>,
    focused: Option<String>,
    /// Drill-down trail: focus history, empty = root (`full`).
    /// Each entry is a 1-hop `focus_node` step; breadcrumbs + `.root` /
    /// `.parent` navigate it without a DB reload.
    trail: Vec<Crumb>,
    hovered: Option<usize>,
    selected: Option<usize>,
    /// Edge under the cursor: hover highlight + attribute tooltip.
    hovered_edge: Option<usize>,
    /// Canvas-local cursor position, for anchoring the hover tooltip.
    hover_pos: Option<(f32, f32)>,
    /// Schema-view multi-selection: selected node-table and rel-table names.
    /// Multi-select clicks (⌘ on macOS, Ctrl elsewhere) collect types while
    /// staying in the schema view; a plain click adds its type and displays
    /// the accumulated subset as the data graph.
    schema_node_sel: HashSet<String>,
    schema_edge_sel: HashSet<String>,
    schema_mode: bool,
    sidebar_open: bool,
    show_preferences: bool,
    /// Open in-app menu (`"File"` / `"View"`), `None` when closed. Only
    /// used off macOS: `cx.set_menus` draws a native bar on macOS but just
    /// stores the menus on Windows/Linux, so the app draws its own there
    /// (`render_menu_bar`).
    open_menu: Option<String>,
    /// `Some(true)` = force dark, `Some(false)` = light, `None` = follow OS.
    dark_override: Option<bool>,
    /// Label typeface + size for canvas labels (Preferences window).
    label_font: String,
    label_font_size: f32,
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
        // Canonicalize so the sidebar shows a real path, never a bare ".".
        let db_dir = opts.db_dir.clone().unwrap_or_else(backend::default_db_dir);
        let db_dir = db_dir.canonicalize().unwrap_or(db_dir);
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
            view_mode: ViewMode::Graph,
            auto_view_done: false,
            shown: GraphData::default(),
            communities: Vec::new(),
            cluster_modularity: 0.0,
            cluster_status: "No graph loaded.".to_string(),
            scores: Vec::new(),
            top_ranks: Vec::new(),
            tree: Rc::new(Vec::new()),
            tree_names: Rc::new(Vec::new()),
            treemap_hover: None,
            right_open: true,
            status: "Pick a database to begin.".to_string(),
            query: String::new(),
            query_cursor: 0,
            search_results: Vec::new(),
            focused: None,
            trail: Vec::new(),
            hovered: None,
            selected: None,
            hovered_edge: None,
            hover_pos: None,
            schema_node_sel: HashSet::new(),
            schema_edge_sel: HashSet::new(),
            schema_mode: opts.schema_mode,
            sidebar_open: false,
            show_preferences: false,
            open_menu: None,
            dark_override: None,
            label_font: ".SystemUIFont".to_string(),
            label_font_size: 11.0,
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
                self.trail.clear();
                self.selected = None;
                self.hovered = None;
                self.hover_pos = None;
                self.treemap_hover = None;
                self.drag_node = None;
                self.panning = None;
                self.search_results.clear();
                self.running = !self.schema_mode;
                self.maybe_auto_view(data.links.len());
                self.shown = data;
                self.refresh_analytics();
                self.set_status(msg, cx);
            }
            Err(e) => {
                // Also stderr: window status is invisible on headless runs.
                eprintln!("bugscope: load failed: {e:#}");
                self.set_status(format!("Load failed: {e:#}"), cx)
            }
        }
    }

    /// Display name for a breadcrumb: node name when known, else the id.
    fn crumb_label(&self, node_id: &str) -> String {
        self.full
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .map(|n| truncate_label(&n.name))
            .or_else(|| {
                self.shown
                    .nodes
                    .iter()
                    .find(|n| n.id == node_id)
                    .map(|n| truncate_label(&n.name))
            })
            .unwrap_or_else(|| truncate_label(node_id))
    }

    /// Show `data` without touching the DB: the shared tail of every
    /// trail navigation (`go_root` / `go_parent` / breadcrumb jump).
    fn display(
        &mut self,
        data: GraphData,
        focused: Option<String>,
        status: String,
        cx: &mut Context<Self>,
    ) {
        self.focused = focused.clone();
        self.selected = None;
        self.hovered = None;
        self.hovered_edge = None;
        self.hover_pos = None;
        self.treemap_hover = None;
        self.drag_node = None;
        self.panning = None;
        self.model.load(&data);
        self.model.settle();
        self.frame_initial(focused.as_deref());
        self.running = !self.schema_mode && !data.nodes.is_empty();
        // Focus keeps the layout alive so the neighborhood settles
        // around the centered node; root reuses the settle above.
        if focused.is_some() {
            self.running = true;
        }
        self.maybe_auto_view(data.links.len());
        self.shown = data;
        self.refresh_analytics();
        self.set_status(status, cx);
    }

    fn db_name(&self) -> String {
        self.selected_db
            .and_then(|id| self.databases.get(id))
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "graph".to_string())
    }

    /// Re-apply the current trail after it was mutated (pop / truncate).
    /// Uses the cached `full` graph — no DB round-trip.
    fn apply_trail(&mut self, cx: &mut Context<Self>) {
        let Some(last) = self.trail.last().cloned() else {
            let data = self.full.clone();
            let msg = format!(
                "{}: {} nodes, {} edges",
                self.db_name(),
                data.nodes.len(),
                data.links.len()
            );
            self.display(data, None, msg, cx);
            return;
        };
        let view = backend::neighborhood(&self.full, &last.id);
        if view.nodes.is_empty() {
            self.set_status(format!("Node {} not in loaded graph", last.id), cx);
            return;
        }
        self.display(
            view,
            Some(last.id.clone()),
            format!("Neighborhood of {}", last.label),
            cx,
        );
    }

    /// Breadcrumb / `.root`: back to the full graph (or Cypher result).
    pub fn go_root(&mut self, cx: &mut Context<Self>) {
        if self.trail.is_empty() && self.focused.is_none() {
            self.set_status("Already at root", cx);
            return;
        }
        self.trail.clear();
        self.apply_trail(cx);
    }

    /// Breadcrumb / `.parent`: one step up the drill-down trail.
    pub fn go_parent(&mut self, cx: &mut Context<Self>) {
        if self.trail.is_empty() {
            self.set_status("Already at root", cx);
            return;
        }
        self.trail.pop();
        self.apply_trail(cx);
    }

    /// Jump to breadcrumb `index` (`None` = root).
    pub fn go_to_crumb(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        match index {
            None => self.go_root(cx),
            Some(i) if i + 1 >= self.trail.len() => {
                // Already on (or past) this crumb — no-op, don't re-settle.
            }
            Some(i) => {
                self.trail.truncate(i + 1);
                self.apply_trail(cx);
            }
        }
    }

    /// `.schema [on|off]`: deterministically enter/leave the schema view.
    /// No arg (or `on`) enters it; `off`/`data`/`graph` leaves it.
    /// The schema multi-selection survives the round-trip so types picked
    /// with the multi-select key can be extended after inspecting data.
    pub fn set_schema_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.schema_mode == enabled {
            // Already there — still reset any drill-down so `.schema`
            // always lands on the schema root.
            self.go_root(cx);
            return;
        }
        self.schema_mode = enabled;
        self.load_graph(cx);
    }

    /// Clear the schema multi-selection: database switches, manual reloads
    /// of unrelated content, and explicit resets — not the schema ⇄ data
    /// round-trip, which preserves the selection by design.
    fn clear_schema_selection(&mut self) {
        self.schema_node_sel.clear();
        self.schema_edge_sel.clear();
        self.hovered_edge = None;
    }

    /// Short `TypeA|TypeB + :RelA` summary of the schema selection for status.
    fn schema_selection_summary(&self) -> String {
        let mut nodes: Vec<&str> = self.schema_node_sel.iter().map(|s| s.as_str()).collect();
        nodes.sort_unstable();
        let mut edges: Vec<&str> = self.schema_edge_sel.iter().map(|s| s.as_str()).collect();
        edges.sort_unstable();
        let mut parts = Vec::new();
        if !nodes.is_empty() {
            parts.push(nodes.join("|"));
        }
        if !edges.is_empty() {
            parts.push(
                edges
                    .iter()
                    .map(|e| format!(":{e}"))
                    .collect::<Vec<_>>()
                    .join("|"),
            );
        }
        if parts.is_empty() {
            "schema".to_string()
        } else {
            parts.join(" + ")
        }
    }

    /// Run the subset query for the current schema selection and show the
    /// data graph, leaving schema mode. `full` becomes the subset so
    /// breadcrumbs, focus, and analytics operate on what is displayed.
    fn run_schema_subset(&mut self, cx: &mut Context<Self>) {
        let nodes: Vec<String> = self.schema_node_sel.iter().cloned().collect();
        let edges: Vec<String> = self.schema_edge_sel.iter().cloned().collect();
        if backend::schema_subset_query(&nodes, &edges, Some(self.limit)).is_none() {
            self.set_status("Schema selection empty — showing schema overview", cx);
            return;
        }
        let Some(id) = self.selected_db else { return };
        let Some(db) = self.databases.get(id) else {
            return;
        };
        let path = db.path.clone();
        let limit = self.limit;
        let work = backend::open_connection(&path)
            .and_then(|conn| backend::run_schema_subset(&conn, &nodes, &edges, limit));
        match work {
            Ok(data) => {
                let desc = self.schema_selection_summary();
                let (nn, ne) = (data.nodes.len(), data.links.len());
                self.schema_mode = false;
                self.search_results.clear();
                self.trail.clear();
                self.focused = None;
                self.selected = None;
                self.hovered = None;
                self.hovered_edge = None;
                self.hover_pos = None;
                self.treemap_hover = None;
                self.full = data.clone();
                self.model.load(&data);
                self.model.settle();
                self.frame_initial(None);
                self.running = !data.nodes.is_empty();
                self.maybe_auto_view(ne);
                self.shown = data;
                self.refresh_analytics();
                self.set_status(format!("Schema {desc}: {nn} nodes, {ne} edges"), cx);
            }
            Err(e) => {
                eprintln!("bugscope: schema query failed: {e:#}");
                self.set_status(format!("Schema query failed: {e:#}"), cx)
            }
        }
    }

    /// Plain click on a schema node type: add it to the selection and display
    /// the accumulated subset — both endpoints bound to the selected types
    /// plus their isolated nodes, so no foreign type can appear. A fresh
    /// schema view reduces to `MATCH (a:Type)-[b]->(c:Type) RETURN *`; after
    /// multi-select clicks it displays the union of everything collected.
    fn navigate_schema_node(&mut self, table: &str, cx: &mut Context<Self>) {
        self.schema_node_sel.insert(table.to_string());
        self.run_schema_subset(cx);
    }

    /// Plain click on a schema edge type: add it and display the subset.
    fn navigate_schema_edge(&mut self, rel: &str, cx: &mut Context<Self>) {
        self.schema_edge_sel.insert(rel.to_string());
        self.run_schema_subset(cx);
    }

    /// Status line describing the pending schema selection.
    fn schema_selection_status(&self) -> String {
        if self.schema_node_sel.is_empty() && self.schema_edge_sel.is_empty() {
            return "Schema selection cleared".to_string();
        }
        format!(
            "Schema selection: {} \u{2014} click a type to display, {} to collect more",
            self.schema_selection_summary(),
            multiselect_key()
        )
    }

    /// Multi-select toggle (⌘-click / Ctrl-click) on a node type: update the
    /// selection and stay in the schema view so more types can be collected.
    /// Nothing is queried until a plain click (or the sidebar Show button)
    /// displays the accumulated selection.
    fn toggle_schema_node(&mut self, table: &str, cx: &mut Context<Self>) {
        if !self.schema_node_sel.remove(table) {
            self.schema_node_sel.insert(table.to_string());
        }
        self.set_status(self.schema_selection_status(), cx);
    }

    /// Multi-select toggle (⌘-click / Ctrl-click) on an edge type: collect
    /// without leaving the schema view.
    fn toggle_schema_edge(&mut self, rel: &str, cx: &mut Context<Self>) {
        if !self.schema_edge_sel.remove(rel) {
            self.schema_edge_sel.insert(rel.to_string());
        }
        self.set_status(self.schema_selection_status(), cx);
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
                // A different file has a different schema: drop the selection.
                view.clear_schema_selection();
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

    /// Header toggle + menu actions for the center-pane view.
    pub fn set_view_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.view_mode = mode;
        cx.notify();
    }

    /// One-shot startup default: pick the view from the edge count the
    /// first time a graph is shown; later graphs keep the current view.
    fn maybe_auto_view(&mut self, edge_count: usize) {
        if !self.auto_view_done {
            self.view_mode = auto_view_mode(edge_count);
            self.auto_view_done = true;
        }
    }

    pub fn toggle_right_pane(&mut self, cx: &mut Context<Self>) {
        self.right_open = !self.right_open;
        cx.notify();
    }

    /// Recompute Leiden communities + PageRank over the displayed graph.
    /// Runs synchronously: loaders cap the graph at 10–20k nodes, where
    /// both algorithms finish in well under a second.
    fn refresh_analytics(&mut self) {
        let n = self.shown.nodes.len();
        if n == 0 {
            self.communities.clear();
            self.scores.clear();
            self.top_ranks.clear();
            self.cluster_modularity = 0.0;
            self.cluster_status = "No graph loaded.".to_string();
            self.tree = Rc::new(Vec::new());
            self.tree_names = Rc::new(Vec::new());
            self.treemap_hover = None;
            return;
        }
        let (assignment, modularity, _) = backend::graphr_leiden_full(&self.shown);
        self.communities = assignment;
        self.cluster_modularity = modularity;
        let mut distinct = self.communities.clone();
        distinct.sort_unstable();
        distinct.dedup();
        self.cluster_status = format!(
            "{} Leiden communities · Q={:.2}",
            distinct.len(),
            modularity
        );
        let ranks = backend::graphr_page_rank(&self.shown);
        let score_of: std::collections::HashMap<&str, f64> =
            ranks.iter().map(|(id, s)| (id.as_str(), *s)).collect();
        self.scores = self
            .shown
            .nodes
            .iter()
            .map(|nd| score_of.get(nd.id.as_str()).copied().unwrap_or(0.0))
            .collect();
        let by_id: std::collections::HashMap<&str, &GraphNode> = self
            .shown
            .nodes
            .iter()
            .map(|nd| (nd.id.as_str(), nd))
            .collect();
        self.top_ranks = ranks
            .into_iter()
            .take(10)
            .filter_map(|(id, score)| {
                by_id.get(id.as_str()).map(|nd| TopRank {
                    id: id.clone(),
                    name: nd.name.clone(),
                    label: nd.label.clone(),
                    score,
                })
            })
            .collect();
        let tree = clusters::build_communities(n, &self.communities, Some(&self.scores));
        let names = self
            .shown
            .nodes
            .iter()
            .map(|nd| truncate_label(&nd.name))
            .collect();
        self.tree = Rc::new(tree);
        self.tree_names = Rc::new(names);
        if self.treemap_hover.map(|i| i >= n).unwrap_or(false) {
            self.treemap_hover = None;
        }
        if self.selected.map(|i| i >= n).unwrap_or(false) {
            self.selected = None;
        }
    }

    /// Squarified tiles for a viewport, shared by paint + hit-testing.
    fn tree_tiles(vw: f32, vh: f32, tree: &[clusters::Community]) -> Vec<clusters::Tile> {
        clusters::layout_treemap(
            tree,
            clusters::Rect::new(0.0, 0.0, vw.max(50.0), vh.max(50.0)),
            5.0,
            20.0,
        )
    }

    /// Deepest treemap tile under a canvas-local point.
    fn pick_treemap(&self, lx: f32, ly: f32) -> Option<clusters::Tile> {
        let (vw, vh) = self.canvas_size();
        Self::tree_tiles(vw, vh, &self.tree)
            .into_iter()
            .rev()
            .find(|t| t.rect.contains(lx, ly))
    }

    /// Sunburst wedges for the current canvas, shared by paint +
    /// hit-testing so both always agree.
    fn sunburst_layout(&self) -> (Vec<clusters::SunburstWedge>, f32, f32, f32) {
        let (vw, vh) = self.canvas_size();
        let (cx, cy, radius) = clusters::sunburst_frame(vw, vh);
        let wedges = clusters::layout_sunburst(&self.tree, radius);
        (wedges, cx, cy, radius)
    }

    /// Sunburst hit under a canvas-local point: center circle, wedge, or
    /// miss (outside the disc).
    fn pick_sunburst(&self, lx: f32, ly: f32) -> Option<clusters::SunburstHit> {
        let (wedges, cx, cy, radius) = self.sunburst_layout();
        clusters::pick_sunburst(&wedges, cx, cy, radius, lx, ly)
    }

    /// Displayed-node index for a sunburst hit: the member arc itself, or
    /// the heaviest member when the community arc (or center) was hit.
    fn sunburst_hit_node(&self, hit: clusters::SunburstHit) -> Option<usize> {
        match hit {
            clusters::SunburstHit::Center => None,
            clusters::SunburstHit::Wedge { community, node } => node.or_else(|| {
                self.tree
                    .get(community)
                    .and_then(|c| c.members.first().copied())
            }),
        }
    }

    /// Displayed-node index shown for a tile: the member cell itself, or
    /// the heaviest member when the community header was hit.
    fn treemap_tile_node(&self, tile: &clusters::Tile) -> Option<usize> {
        tile.node.or_else(|| {
            self.tree
                .get(tile.community)
                .and_then(|c| c.members.first().copied())
        })
    }

    fn tree_snapshot(&self, theme: Theme) -> TreeSnapshot {
        let selected_community = self
            .selected
            .and_then(|i| self.tree.iter().position(|c| c.members.contains(&i)));
        let center_label = self
            .trail
            .last()
            .map(|c| c.label.clone())
            .unwrap_or_else(|| format!("{} nodes", self.shown.nodes.len()));
        let hover_lines = self.treemap_hover.and_then(|i| {
            self.model.nodes.get(i).map(|n| {
                node_tooltip_lines(
                    &n.name,
                    &n.id,
                    &n.label,
                    n.degree,
                    &sorted_props(&n.properties),
                )
            })
        });
        TreeSnapshot {
            tree: self.tree.clone(),
            names: self.tree_names.clone(),
            theme,
            hover_node: self.treemap_hover,
            selected_node: self.selected,
            selected_community,
            label_font: self.label_font.clone(),
            label_font_size: self.label_font_size,
            summary: self.cluster_status.clone(),
            center_label,
            hover_pos: self.hover_pos,
            hover_lines,
        }
    }

    pub fn toggle_preferences(&mut self, cx: &mut Context<Self>) {
        self.show_preferences = !self.show_preferences;
        cx.notify();
    }

    /// Friendly display name: the default family starts with a literal
    /// dot, which reads as a stray glyph in UI text.
    pub fn label_font_display(&self) -> &str {
        self.label_font
            .trim_start_matches('.')
            .split(' ')
            .next()
            .unwrap_or(&self.label_font)
    }

    pub fn set_label_font(&mut self, name: &str, cx: &mut Context<Self>) {
        self.label_font = name.to_string();
        cx.notify();
    }

    pub fn bump_label_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.label_font_size = (self.label_font_size + delta).clamp(8.0, 24.0);
        cx.notify();
    }

    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.query.clear();
        self.query_cursor = 0;
        self.search_results.clear();
        self.clear_schema_selection();
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

    /// Enter in the query bar: dot-commands (`.root` / `.parent` /
    /// `.schema`), `/foo` node search, or Cypher (replaces the graph).
    /// Bare `/` clears text-search state (matches + search zoom).
    fn run_query(&mut self, cx: &mut Context<Self>) {
        let raw = self.query.clone();
        let raw = raw.trim().to_string();
        if raw.is_empty() {
            return;
        }
        // Dot-commands first — they never need a database handle.
        if raw.starts_with('.') {
            self.run_dot_command(&raw, cx);
            return;
        }
        let Some(id) = self.selected_db else { return };
        let Some(db) = self.databases.get(id) else {
            return;
        };
        let is_search = raw.starts_with('/');
        let mut term = raw.trim_start_matches('/').trim().to_string();
        // Accept both `/rdf` and `/search rdf` (the old hint text
        // suggested the latter).
        if let Some(rest) = term.strip_prefix("search") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                term = rest.trim().to_string();
            }
        }
        if is_search && term.is_empty() {
            // Bare `/`: clear matches + any search zoom, back to root.
            self.clear_text_search(cx);
            self.query.clear();
            self.query_cursor = 0;
            cx.notify();
            return;
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
                    // A new text search starts from the root so matches
                    // are in full-graph context: drop any previous
                    // search-driven zoom/selection first.
                    if self.focused.is_some() || !self.trail.is_empty() {
                        self.trail.clear();
                        let root = self.full.clone();
                        let msg = format!(
                            "{}: {} nodes, {} edges",
                            self.db_name(),
                            root.nodes.len(),
                            root.links.len()
                        );
                        self.display(root, None, msg, cx);
                    } else {
                        self.selected = None;
                        self.hovered = None;
                        self.hover_pos = None;
                        self.hovered_edge = None;
                        self.treemap_hover = None;
                    }
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
                    self.trail.clear();
                    self.focused = None;
                    self.selected = None;
                    self.hovered = None;
                    self.hovered_edge = None;
                    self.hover_pos = None;
                    self.treemap_hover = None;
                    // A hand-written query replaces the graph: any schema
                    // multi-selection it may have come from is stale.
                    self.clear_schema_selection();
                    self.full = data.clone();
                    self.model.load(&data);
                    self.model.settle();
                    self.frame_initial(None);
                    self.running = !data.nodes.is_empty() && !self.schema_mode;
                    let (nn, ne) = (data.nodes.len(), data.links.len());
                    self.maybe_auto_view(ne);
                    self.shown = data;
                    self.refresh_analytics();
                    self.set_status(format!("Cypher: {nn} nodes, {ne} edges"), cx);
                }
            }
            Err(e) => {
                eprintln!("bugscope: query failed: {e:#}");
                self.set_status(format!("Query failed: {e:#}"), cx)
            }
        }
    }

    /// `.root` / `.parent` / `.schema` / `.data` — query-box navigation commands.
    /// Unknown `.foo` reports the valid set instead of running Cypher.
    fn run_dot_command(&mut self, raw: &str, cx: &mut Context<Self>) {
        let mut parts = raw.split_whitespace();
        let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
        let arg = parts.next().unwrap_or("").to_ascii_lowercase();
        let clear_input = |view: &mut Self, cx: &mut Context<Self>| {
            view.query.clear();
            view.query_cursor = 0;
            cx.notify();
        };
        match cmd.as_str() {
            ".root" => {
                self.go_root(cx);
                clear_input(self, cx);
            }
            ".parent" | ".up" | ".back" => {
                self.go_parent(cx);
                clear_input(self, cx);
            }
            ".schema" => {
                let enabled = match arg.as_str() {
                    "" | "on" | "schema" => true,
                    "off" | "data" | "graph" | "no" => false,
                    "toggle" => !self.schema_mode,
                    _ => {
                        self.set_status(
                            format!("Unknown .schema arg {arg:?} — try .schema, .schema on/off"),
                            cx,
                        );
                        return;
                    }
                };
                self.set_schema_mode(enabled, cx);
                clear_input(self, cx);
            }
            // First-class way back from `.schema`: `.data` == `.schema off`.
            ".data" | ".graph" => {
                if !arg.is_empty() {
                    self.set_status(format!("Unknown .data arg {arg:?} — try .data"), cx);
                    return;
                }
                self.set_schema_mode(false, cx);
                clear_input(self, cx);
            }
            _ => {
                self.set_status(
                    "Unknown command — try .root, .parent, .data, .schema".to_string(),
                    cx,
                );
            }
        }
    }

    /// Bare-`/` behavior: drop text-search matches + any zoom/selection
    /// so the next `/foo` starts from a clean root view.
    fn clear_text_search(&mut self, cx: &mut Context<Self>) {
        self.search_results.clear();
        if self.trail.is_empty() && self.focused.is_none() {
            self.selected = None;
            self.hovered = None;
            self.hovered_edge = None;
            self.hover_pos = None;
            self.treemap_hover = None;
            self.set_status("Search cleared", cx);
            return;
        }
        self.trail.clear();
        let root = self.full.clone();
        let msg = format!(
            "{}: {} nodes, {} edges — search cleared",
            self.db_name(),
            root.nodes.len(),
            root.links.len()
        );
        self.display(root, None, msg, cx);
    }

    fn focus_node(&mut self, node_id: &str, cx: &mut Context<Self>) {
        let view = backend::neighborhood(&self.full, node_id);
        if view.nodes.is_empty() {
            self.set_status(format!("Node {node_id} not in loaded graph"), cx);
            return;
        }
        // Push the drill-down step (skip duplicates from double-clicks).
        // The label is captured now so breadcrumbs survive later `full`
        // replacements (Cypher / reload).
        if self
            .trail
            .last()
            .map(|c| c.id.as_str() != node_id)
            .unwrap_or(true)
        {
            let label = self.crumb_label(node_id);
            self.trail.push(Crumb {
                id: node_id.to_string(),
                label: label.clone(),
            });
            self.display(
                view,
                Some(node_id.to_string()),
                format!("Neighborhood of {label}"),
                cx,
            );
        } else {
            self.display(
                view,
                Some(node_id.to_string()),
                format!(
                    "Neighborhood of {}",
                    self.trail
                        .last()
                        .map(|c| c.label.clone())
                        .unwrap_or_else(|| node_id.to_string())
                ),
                cx,
            );
        }
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
        // Resolve schema multi-selection (type names) to model indices for
        // this frame. Only meaningful while the schema graph is displayed.
        let (sel_nodes, sel_edges) = if self.schema_mode {
            let nodes = self
                .model
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| self.schema_node_sel.contains(&n.name))
                .map(|(i, _)| i)
                .collect();
            let edges = self
                .model
                .links
                .iter()
                .enumerate()
                .filter(|(_, l)| self.schema_edge_sel.contains(&l.label))
                .map(|(i, _)| i)
                .collect();
            (nodes, edges)
        } else {
            (Vec::new(), Vec::new())
        };
        // Attribute lines for the hovered node/edge, built once per frame
        // from the model (node hover wins over edge hover).
        let hover_lines = self
            .hovered
            .and_then(|i| {
                self.model.nodes.get(i).map(|n| {
                    node_tooltip_lines(
                        &n.name,
                        &n.id,
                        &n.label,
                        n.degree,
                        &sorted_props(&n.properties),
                    )
                })
            })
            .or_else(|| {
                self.hovered_edge.and_then(|e| {
                    self.model.links.get(e).map(|l| {
                        let (from_name, from_id) = self
                            .model
                            .nodes
                            .get(l.source)
                            .map(|n| (n.name.as_str(), n.id.as_str()))
                            .unwrap_or(("", ""));
                        let (to_name, to_id) = self
                            .model
                            .nodes
                            .get(l.target)
                            .map(|n| (n.name.as_str(), n.id.as_str()))
                            .unwrap_or(("", ""));
                        edge_tooltip_lines(
                            &l.label,
                            from_name,
                            from_id,
                            to_name,
                            to_id,
                            &sorted_props(&l.properties),
                        )
                    })
                })
            });
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
            sel_nodes,
            sel_edges,
            hover_edge: self.hovered_edge,
            hover_pos: self.hover_pos,
            hover_lines,
            label_font: self.label_font.clone(),
            label_font_size: self.label_font_size,
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

    /// In-app menu bar for Windows/Linux: `cx.set_menus` (see `main.rs`)
    /// only draws a native bar on macOS — on other platforms GPUI stores
    /// the menus without displaying anything, so without this the File/View
    /// commands would be unreachable there. Mirrors the native menus item
    /// for item (plus Preferences/Quit, which live in the app menu on macOS
    /// and have no native home off it). Returns `None` on macOS.
    fn render_menu_bar(&mut self, theme: Theme, cx: &mut Context<Self>) -> Option<Div> {
        if cfg!(target_os = "macos") {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(28.))
                .px_2()
                .gap_1()
                .bg(theme.surface)
                .border_b_1()
                .border_color(theme.border)
                .text_color(theme.foreground)
                .text_sm()
                .child(self.menu_dropdown("File", theme, cx))
                .child(self.menu_dropdown("View", theme, cx)),
        )
    }

    /// One top-level menu: a title button plus its dropdown panel while
    /// open. The panel is `deferred` + `anchored` so it paints above the
    /// header/canvas below instead of underneath them; `on_mouse_down_out`
    /// on the wrapper closes the menu when anything else is clicked (panel
    /// picks still run — they close the menu themselves after acting).
    fn menu_dropdown(&mut self, name: &str, theme: Theme, cx: &mut Context<Self>) -> Div {
        let open = self.open_menu.as_deref() == Some(name);
        let mut title = div()
            .px_2()
            .py(px(2.))
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(theme.selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let name = name.to_string();
                    move |view, _, _, cx| {
                        if view.open_menu.as_deref() == Some(&name) {
                            view.open_menu = None;
                        } else {
                            view.open_menu = Some(name.clone());
                        }
                        cx.notify();
                    }
                }),
            )
            .child(name.to_string());
        if open {
            title = title.bg(theme.selection);
        }
        let mut wrap = div()
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                if view.open_menu.is_some() {
                    view.open_menu = None;
                    cx.notify();
                }
            }))
            .child(title);
        if open {
            let panel = match name {
                "File" => self.file_menu_panel(theme, cx),
                _ => self.view_menu_panel(theme, cx),
            };
            wrap = wrap.child(
                deferred(anchored().anchor(Corner::TopLeft).child(panel)),
            );
        }
        wrap
    }

    /// Shared dropdown panel styling for the in-app menu bar.
    fn menu_panel(theme: Theme) -> Div {
        div()
            .flex()
            .flex_col()
            .min_w(px(250.))
            .py_1()
            .rounded_md()
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .text_color(theme.foreground)
            .shadow(vec![BoxShadow {
                color: hsla(0., 0., 0., 0.4),
                blur_radius: px(12.),
                spread_radius: px(0.),
                offset: point(px(0.), px(2.)),
            }])
    }

    /// One clickable menu row: label left, shortcut hint right. Runs the
    /// action and closes the menu.
    fn menu_row(
        label: &str,
        shortcut: Option<&str>,
        theme: Theme,
        cx: &mut Context<Self>,
        on_pick: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_6()
            .mx_1()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(theme.selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, _, cx| {
                    view.open_menu = None;
                    on_pick(view, cx);
                }),
            )
            .child(label.to_string())
            .children(shortcut.map(|s| {
                div()
                    .text_xs()
                    .text_color(theme.secondary)
                    .child(s.to_string())
            }))
    }

    fn menu_separator(theme: Theme) -> Div {
        div().h(px(1.)).my_1().mx_3().bg(theme.border)
    }

    /// `File` dropdown: same entries as the native File menu, plus the
    /// Preferences/Quit entries that live in the app menu on macOS.
    fn file_menu_panel(&mut self, theme: Theme, cx: &mut Context<Self>) -> Div {
        Self::menu_panel(theme)
            .child(Self::menu_row(
                "Open Database…",
                Some("Ctrl+O"),
                theme,
                cx,
                |view, cx| view.open_file_dialog(cx),
            ))
            .child(Self::menu_row(
                "Reload Graph",
                Some("Ctrl+R"),
                theme,
                cx,
                |view, cx| view.load_graph(cx),
            ))
            .child(Self::menu_separator(theme))
            .child(Self::menu_row(
                "Toggle Schema View",
                None,
                theme,
                cx,
                |view, cx| view.toggle_schema(cx),
            ))
            .child(Self::menu_separator(theme))
            .child(Self::menu_row(
                "Preferences…",
                Some("Ctrl+,"),
                theme,
                cx,
                |view, cx| view.toggle_preferences(cx),
            ))
            .child(Self::menu_row("Quit", None, theme, cx, |_view, cx| {
                cx.quit()
            }))
    }

    /// `View` dropdown: same entries as the native View menu.
    fn view_menu_panel(&mut self, theme: Theme, cx: &mut Context<Self>) -> Div {
        Self::menu_panel(theme)
            .child(Self::menu_row(
                "Pause/Resume Layout",
                None,
                theme,
                cx,
                |view, cx| view.toggle_layout(cx),
            ))
            .child(Self::menu_row(
                "Toggle Light/Dark Theme",
                None,
                theme,
                cx,
                |view, cx| view.toggle_theme(cx),
            ))
            .child(Self::menu_row(
                "Toggle Sidebar",
                Some("Ctrl+B"),
                theme,
                cx,
                |view, cx| view.toggle_sidebar(cx),
            ))
            .child(Self::menu_row(
                "Toggle Insights Pane",
                Some("Ctrl+I"),
                theme,
                cx,
                |view, cx| view.toggle_right_pane(cx),
            ))
            .child(Self::menu_separator(theme))
            .child(Self::menu_row(
                "Graph View",
                Some("Ctrl+1"),
                theme,
                cx,
                |view, cx| view.set_view_mode(ViewMode::Graph, cx),
            ))
            .child(Self::menu_row(
                "Treemap (Leiden) View",
                Some("Ctrl+2"),
                theme,
                cx,
                |view, cx| view.set_view_mode(ViewMode::Treemap, cx),
            ))
            .child(Self::menu_row(
                "Sunburst (Leiden) View",
                Some("Ctrl+3"),
                theme,
                cx,
                |view, cx| view.set_view_mode(ViewMode::Sunburst, cx),
            ))
            .child(Self::menu_separator(theme))
            .child(Self::menu_row("Reset View", None, theme, cx, |view, cx| {
                view.reset_view(cx)
            }))
    }

    fn render_header(&mut self, theme: Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        let db_name = self
            .selected_db
            .and_then(|id| self.databases.get(id))
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "no database".to_string());
        let status = self.status.clone();
        // The theme button flips between the two palettes; the glyph shows
        // what you get when you click (sun on dark, moon on light).
        let theme_button = if theme.dark { "☀" } else { "☾" };
        // Data ↔ schema toggle: same control language as the view toggle.
        // Active side reads as a filled pill; the other as a quiet button.
        let mut schema_toggle = div().flex().flex_row().gap_1();
        for (label, enabled) in [("Data", false), ("Schema", true)] {
            let active = self.schema_mode == enabled;
            schema_toggle = schema_toggle.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .bg(if active {
                        theme.accent
                    } else {
                        theme.selection
                    })
                    .text_color(if active {
                        theme.on_accent
                    } else {
                        theme.foreground
                    })
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| view.set_schema_mode(enabled, cx)),
                    )
                    .child(label.to_string()),
            );
        }
        // View toggle: graph ↔ Leiden treemap ↔ Leiden sunburst. The active
        // view reads as a filled pill; the others as quiet buttons.
        let mut toggle = div().flex().flex_row().gap_1();
        for (label, mode) in [
            ("Graph", ViewMode::Graph),
            ("Treemap", ViewMode::Treemap),
            ("Sunburst", ViewMode::Sunburst),
        ] {
            let active = self.view_mode == mode;
            toggle = toggle.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .bg(if active {
                        theme.accent
                    } else {
                        theme.selection
                    })
                    .text_color(if active {
                        theme.on_accent
                    } else {
                        theme.foreground
                    })
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| view.set_view_mode(mode, cx)),
                    )
                    .child(label.to_string()),
            );
        }
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .bg(theme.surface)
            .text_color(theme.foreground)
            .child(decorations::drag_area(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(div().font_weight(FontWeight::BOLD).child("Bugscope"))
                    .child(div().text_sm().child(format!("{db_name} · {status}"))),
                window,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(schema_toggle)
                    .child(toggle)
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
                    )
                    .children(decorations::window_buttons(theme, window)),
            )
    }

    /// Breadcrumb bar under the header: `Root › A › B` for the drill-down
    /// trail. Clicking a crumb jumps there (cached, no DB reload); the
    /// trailing `↑` / `⟲` shortcuts mirror `.parent` / `.root`.
    fn render_breadcrumbs(&mut self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let trail = self.trail.clone();
        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_3()
            .py_1()
            .bg(theme.surface)
            .text_color(theme.foreground)
            .text_xs();
        // Root crumb: active pill at root, clickable shortcut above it.
        let at_root = trail.is_empty();
        let root_el = div()
            .px_2()
            .py(px(2.))
            .rounded_md()
            .bg(if at_root {
                theme.accent
            } else {
                theme.selection
            })
            .text_color(if at_root {
                theme.on_accent
            } else {
                theme.foreground
            })
            .cursor_pointer()
            .hover(|s| s.bg(if at_root { theme.accent } else { theme.border }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.go_root(cx)),
            )
            .child("Root".to_string());
        row = row.child(root_el);
        for (i, crumb) in trail.iter().enumerate() {
            let last = i + 1 == trail.len();
            row = row.child(div().text_color(theme.secondary).child("›".to_string()));
            let label = crumb.label.clone();
            let el = div()
                .px_2()
                .py(px(2.))
                .rounded_md()
                .bg(if last { theme.accent } else { theme.selection })
                .text_color(if last {
                    theme.on_accent
                } else {
                    theme.foreground
                })
                .cursor_pointer()
                .hover(|s| s.bg(if last { theme.accent } else { theme.border }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| view.go_to_crumb(Some(i), cx)),
                )
                .child(label);
            row = row.child(el);
        }
        if self.schema_mode {
            // Clickable way back: the badge leaves the schema view.
            row = row.child(
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .bg(theme.accent)
                    .text_color(theme.on_accent)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.border))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| view.set_schema_mode(false, cx)),
                    )
                    .child("schema ✕".to_string()),
            );
        }
        if !at_root {
            row = row.child(div().flex_1());
            row = row.child(
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .bg(theme.selection)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.border))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| view.go_parent(cx)),
                    )
                    .child("↑ Parent".to_string()),
            );
            row = row.child(
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .bg(theme.selection)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.border))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| view.go_root(cx)),
                    )
                    .child("⟲ Root".to_string()),
            );
        }
        row
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
                            view.clear_schema_selection();
                            view.load_graph(cx);
                        }),
                    )
                    .child(d.name.clone()),
            );
        }
        if self.schema_mode {
            col = self.render_schema_section(col, theme, cx);
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

    /// Sidebar browser for the schema view: every node-table and rel-table
    /// with the same click semantics as the canvas (plain click navigates
    /// by that single type, multi-select key toggles it in the subset).
    /// Edge types that are hard to hit on the canvas are easy to pick here.
    fn render_schema_section(
        &mut self,
        mut col: Stateful<Div>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        col = col
            .child(
                div()
                    .pt_2()
                    .font_weight(FontWeight::BOLD)
                    .child("Schema types"),
            )
            .child(div().text_xs().text_color(theme.secondary).child(format!(
                "Click displays the subset \u{b7} {} collects types first",
                multiselect_key()
            )));
        let mut node_types: Vec<String> = self.shown.nodes.iter().map(|n| n.name.clone()).collect();
        node_types.sort();
        node_types.dedup();
        let mut edge_types: Vec<String> = self
            .shown
            .links
            .iter()
            .map(|l| l.label.clone())
            .filter(|l| !l.is_empty())
            .collect();
        edge_types.sort();
        edge_types.dedup();
        if !node_types.is_empty() {
            col = col.child(
                div()
                    .pt_1()
                    .text_xs()
                    .text_color(theme.secondary)
                    .child("Nodes"),
            );
        }
        for t in node_types {
            let active = self.schema_node_sel.contains(&t);
            let name = t.clone();
            col = col.child(
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .cursor_pointer()
                    .bg(if active { theme.accent } else { theme.surface })
                    .text_color(if active {
                        theme.on_accent
                    } else {
                        theme.foreground
                    })
                    .hover(|s| {
                        s.bg(if active {
                            theme.accent
                        } else {
                            theme.selection
                        })
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, ev: &MouseDownEvent, _, cx| {
                            if ev.modifiers.platform || ev.modifiers.control {
                                view.toggle_schema_node(&name.clone(), cx);
                            } else {
                                view.navigate_schema_node(&name.clone(), cx);
                            }
                        }),
                    )
                    .child(t),
            );
        }
        if !edge_types.is_empty() {
            col = col.child(
                div()
                    .pt_1()
                    .text_xs()
                    .text_color(theme.secondary)
                    .child("Edges"),
            );
        }
        for t in edge_types {
            let active = self.schema_edge_sel.contains(&t);
            let name = t.clone();
            let row = format!(":{t}");
            col = col.child(
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .cursor_pointer()
                    .bg(if active { theme.accent } else { theme.surface })
                    .text_color(if active {
                        theme.on_accent
                    } else {
                        theme.foreground
                    })
                    .hover(|s| {
                        s.bg(if active {
                            theme.accent
                        } else {
                            theme.selection
                        })
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, ev: &MouseDownEvent, _, cx| {
                            if ev.modifiers.platform || ev.modifiers.control {
                                view.toggle_schema_edge(&name.clone(), cx);
                            } else {
                                view.navigate_schema_edge(&name.clone(), cx);
                            }
                        }),
                    )
                    .child(row),
            );
        }
        if !self.schema_node_sel.is_empty() || !self.schema_edge_sel.is_empty() {
            let summary = self.schema_selection_summary();
            col = col.child(
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
                        cx.listener(|view, _, _, cx| view.run_schema_subset(cx)),
                    )
                    .child(format!("Show {summary}")),
            );
            col = col.child(
                div()
                    .px_2()
                    .py_1()
                    .mt_1()
                    .rounded_md()
                    .bg(theme.selection)
                    .cursor_pointer()
                    .text_sm()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.clear_schema_selection();
                            view.set_status("Schema selection cleared", cx);
                        }),
                    )
                    .child("Clear selection"),
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

    fn render_graph_canvas(&mut self, theme: Theme, cx: &mut Context<Self>) -> Div {
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
                    let world = view.camera.screen_to_world(lx, ly, vw, vh);
                    // Schema view: clicks navigate by type. A plain click adds
                    // the type to the selection and displays the accumulated
                    // subset; the platform multi-select key (⌘ on macOS, Ctrl
                    // elsewhere) only collects the type and stays here so more
                    // can follow. Double-click navigates like a plain click
                    // (schema nodes have no 1-hop data focus).
                    if view.schema_mode {
                        let multi = ev.modifiers.platform || ev.modifiers.control;
                        let node = view.model.pick(world, 6.0);
                        let edge = if node.is_none() {
                            view.model.pick_edge(world, 8.0)
                        } else {
                            None
                        };
                        if multi {
                            if let Some(i) = node {
                                let t = view.model.nodes[i].name.clone();
                                view.toggle_schema_node(&t, cx);
                            } else if let Some(e) = edge {
                                let t = view.model.links[e].label.clone();
                                view.toggle_schema_edge(&t, cx);
                            }
                            return;
                        }
                        if let Some(i) = node {
                            let t = view.model.nodes[i].name.clone();
                            view.navigate_schema_node(&t, cx);
                        } else if let Some(e) = edge {
                            let t = view.model.links[e].label.clone();
                            view.navigate_schema_edge(&t, cx);
                        } else {
                            // Empty space: pan, like the data view.
                            view.panning = Some(ev.position);
                            cx.notify();
                        }
                        return;
                    }
                    if ev.click_count >= 2 {
                        if let Some(i) = view.model.pick(world, 6.0) {
                            let id = view.model.nodes[i].id.clone();
                            view.focus_node(&id, cx);
                        }
                        return;
                    }
                    view.panning = Some(ev.position);
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
                // Track the edge under the cursor whenever no node is hit,
                // so hovering a relationship shows its attributes (and, in
                // the schema view, highlights the rel type before click).
                // Tolerance is screen-constant (~10px) via zoom scaling.
                let tol = (10.0 / view.camera.zoom.max(0.2)) as f32;
                let he = if h.is_none() {
                    view.model.pick_edge(world, tol)
                } else {
                    None
                };
                // Cursor position only matters while something is hovered
                // (tooltip anchor); otherwise leave it empty so empty-canvas
                // moves don't re-render every frame.
                let pos = if h.is_some() || he.is_some() {
                    Some((lx, ly))
                } else {
                    None
                };
                if h != view.hovered || he != view.hovered_edge || pos != view.hover_pos {
                    view.hovered = h;
                    view.hovered_edge = he;
                    view.hover_pos = pos;
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

    /// Divider gutter between the canvas and the insights pane, mirroring
    /// the left sidebar divider. Always rendered so the pane can be
    /// reopened when hidden.
    fn render_right_divider(&mut self, theme: Theme, cx: &mut Context<Self>) -> Div {
        // Chevron points the way the bar will go.
        let glyph = if self.right_open { "»" } else { "«" };
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
                        cx.listener(|view, _, _, cx| view.toggle_right_pane(cx)),
                    )
                    .child(glyph),
            )
            .child(line())
    }

    /// Collapsible right pane: top 10 nodes by PageRank over the displayed
    /// graph, plus the Leiden community summary. Clicking a row focuses the
    /// node's 1-hop neighborhood; clicking a community selects its heaviest
    /// member (the view sticks — switch it via the header toggle).
    fn render_insights(&mut self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .w(px(260.))
            .h_full()
            .id("insights")
            .overflow_y_scroll()
            .bg(theme.surface)
            .text_color(theme.foreground)
            .text_sm()
            .child(div().font_weight(FontWeight::BOLD).child("Insights"))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.secondary)
                    .child(self.cluster_status.clone()),
            )
            .child(
                div()
                    .pt_1()
                    .font_weight(FontWeight::BOLD)
                    .child("Top PageRank"),
            );
        if self.top_ranks.is_empty() {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(theme.secondary)
                    .child("Load a graph to rank nodes."),
            );
        }
        for (rank, top) in self.top_ranks.clone().into_iter().enumerate() {
            let id = top.id.clone();
            let row = format!("#{} {} · {}", rank + 1, top.name, top.label);
            let score = format!("{:.4}", top.score);
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
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
                    .child(div().flex_1().child(row))
                    .child(div().text_xs().text_color(theme.secondary).child(score)),
            );
        }
        col = col.child(
            div()
                .pt_2()
                .font_weight(FontWeight::BOLD)
                .child("Leiden communities"),
        );
        if self.tree.is_empty() {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(theme.secondary)
                    .child("No communities yet."),
            );
        }
        for (pos, comm) in self.tree.iter().enumerate() {
            let first = comm.members.first().copied();
            let label = format!("C{} · {} nodes", pos + 1, comm.members.len());
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.selection))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            if let Some(m) = first {
                                view.selected = Some(m);
                                cx.notify();
                            }
                        }),
                    )
                    .child(
                        div()
                            .w(px(10.))
                            .h(px(10.))
                            .rounded_sm()
                            .bg(node_color(&theme, pos)),
                    )
                    .child(div().flex_1().child(label)),
            );
        }
        col
    }

    fn render_treemap_canvas(&mut self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let snap = self.tree_snapshot(theme);
        let origin_cell = self.canvas_origin.clone();
        let size_cell = self.canvas_size.clone();
        div()
            .flex_1()
            .h_full()
            .bg(theme.inset)
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        origin_cell.set(bounds.origin);
                        size_cell.set(bounds.size);
                        (snap, bounds)
                    },
                    move |_bounds, (snap, bounds), window, cx| {
                        paint_treemap(&snap, bounds, window, cx);
                    },
                )
                .flex_1()
                .h_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, ev: &MouseDownEvent, _, cx| {
                    let (lx, ly) = view.to_local(ev.position);
                    let Some(tile) = view.pick_treemap(lx, ly) else {
                        view.treemap_hover = None;
                        cx.notify();
                        return;
                    };
                    let node = view.treemap_tile_node(&tile);
                    // Schema view: tiles are node types — click navigates by
                    // type instead of selecting/focusing data nodes.
                    if view.schema_mode {
                        let table = node
                            .and_then(|i| view.shown.nodes.get(i))
                            .map(|n| n.name.clone());
                        if let Some(t) = table {
                            if ev.modifiers.platform || ev.modifiers.control {
                                view.toggle_schema_node(&t, cx);
                            } else {
                                view.navigate_schema_node(&t, cx);
                            }
                        }
                        return;
                    }
                    if ev.click_count >= 2 {
                        if let Some(i) = node {
                            if let Some(nd) = view.shown.nodes.get(i) {
                                let id = nd.id.clone();
                                view.focus_node(&id, cx);
                            }
                        }
                        return;
                    }
                    view.selected = node;
                    view.treemap_hover = node;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _, cx| {
                let (lx, ly) = view.to_local(ev.position);
                let h = view
                    .pick_treemap(lx, ly)
                    .and_then(|t| view.treemap_tile_node(&t));
                let pos = if h.is_some() { Some((lx, ly)) } else { None };
                if h != view.treemap_hover || pos != view.hover_pos {
                    view.treemap_hover = h;
                    view.hover_pos = pos;
                    cx.notify();
                }
            }))
    }

    fn render_sunburst_canvas(&mut self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let snap = self.tree_snapshot(theme);
        let origin_cell = self.canvas_origin.clone();
        let size_cell = self.canvas_size.clone();
        div()
            .flex_1()
            .h_full()
            .bg(theme.inset)
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        origin_cell.set(bounds.origin);
                        size_cell.set(bounds.size);
                        (snap, bounds)
                    },
                    move |_bounds, (snap, bounds), window, cx| {
                        paint_sunburst(&snap, bounds, window, cx);
                    },
                )
                .flex_1()
                .h_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, ev: &MouseDownEvent, _, cx| {
                    let (lx, ly) = view.to_local(ev.position);
                    let Some(hit) = view.pick_sunburst(lx, ly) else {
                        view.treemap_hover = None;
                        cx.notify();
                        return;
                    };
                    // Center circle steps one level up the trail.
                    if hit == clusters::SunburstHit::Center {
                        if !view.trail.is_empty() {
                            view.go_parent(cx);
                        } else {
                            view.treemap_hover = None;
                            cx.notify();
                        }
                        return;
                    }
                    let node = view.sunburst_hit_node(hit);
                    // Schema view: arcs are node types — navigate by type.
                    if view.schema_mode {
                        let table = node
                            .and_then(|i| view.shown.nodes.get(i))
                            .map(|n| n.name.clone());
                        if let Some(t) = table {
                            if ev.modifiers.platform || ev.modifiers.control {
                                view.toggle_schema_node(&t, cx);
                            } else {
                                view.navigate_schema_node(&t, cx);
                            }
                        }
                        return;
                    }
                    if ev.click_count >= 2 {
                        if let Some(i) = node {
                            if let Some(nd) = view.shown.nodes.get(i) {
                                let id = nd.id.clone();
                                view.focus_node(&id, cx);
                            }
                        }
                        return;
                    }
                    view.selected = node;
                    view.treemap_hover = node;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _, cx| {
                let (lx, ly) = view.to_local(ev.position);
                let h = view
                    .pick_sunburst(lx, ly)
                    .and_then(|hit| view.sunburst_hit_node(hit));
                let pos = if h.is_some() { Some((lx, ly)) } else { None };
                if h != view.treemap_hover || pos != view.hover_pos {
                    view.treemap_hover = h;
                    view.hover_pos = pos;
                    cx.notify();
                }
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
                "/foo search · / clears · .root .parent .schema .data · else Cypher — Enter"
                    .to_string(),
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
        let center = match self.view_mode {
            ViewMode::Graph => self.render_graph_canvas(theme, cx),
            ViewMode::Treemap => self.render_treemap_canvas(theme, cx),
            ViewMode::Sunburst => self.render_sunburst_canvas(theme, cx),
        };
        let insights = if self.right_open {
            Some(self.render_insights(theme, cx))
        } else {
            None
        };
        let root = div()
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            // In-app menu bar on Windows/Linux (None on macOS, which has
            // the native bar). Must come before the header so tab order
            // and layout match the platform convention.
            .children(self.render_menu_bar(theme, cx))
            .child(self.render_header(theme, window, cx))
            .child(self.render_breadcrumbs(theme, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar)
                    .child(self.render_divider(theme, cx))
                    .child(center)
                    .child(self.render_right_divider(theme, cx))
                    .children(insights),
            )
            // Compact query bar at the bottom: `/foo` searches, bare `/`
            // clears, `.root` / `.parent` / `.schema` / `.data` navigate, anything
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
            .children(self.render_preferences_modal(theme, cx));
        decorations::client_frame(root, theme, window)
    }
}

impl RootView {
    /// Native-style Preferences panel (app menu → Preferences…, ⌘,): a
    /// centered modal with the label typeface + size controls. Rendered on
    /// every platform from the same menu entry.
    fn render_preferences_modal(&mut self, theme: Theme, cx: &mut Context<Self>) -> Option<Div> {
        if !self.show_preferences {
            return None;
        }
        let mut chips = div().flex().flex_row().flex_wrap().gap_1();
        for name in [".SystemUIFont", "Helvetica Neue", "Menlo", "Georgia"] {
            let active = self.label_font == name;
            let short = name
                .trim_start_matches('.')
                .split(' ')
                .next()
                .unwrap_or(name);
            let name_owned = name.to_string();
            chips = chips.child(
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .cursor_pointer()
                    .text_xs()
                    .bg(if active { theme.accent } else { theme.surface })
                    .hover(|s| s.bg(theme.selection))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.set_label_font(&name_owned.clone(), cx);
                        }),
                    )
                    .child(short.to_string()),
            );
        }
        let panel = div()
            .w(px(360.))
            .rounded_lg()
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .text_color(theme.foreground)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child("Preferences"),
            )
            .child(div().font_weight(FontWeight::BOLD).child("Label font"))
            .child(div().text_xs().text_color(theme.secondary).child(format!(
                "{} · {:.0}pt",
                self.label_font_display(),
                self.label_font_size
            )))
            .child(chips)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded_md()
                            .cursor_pointer()
                            .bg(theme.inset)
                            .hover(|s| s.bg(theme.selection))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, _, cx| view.bump_label_font_size(-1.0, cx)),
                            )
                            .child("A−"),
                    )
                    .child(format!("{:.0}pt", self.label_font_size))
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded_md()
                            .cursor_pointer()
                            .bg(theme.inset)
                            .hover(|s| s.bg(theme.selection))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, _, cx| view.bump_label_font_size(1.0, cx)),
                            )
                            .child("A+"),
                    ),
            )
            .child(
                div().flex().flex_row().justify_end().child(
                    div()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(theme.accent)
                        .hover(|s| s.bg(theme.selection))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, _, cx| view.toggle_preferences(cx)),
                        )
                        .child("Close"),
                ),
            );
        Some(
            div()
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(black().opacity(0.45))
                .child(panel),
        )
    }
}

/// Human label for the platform multi-select modifier used by schema
/// navigation: ⌘ on macOS, Ctrl everywhere else.
fn multiselect_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘-click"
    } else {
        "Ctrl-click"
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

/// Sorted `(key, value)` pairs for deterministic hover tooltips.
fn sorted_props(map: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// One attribute value on a single tooltip line.
fn truncate_attr_value(s: &str) -> String {
    const MAX: usize = 48;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let kept: String = s.chars().take(MAX - 1).collect();
    format!("{kept}…")
}

/// Attribute lines for a hovered node: title + identity + properties.
/// First line is the title; the painter renders it emphasized.
fn node_tooltip_lines(
    name: &str,
    id: &str,
    label: &str,
    degree: usize,
    props: &[(String, String)],
) -> Vec<String> {
    const MAX_PROPS: usize = 12;
    let mut lines = Vec::new();
    lines.push(if name.is_empty() {
        id.to_string()
    } else {
        name.to_string()
    });
    lines.push(format!("id: {id}"));
    if !label.is_empty() {
        lines.push(format!("type: {label}"));
    }
    lines.push(format!("connections: {degree}"));
    for (k, v) in props.iter().take(MAX_PROPS) {
        lines.push(format!("{}: {}", k, truncate_attr_value(v)));
    }
    if props.len() > MAX_PROPS {
        lines.push(format!("… +{} more", props.len() - MAX_PROPS));
    }
    lines
}

/// Attribute lines for a hovered edge: rel type + endpoints + properties.
fn edge_tooltip_lines(
    rel: &str,
    from_name: &str,
    from_id: &str,
    to_name: &str,
    to_id: &str,
    props: &[(String, String)],
) -> Vec<String> {
    const MAX_PROPS: usize = 10;
    let mut lines = Vec::new();
    lines.push(if rel.is_empty() {
        "relationship".to_string()
    } else {
        format!(":{rel}")
    });
    lines.push(format!(
        "from: {}",
        truncate_attr_value(if from_name.is_empty() {
            from_id
        } else {
            from_name
        })
    ));
    lines.push(format!(
        "to: {}",
        truncate_attr_value(if to_name.is_empty() { to_id } else { to_name })
    ));
    for (k, v) in props.iter().take(MAX_PROPS) {
        lines.push(format!("{}: {}", k, truncate_attr_value(v)));
    }
    if props.len() > MAX_PROPS {
        lines.push(format!("… +{} more", props.len() - MAX_PROPS));
    }
    lines
}

/// Hover attribute tooltip core: bg card + one shaped line per row,
/// anchored near the cursor and clamped to the canvas box.
#[allow(clippy::too_many_arguments)]
fn paint_tooltip_lines(
    theme: &Theme,
    label_font: &String,
    label_font_size: f32,
    lines: &[String],
    anchor_local: (f32, f32),
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    const MAX_LINES: usize = 16;
    if lines.is_empty() {
        return;
    }
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let font_size = px(label_font_size.max(8.0));
    let pad_x = 8.0;
    let pad_y = 6.0;
    let gap = 1.0;
    let take = lines.len().min(MAX_LINES);
    // Shape first so the card fits the real text.
    let mut shaped: Vec<(gpui::ShapedLine, f32, f32)> = Vec::with_capacity(take);
    let mut card_w = 0.0f32;
    let mut card_h = pad_y * 2.0;
    for (i, line) in lines.iter().take(take).enumerate() {
        let text: SharedString = line.clone().into();
        let run = TextRun {
            len: text.len(),
            font: font(label_font),
            color: if i == 0 {
                theme.bright
            } else {
                theme.foreground
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let s = window
            .text_system()
            .shape_line(text, font_size, &[run], None);
        let w = f32::from(s.width);
        let h = f32::from(s.ascent + s.descent);
        card_w = card_w.max(w);
        card_h += h + if i + 1 < take { gap } else { 0.0 };
        shaped.push((s, w, h));
    }
    card_w += pad_x * 2.0;
    if card_w > vw - 8.0 || card_h > vh - 8.0 {
        return;
    }
    // Cursor-anchored, flipped when it would spill past the edge.
    let mut tx = anchor_local.0 + 14.0;
    if tx + card_w > vw - 4.0 {
        tx = anchor_local.0 - card_w - 12.0;
    }
    let mut ty = anchor_local.1 + 16.0;
    if ty + card_h > vh - 4.0 {
        ty = anchor_local.1 - card_h - 12.0;
    }
    tx = tx.clamp(4.0, (vw - card_w - 4.0).max(4.0));
    ty = ty.clamp(4.0, (vh - card_h - 4.0).max(4.0));
    window.paint_quad(PaintQuad {
        bounds: Bounds::new(
            point(px(ox + tx), px(oy + ty)),
            size(px(card_w), px(card_h)),
        ),
        corner_radii: Corners::all(px(6.)),
        background: theme.surface.into(),
        border_widths: Edges::all(px(1.)),
        border_color: theme.border,
        border_style: BorderStyle::Solid,
    });
    let mut y = ty + pad_y;
    for (s, _, h) in &shaped {
        let _ = s.paint(point(px(ox + tx + pad_x), px(oy + y)), px(14.), window, cx);
        y += *h + gap;
    }
}

/// Graph-view hover tooltip from the prebuilt snapshot lines. Falls back
/// to the hovered node position / edge midpoint when the cursor position
/// is unavailable.
fn paint_hover_tooltip(snap: &Snapshot, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let Some(lines) = snap.hover_lines.as_ref() else {
        return;
    };
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    let anchor = snap.hover_pos.unwrap_or_else(|| {
        if let Some(n) = snap.hovered.and_then(|i| snap.nodes.get(i)) {
            snap.camera.world_to_screen(n.x, n.y, vw, vh)
        } else if let Some(l) = snap.hover_edge.and_then(|e| snap.links.get(e)) {
            let (Some(a), Some(b)) = (snap.nodes.get(l.source), snap.nodes.get(l.target)) else {
                return (vw / 2.0, vh / 2.0);
            };
            let (x0, y0) = snap.camera.world_to_screen(a.x, a.y, vw, vh);
            let (x1, y1) = snap.camera.world_to_screen(b.x, b.y, vw, vh);
            ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
        } else {
            (vw / 2.0, vh / 2.0)
        }
    });
    paint_tooltip_lines(
        &snap.theme,
        &snap.label_font,
        snap.label_font_size,
        lines,
        anchor,
        bounds,
        window,
        cx,
    );
}

/// Treemap / sunburst hover tooltip from the prebuilt snapshot lines.
#[allow(clippy::too_many_arguments)]
fn paint_tree_tooltip(
    theme: &Theme,
    label_font: &String,
    label_font_size: f32,
    lines: &Option<Vec<String>>,
    anchor: &Option<(f32, f32)>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let (Some(ls), Some(pos)) = (lines.as_ref(), anchor.as_ref()) else {
        return;
    };
    paint_tooltip_lines(
        theme,
        label_font,
        label_font_size,
        ls,
        *pos,
        bounds,
        window,
        cx,
    );
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
    paint_hover_tooltip(snap, bounds, window, cx);
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
    let font_size = px((snap.label_font_size - 1.0).max(8.0));
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
            font: font(&snap.label_font),
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
    for (li, l) in snap.links.iter().enumerate() {
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
            || Some(t) == snap.hovered
            || snap.sel_edges.contains(&li)
            || Some(li) == snap.hover_edge;
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
        // Schema multi-selection reads as the same amber ring as the
        // single selection — the hue keeps meaning "chosen type".
        let (ring_width, ring_color) = if Some(i) == snap.selected || snap.sel_nodes.contains(&i) {
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
    let font_size = px(snap.label_font_size);
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
            font: font(&snap.label_font),
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

/// Treemap of the Leiden communities, disktree-style: tiles are painted, not
/// composed from elements (thousands of rects would spend the frame in
/// layout). Each community gets a header band with its name over member cells
/// tinted by community hue; the selected node's community reads as an amber
/// ring on every one of its cells, like the graph view's hot edges.
fn paint_treemap(snap: &TreeSnapshot, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let font_size = px(snap.label_font_size);

    let tiles = RootView::tree_tiles(vw, vh, &snap.tree);
    if tiles.is_empty() {
        // Empty state: centered hint instead of a blank canvas.
        let text: SharedString = if snap.tree.is_empty() {
            snap.summary.clone().into()
        } else {
            "Treemap tiles too small — load a smaller graph or resize.".into()
        };
        let run = TextRun {
            len: text.len(),
            font: font(&snap.label_font),
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
        let _ = shaped.paint(
            point(px(ox + (vw - w) / 2.0), px(oy + (vh - h) / 2.0)),
            px(14.),
            window,
            cx,
        );
        return;
    }

    // Fills first; outlines after every fill so member cells never cover
    // their community's selection ring (same ordering trick as disktree).
    let mut outlines: Vec<(u8, Bounds<Pixels>, f32, Hsla)> = Vec::new();
    for tile in &tiles {
        let fill = node_color(&snap.theme, tile.community);
        let quad_bounds = Bounds::new(
            point(px(ox + tile.rect.x), px(oy + tile.rect.y)),
            size(px(tile.rect.w), px(tile.rect.h)),
        );
        if tile.header {
            window.paint_quad(PaintQuad {
                bounds: quad_bounds,
                corner_radii: Corners::all(px(3.)),
                background: snap.theme.surface.into(),
                border_widths: Edges::default(),
                border_color: transparent_black(),
                border_style: BorderStyle::Solid,
            });
            // Community strip: a thin slab of the hue across the top, so the
            // first level of structure reads before any detail.
            let strip = Bounds::new(
                quad_bounds.origin,
                size(
                    quad_bounds.size.width,
                    px(2.0f32.min(f32::from(quad_bounds.size.height))),
                ),
            );
            window.paint_quad(PaintQuad {
                bounds: strip,
                corner_radii: Corners::default(),
                background: fill.into(),
                border_widths: Edges::default(),
                border_color: transparent_black(),
                border_style: BorderStyle::Solid,
            });
        } else {
            window.paint_quad(PaintQuad {
                bounds: quad_bounds,
                corner_radii: Corners::all(px(3.)),
                background: fill.into(),
                border_widths: Edges::default(),
                border_color: transparent_black(),
                border_style: BorderStyle::Solid,
            });
        }
        let node_here = tile.node.filter(|n| Some(*n) == snap.hover_node);
        let ring = Bounds::new(
            point(px(ox + tile.rect.x), px(oy + tile.rect.y)),
            size(px(tile.rect.w), px(tile.rect.h)),
        );
        let outline = if tile.node.is_some() && tile.node == snap.selected_node {
            Some((3u8, ring, 2.0, snap.theme.bright))
        } else if node_here.is_some() {
            Some((2, ring, 1.0, snap.theme.bright))
        } else if Some(tile.community) == snap.selected_community {
            Some((1, ring, 1.5, highlight(&snap.theme)))
        } else {
            None
        };
        if let Some((rank, ring, width, color)) = outline {
            outlines.push((rank, ring, width, color));
        }
    }
    outlines.sort_by_key(|(rank, ..)| *rank);
    for (_, ring, width, color) in outlines {
        window.paint_quad(PaintQuad {
            bounds: ring,
            corner_radii: Corners::all(px(3.)),
            background: transparent_black().into(),
            border_widths: Edges::all(px(width)),
            border_color: color,
            border_style: BorderStyle::Solid,
        });
    }

    // Labels: every header names its community; member cells label when the
    // text fits, largest first so the cap keeps informative names.
    let mut cells: Vec<&clusters::Tile> = tiles
        .iter()
        .filter(|t| !t.header && t.node.is_some())
        .collect();
    cells.sort_by(|a, b| {
        b.rect
            .area()
            .partial_cmp(&a.rect.area())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for tile in &tiles {
        if !tile.header {
            continue;
        }
        let count = snap
            .tree
            .get(tile.community)
            .map(|c| c.members.len())
            .unwrap_or(0);
        let label = format!("C{} · {count} nodes", tile.community + 1);
        paint_tree_text(
            snap,
            &label,
            ox + tile.rect.x + 6.0,
            oy + tile.rect.y + 3.0,
            tile.rect.w - 12.0,
            snap.theme.bright,
            window,
            cx,
        );
    }
    for tile in cells.into_iter().take(150) {
        if tile.rect.w < 56.0 || tile.rect.h < 18.0 {
            continue;
        }
        let Some(i) = tile.node else { continue };
        let Some(name) = snap.names.get(i) else {
            continue;
        };
        paint_tree_text(
            snap,
            name,
            ox + tile.rect.x + 5.0,
            oy + tile.rect.y + 3.0,
            tile.rect.w - 10.0,
            snap.theme.bright,
            window,
            cx,
        );
    }

    // Summary pill, bottom-left over the mosaic.
    let text: SharedString = snap.summary.clone().into();
    let run = TextRun {
        len: text.len(),
        font: font(&snap.label_font),
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
    let pad_x = 6.0;
    let pad_y = 3.0;
    window.paint_quad(PaintQuad {
        bounds: Bounds::new(
            point(px(ox + 8.0), px(oy + vh - h - pad_y * 2.0 - 8.0)),
            size(px(w + pad_x * 2.0), px(h + pad_y * 2.0)),
        ),
        corner_radii: Corners::all(px(4.)),
        background: snap.theme.inset.opacity(0.88).into(),
        border_widths: Edges::default(),
        border_color: transparent_black(),
        border_style: BorderStyle::Solid,
    });
    let _ = shaped.paint(
        point(
            px(ox + 8.0 + pad_x),
            px(oy + vh - h - pad_y * 2.0 - 8.0 + pad_y),
        ),
        px(14.),
        window,
        cx,
    );
    paint_tree_tooltip(
        &snap.theme,
        &snap.label_font,
        snap.label_font_size,
        &snap.hover_lines,
        &snap.hover_pos,
        bounds,
        window,
        cx,
    );
}

/// One treemap label, painted only when the shaped text fits its tile —
/// nothing may bleed into the neighbour.
#[allow(clippy::too_many_arguments)]
fn paint_tree_text(
    snap: &TreeSnapshot,
    label: &str,
    x: f32,
    y: f32,
    max_w: f32,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    if max_w < 20.0 {
        return;
    }
    let text: SharedString = label.to_string().into();
    let run = TextRun {
        len: text.len(),
        font: font(&snap.label_font),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(text, px(snap.label_font_size), &[run], None);
    if f32::from(shaped.width) > max_w {
        return;
    }
    let _ = shaped.paint(point(px(x), px(y)), px(14.), window, cx);
}

/// Polygon for a sunburst wedge: outer arc sampled forward, inner arc back.
/// Canvas-local points (the caller offsets by the paint origin).
fn sunburst_polygon(w: &clusters::SunburstWedge, cx: f32, cy: f32) -> Vec<Point<Pixels>> {
    let steps = ((w.span() / 0.06).ceil() as usize).clamp(3, 64);
    let mut pts = Vec::with_capacity(2 * (steps + 1));
    for i in 0..=steps {
        let a = w.start + w.span() * i as f32 / steps as f32;
        pts.push(point(
            px(cx + w.outer * a.cos()),
            px(cy + w.outer * a.sin()),
        ));
    }
    for i in (0..=steps).rev() {
        let a = w.start + w.span() * i as f32 / steps as f32;
        pts.push(point(
            px(cx + w.inner * a.cos()),
            px(cy + w.inner * a.sin()),
        ));
    }
    pts
}

/// Bostock-style tint: members share their community hue, fading lighter
/// as member rank falls so adjacent arcs stay distinguishable.
fn member_shade(base: Hsla, rank: usize, total: usize) -> Hsla {
    let t = if total > 1 {
        rank as f32 / (total - 1) as f32
    } else {
        0.0
    };
    Hsla {
        h: base.h,
        s: (base.s * (1.0 - 0.25 * t)).max(0.0),
        l: (base.l + 0.14 * t).min(0.92),
        a: base.a,
    }
}

/// Sunburst of the Leiden communities: the same hierarchy as the treemap,
/// radial instead of rectangular. The center disc is the root of the
/// displayed graph; the inner ring holds one arc per community and the
/// outer ring fans out community members, each slice proportional to its
/// PageRank weight. Clicking the center steps one level up the drill-down
/// trail. Wedges are painted as filled arc polygons (not composed
/// elements), with selection/hover as stroked outlines on top.
fn paint_sunburst(snap: &TreeSnapshot, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let vw = f32::from(bounds.size.width);
    let vh = f32::from(bounds.size.height);
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let font_size = px(snap.label_font_size);

    let (mcx, mcy, radius) = clusters::sunburst_frame(vw, vh);
    let wedges = clusters::layout_sunburst(&snap.tree, radius);
    // Canvas-local → absolute.
    let placed: Vec<Vec<Point<Pixels>>> = wedges
        .iter()
        .map(|w| {
            sunburst_polygon(w, mcx, mcy)
                .into_iter()
                .map(|p| point(px(f32::from(p.x) + ox), px(f32::from(p.y) + oy)))
                .collect()
        })
        .collect();
    if wedges.is_empty() {
        let text: SharedString = if snap.tree.is_empty() {
            snap.summary.clone().into()
        } else {
            "Sunburst arcs too small — load a smaller graph or resize.".into()
        };
        let run = TextRun {
            len: text.len(),
            font: font(&snap.label_font),
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
        let _ = shaped.paint(
            point(px(ox + (vw - w) / 2.0), px(oy + (vh - h) / 2.0)),
            px(14.),
            window,
            cx,
        );
        return;
    }

    // Root disc: the whole displayed graph, with an accent rim. This is
    // what the center-click "up" affordance points at.
    let root_r = radius * clusters::SUNBURST_CENTER_FRAC;
    let mut disc_pts = Vec::with_capacity(65);
    for k in 0..=64 {
        let a = k as f32 / 64.0 * std::f32::consts::TAU;
        disc_pts.push(point(
            px(ox + mcx + root_r * a.cos()),
            px(oy + mcy + root_r * a.sin()),
        ));
    }
    let mut disc_path = PathBuilder::fill();
    disc_path.add_polygon(&disc_pts, true);
    if let Ok(path) = disc_path.build() {
        window.paint_path(path, snap.theme.surface);
    }
    let mut disc_rim = PathBuilder::stroke(px(1.5));
    disc_rim.add_polygon(&disc_pts, true);
    if let Ok(path) = disc_rim.build() {
        window.paint_path(path, snap.theme.accent);
    }
    paint_centered_text(
        snap,
        snap.theme.foreground,
        &snap.center_label,
        ox + mcx,
        oy + mcy,
        root_r * 1.6,
        window,
        cx,
    );

    // Fills first, then separators, then selection strokes, so rings never
    // cover their outlines. Member rank within each community drives the
    // tint gradient (layout emits members heaviest-first).
    let mut member_rank = vec![0usize; snap.tree.len()];
    let mut strokes: Vec<(u8, usize, f32, Hsla)> = Vec::new();
    for (i, w) in wedges.iter().enumerate() {
        let base = node_color(&snap.theme, w.community);
        let fill = match w.node {
            None => base,
            Some(_) => {
                let total = snap
                    .tree
                    .get(w.community)
                    .map(|c| c.members.len())
                    .unwrap_or(1);
                let rank = member_rank[w.community].min(total.saturating_sub(1));
                member_rank[w.community] += 1;
                member_shade(base, rank, total)
            }
        };
        let mut fill_path = PathBuilder::fill();
        fill_path.add_polygon(&placed[i], true);
        if let Ok(path) = fill_path.build() {
            window.paint_path(path, fill);
        }
        let node_here = w.node.filter(|n| Some(*n) == snap.hover_node);
        let stroke = if w.node.is_some() && w.node == snap.selected_node {
            Some((3u8, i, 2.0, snap.theme.bright))
        } else if node_here.is_some() {
            Some((2, i, 1.0, snap.theme.bright))
        } else if Some(w.community) == snap.selected_community {
            Some((1, i, 1.5, highlight(&snap.theme)))
        } else {
            None
        };
        if let Some(s) = stroke {
            strokes.push(s);
        }
    }
    // Hairline separators between slices, like the reference's gaps.
    for pts in &placed {
        let mut sep = PathBuilder::stroke(px(1.));
        sep.add_polygon(pts, true);
        if let Ok(path) = sep.build() {
            window.paint_path(path, snap.theme.inset);
        }
    }
    strokes.sort_by_key(|(rank, ..)| *rank);
    for (_, i, width, color) in strokes {
        let mut stroke_path = PathBuilder::stroke(px(width));
        stroke_path.add_polygon(&placed[i], true);
        if let Ok(path) = stroke_path.build() {
            window.paint_path(path, color);
        }
    }

    // Labels, centered on each wedge: every readable community arc, then
    // the largest member arcs first so the cap keeps informative names.
    for w in wedges.iter().filter(|w| w.node.is_none()) {
        if w.arc_len() < 48.0 {
            continue;
        }
        let count = snap
            .tree
            .get(w.community)
            .map(|c| c.members.len())
            .unwrap_or(0);
        let label = format!("C{} · {count}", w.community + 1);
        let a = w.mid_angle();
        let r = w.mid_radius();
        paint_centered_text(
            snap,
            snap.theme.bright,
            &label,
            ox + mcx + r * a.cos(),
            oy + mcy + r * a.sin(),
            w.arc_len().min((w.outer - w.inner) * 2.0),
            window,
            cx,
        );
    }
    let mut members: Vec<&clusters::SunburstWedge> =
        wedges.iter().filter(|w| w.node.is_some()).collect();
    members.sort_by(|a, b| {
        b.arc_len()
            .partial_cmp(&a.arc_len())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for w in members.into_iter().take(60) {
        if w.arc_len() < 56.0 {
            continue;
        }
        let Some(i) = w.node else { continue };
        let Some(name) = snap.names.get(i) else {
            continue;
        };
        let a = w.mid_angle();
        let r = w.mid_radius();
        paint_centered_text(
            snap,
            snap.theme.bright,
            name,
            ox + mcx + r * a.cos(),
            oy + mcy + r * a.sin(),
            w.arc_len() - 8.0,
            window,
            cx,
        );
    }

    // Summary pill, bottom-left over the disc.
    let text: SharedString = snap.summary.clone().into();
    let run = TextRun {
        len: text.len(),
        font: font(&snap.label_font),
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
    let pad_x = 6.0;
    let pad_y = 3.0;
    window.paint_quad(PaintQuad {
        bounds: Bounds::new(
            point(px(ox + 8.0), px(oy + vh - h - pad_y * 2.0 - 8.0)),
            size(px(w + pad_x * 2.0), px(h + pad_y * 2.0)),
        ),
        corner_radii: Corners::all(px(4.)),
        background: snap.theme.inset.opacity(0.88).into(),
        border_widths: Edges::default(),
        border_color: transparent_black(),
        border_style: BorderStyle::Solid,
    });
    let _ = shaped.paint(
        point(
            px(ox + 8.0 + pad_x),
            px(oy + vh - h - pad_y * 2.0 - 8.0 + pad_y),
        ),
        px(14.),
        window,
        cx,
    );
    paint_tree_tooltip(
        &snap.theme,
        &snap.label_font,
        snap.label_font_size,
        &snap.hover_lines,
        &snap.hover_pos,
        bounds,
        window,
        cx,
    );
}

/// One sunburst label, centered on its wedge and painted only when the
/// shaped text fits — nothing may bleed into the neighbour.
#[allow(clippy::too_many_arguments)]
fn paint_centered_text(
    snap: &TreeSnapshot,
    color: Hsla,
    label: &str,
    cx_px: f32,
    cy_px: f32,
    max_w: f32,
    window: &mut Window,
    cx: &mut App,
) {
    if max_w < 20.0 {
        return;
    }
    let text: SharedString = label.to_string().into();
    let run = TextRun {
        len: text.len(),
        font: font(&snap.label_font),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(text, px(snap.label_font_size), &[run], None);
    let w = f32::from(shaped.width);
    let h = f32::from(shaped.ascent + shaped.descent);
    if w > max_w {
        return;
    }
    let _ = shaped.paint(
        point(px(cx_px - w / 2.0), px(cy_px - h / 2.0)),
        px(14.),
        window,
        cx,
    );
}
