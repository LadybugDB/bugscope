# Bugscope Graph Visualizer

An interactive graph visualization tool for LadybugDB. Explore relationships between bugs, files, and other entities in your databases through an intuitive visual interface.

This is the native port of [bugscope-tauri](https://github.com/LadybugDB/bugscope-tauri): no Tauri, no React, no webview. A single Rust binary with LadybugDB in-process and a native [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) window.

## Features

- **Interactive Graph View** - Navigate through connected data using a force-directed graph. Drag nodes to rearrange, zoom in/out, and pan around the canvas.
- **Database Selection** - Choose from available LadybugDB databases in the sidebar, or use **Open file…** to browse for any `*.lbdb` file.
- **Visual Encoding** - Node size reflects connection count (more connections = larger nodes), and colors differentiate entity types.
- **Dark/Light Mode** - Follows the system appearance automatically, same themes as the Tauri app.
- **Relationship Labels** - Hover over edges to see the type of relationship between connected nodes.
- **Search + Focus** - Substring search over node properties; click a match to focus its 1-hop neighborhood.
- **Schema View** - Toggle to see node tables instead of the edge graph.
- **Live Layout** - Force simulation (repulsion + springs + damping) runs at 30 Hz and settles when the layout goes quiet; pause/resume any time.

## Usage

1. Select a database from the sidebar on the left (scanned from the working directory), or click **Open file…** to pick a `*.lbdb` anywhere on disk
2. The graph will load and display nodes (entities) and edges (relationships)
3. Click and drag nodes to rearrange the layout
4. Scroll to zoom in/out (zooms at the cursor), click and drag the canvas to pan
5. Hover over nodes to see their labels
6. Hover over edges to see relationship types
7. Double-click a node (or select it and choose "Expand neighborhood") to focus its 1-hop neighborhood
8. Type in the query box and press Enter to search; click a match to focus it

## Prerequisites

Just the Rust toolchain ([rustup](https://rustup.rs)). No Node.js, no Tauri CLI, no WebKit/GTK system libraries, no vendored sigma.js fork — the UI is native GPUI, not a webview.

On macOS: Xcode command line tools (for any Rust build).

## Run

```bash
cargo run --release
```

Working directory matters only for the initial sidebar scan: `*.lbdb` files under `.` are listed automatically. Anything else can be opened via **Open file…**, so you can run from anywhere.

## Project structure

| Path | Contents |
|---|---|
| `src/main.rs` | App entry: window setup, titlebar, autofocus |
| `src/backend.rs` | Direct LadybugDB backend — `open_connection`, `scan_for_databases`, `collect_edge_graph`, `collect_schema_graph`, `search_nodes`, `neighborhood` |
| `src/model.rs` | `GraphModel` (force layout tick, palette, picking) + `Camera` (world↔screen, cursor-anchored zoom, fit) |
| `src/ui.rs` | `RootView` — sidebar, query box, canvas rendering, interaction |
| `src/theme.rs` | Vendored disktree theme (`tokyo_night` dark / `flexoki_light` light), selected via `window.appearance()` like disktree follows the system setting |

The backend runs the same Cypher as the Tauri commands (`MATCH (a)-[r]->(b) RETURN a, r, b`, isolated-node scan, `CALL SHOW_TABLES`, node sampling) with direct `lbug` calls and no IPC boundary.

## Deliberately out of scope for v1

Summary-space PageRank sidecars, Leiden cluster levels and LLM cluster naming, Voronoi overlay, the lever panel, Arrow IPC transport — all were web-renderer or sidecar concerns in bugscope-tauri. The native port loads the edge graph directly and lays it out live. Cluster/color extensions can build on `GraphModel` without an IPC boundary.

### Troubleshooting

- Window doesn't appear on launch - older versions scanned `$HOME` recursively on the UI thread before first paint and hung. Current versions only scan the working directory; update and retry.
- `Load failed: ...` in the status bar - the picked file isn't a readable LadybugDB database, or it's locked by another process.
- Search returns nothing - search scans node `name`/`title`/`label`/`id` plus all properties (first 50k nodes, 50 results); check the query box text and the selected database.
