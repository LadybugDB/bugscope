# Bugscope Graph Visualizer

An interactive graph visualization tool for LadybugDB. Explore relationships between bugs, files, and other entities in your databases through an intuitive visual interface.

This is the native port of [bugscope-tauri](https://github.com/LadybugDB/bugscope-tauri): no Tauri, no React, no webview. A single Rust binary with LadybugDB in-process and a native [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) window.

## Features

- **Interactive Graph View** - Navigate through connected data using a force-directed graph. Drag nodes to rearrange, zoom in/out, and pan around the canvas.
- **Database Selection** - Choose from available LadybugDB databases in the sidebar, or use **Open file…** to browse for any `*.lbdb` file.
- **Visual Encoding** - Node size reflects connection count (more connections = larger nodes), and colors differentiate entity types.
- **Dark/Light Mode** - Follows the system appearance automatically, same themes as the Tauri app.
- **Relationship Labels** - Hover over edges to see the type of relationship between connected nodes.
- **Search + Focus** - Substring search over node properties (`/foo`, bare `/` clears); click a match to focus its 1-hop neighborhood.
- **Breadcrumbs + dot-commands** - Drill-down trail under the header (`Root › A › B`, cached jumps) plus query-box `.root` / `.parent` (`.up`/`.back`) and `.schema` / `.data` (schema view without the menu).
- **Schema View** - Header `Data`/`Schema` toggle (or `.schema` / `.data` in the query box, `File → Toggle Schema View` menu) to switch between node tables and the edge graph. The breadcrumb badge (`schema ✕`) is also a one-click way back to data.
- **Menus everywhere** - macOS uses the native menu bar; Windows and Linux draw an equivalent in-app `File`/`View` bar with the same commands (`Ctrl` in place of `⌘`).
- **Live Layout** - Force simulation (repulsion + springs + damping) runs at 30 Hz and settles when the layout goes quiet; pause/resume any time.
- **Leiden Treemap + Sunburst** - Header toggle (or `⌘1`/`⌘2`/`⌘3`, `Ctrl` on Windows/Linux) switches the canvas between the graph, a squarified treemap of Leiden communities, and a classic sunburst of the same communities (in-process icebug Leiden, same family as `GDS_LEIDEN`): the center disc is the root of the displayed graph, rings move outward with hierarchy depth (communities, then members), slice angles are proportional to PageRank share while each member's reach encodes its absolute weight — heavy members stick out, so the outer edge reads jagged rather than a perfect disc. Double-click drills into a neighborhood; clicking the sunburst center steps one level up the trail. At startup, graphs with more than 64 edges open in the treemap (smaller ones in the graph view); the choice then sticks — navigating never switches views, only the header toggle, menu, or `⌘1`/`⌘2`/`⌘3` (`Ctrl` on Windows/Linux) do.
- **Insights Pane** - Collapsible right pane with the top 10 nodes by PageRank plus the Leiden community summary; clicking a row focuses (PageRank) or selects (community) it.

## Usage

1. Select a database from the sidebar on the left (scanned from the working directory), or click **Open file…** to pick a `*.lbdb` anywhere on disk
2. The graph will load and display nodes (entities) and edges (relationships)
3. Click and drag nodes to rearrange the layout
4. Scroll to zoom in/out (zooms at the cursor), click and drag the canvas to pan
5. Hover over nodes to see their labels
6. Hover over edges to see relationship types
7. Double-click a node (or select it and choose "Expand neighborhood") to focus its 1-hop neighborhood.
   Each focus pushes the breadcrumb trail under the header (`Root › A › B`):
   click any crumb to jump back, or use `↑ Parent` / `⟲ Root` on the right.
8. Type in the query box and press Enter to search; click a match to focus it
9. Switch between the data graph and the schema (table) view with the header `Data`/`Schema` toggle, `.schema` / `.data` in the query box, or `File → Toggle Schema View`

### Query box: search, Cypher, and dot-commands

| Input | Effect |
|---|---|
| `/foo` | Substring search over node `name`/`title`/`label`/`id` + properties. Matches list in the sidebar; starting a new search first resets any zoom so matches are in full-graph context. |
| `/` (bare slash) | Clears text-search state: sidebar matches + any search zoom/selection, back to the root view. |
| `.root` | Breadcrumb root: clear the drill-down trail, show the full graph (or current Cypher result) from cache — no DB reload. |
| `.parent` (aliases: `.up`, `.back`) | One step up the trail (`B` → `A` → root). Reports `Already at root` at the top. |
| `.schema` / `.schema on` | Switch to the schema view (node tables + rel connectivity). Already there → resets to the schema root. Same as the header `Schema` pill. |
| `.data` / `.graph` (also `.schema off`) | Leave the schema view, back to the data graph. Same as the header `Data` pill or clicking the `schema ✕` badge in the breadcrumb bar. `.schema toggle` flips either way. |
| anything else | Runs as read Cypher and replaces the graph (new root, trail cleared). Unknown `.foo` is rejected with the valid list — it never runs as Cypher. |

## Prerequisites

Just the Rust toolchain ([rustup](https://rustup.rs)). No Node.js, no Tauri CLI, no WebKit/GTK system libraries, no vendored sigma.js fork — the UI is native GPUI, not a webview.

On macOS: Xcode command line tools (for any Rust build).

## Run

```bash
cargo run --release
```

Working directory matters only for the initial sidebar scan: `*.lbdb` files under `.` are listed automatically. Anything else can be opened via **Open file…**, so you can run from anywhere.

## Graph analytics (icebug / GDS)

Every connection runs `LOAD algo` (installing from the official repo at
`https://extension.ladybugdb.com` on first use), so PageRank and friends work
straight from the query box as Cypher table functions:

```cypher
CALL PROJECT_GRAPH('G', ['N'], ['E']);
CALL GDS_PAGE_RANK('G') RETURN node.id, rank ORDER BY rank DESC;
```

Available: `PROJECT_GRAPH`, `PAGE_RANK`, `GDS_PAGE_RANK`, `GDS_LOUVAIN`,
`GDS_LEIDEN`, `GDS_PPR`, `GDS_NODE2VEC`, k-core, components, spanning forest.
Set `BUGSCOPE_ALGO_EXTENSION` to a local `.lbug_extension` file to override
the downloaded one (e.g. a build from
[LadybugDB/extensions](https://github.com/LadybugDB/extensions/tree/main/algo)).

The treemap and insights pane run the same algorithms in-process over
whatever is displayed (full scan, Cypher result, or neighborhood), with no
`PROJECT_GRAPH` step: `backend::graphr_leiden_full` (Leiden + modularity)
and `backend::graphr_page_rank`. `backend::gds_leiden_communities` reads
`CALL GDS_LEIDEN(...)` rows back for an existing projection when you want
the extension's own output.

From source:

```bash
bash scripts/download_icebug.sh      # prebuilt libnetworkit for your platform -> ./icebug/
bash scripts/download-liblbug.sh     # prebuilt shared liblbug (dlopen needs shared, not static) -> ./liblbug/
bash scripts/vendor_arrow.sh         # stage libarrow/libomp next to libnetworkit (the algo
                                      # extension references @rpath/libarrow, resolved via our rpaths)
cargo test --test gds_page_rank      # GDS_PAGE_RANK vs the expected ranks (skips when offline)
cargo test --test gds_leiden         # GDS_LEIDEN two-cliques structure + local Leiden cross-check
```

The `icebug-analytics` cargo feature (on by default; `--no-default-features` to skip) links the
icebug Rust crate for in-process analytics such as `backend::graphr_page_rank`.

Bulk loading is tiered CSR → columnar Arrow → row-wise ids, so a silent-empty
fast path can never hide the graph. Our `arrow` is pinned to 55 to match lbug
(0.21.x builds its batches with arrow 55 — decoding them with another major
silently yields zero rows); icebug 13.2.0 needs arrow 56, which lives behind
the `arrow56` alias for the analytics arrays only.

## Project structure

| Path | Contents |
|---|---|
| `src/main.rs` | App entry: window setup, titlebar, autofocus |
| `src/backend.rs` | Direct LadybugDB backend — `open_connection`, `scan_for_databases`, `collect_edge_graph`, `collect_schema_graph`, `search_nodes`, `neighborhood`, in-process PageRank/Leiden (`graphr_page_rank`, `graphr_leiden_full`) + `GDS_LEIDEN` readout (`gds_leiden_communities`) |
| `src/clusters.rs` | Squarified treemap layout over Leiden communities (disktree-inspired `squarify` + header bands + hit-testing), pure geometry with unit tests |
| `src/model.rs` | `GraphModel` (force layout tick, palette, picking) + `Camera` (world↔screen, cursor-anchored zoom, fit) |
| `src/ui.rs` | `RootView` — sidebar, insights pane, query box, canvas graph + treemap views, interaction |
| `src/theme.rs` | Vendored disktree theme (`tokyo_night` dark / `flexoki_light` light), selected via `window.appearance()` like disktree follows the system setting |

The backend runs the same Cypher as the Tauri commands (`MATCH (a)-[r]->(b) RETURN a, r, b`, isolated-node scan, `CALL SHOW_TABLES`, node sampling) with direct `lbug` calls and no IPC boundary.

## Deliberately out of scope for v1

Summary-space PageRank sidecars, LLM cluster naming, Voronoi overlay, the lever panel, Arrow IPC transport — all were web-renderer or sidecar concerns in bugscope-tauri. The native port loads the edge graph directly and lays it out live.

### Troubleshooting

- Window doesn't appear on launch - older versions scanned `$HOME` recursively on the UI thread before first paint and hung. Current versions only scan the working directory; update and retry.
- `Load failed: ...` in the status bar - the picked file isn't a readable LadybugDB database, or it's locked by another process.
- Search returns nothing - search scans node `name`/`title`/`label`/`id` plus all properties (first 50k nodes, 50 results); check the query box text and the selected database.
- Opening a file fails with `Failed to load library: .../.lbdb/extension/<ver>/.../libalgo.lbug_extension` - that DB ran `LOAD algo` before, which ladybug WAL-logs and replays on every open, so one stale cached build bricks the file. Library paths are not the cause (the binary resolves `@rpath` deps from its bundled `Frameworks` plus Homebrew/system locations, no `DYLD_LIBRARY_PATH` needed): it is symbol skew between the cached build and `liblbug` (check with plain `dlopen` — `symbol not found in flat namespace` means skew, `Library not loaded` means paths). Fix: back up, then replace the named cached file with a symbol-matching build (a plain `LOAD EXTENSION` of that file must succeed); `INSTALL algo` may re-serve the stale build, so avoid re-running it until upstream publishes matching bits. `BUGSCOPE_ALGO_EXTENSION` only affects post-open `LOAD`, not WAL replay.
- `bugscope.exe` won't start (`*.dll was not found`, Windows) - fixed by staging every runtime DLL flat next to the exe: the Windows loader (unlike ELF `RPATH` / Mach-O `@rpath`) never looks in the `icebug/` + `liblbug/` subdirs. Current zips ship `lbug_shared`, `networkit_state`, `arrow` *and* its `bz2`/`brotli`/`lz4` family plus OpenSSL beside the exe (`scripts/stage_windows_bundle.sh`, asserted in CI), and the exe additionally prepends its bundled subdirs to `PATH` for runtime `LOAD EXTENSION` deps. On an older zip, copy *all* `*.dll` from `icebug/lib` (including `icebug/lib/networkit`), `liblbug`, and the Arrow runtime next to the exe — copying just the three named DLLs only reaches the next missing-dep error.
