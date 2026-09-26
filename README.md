# bugscope-gpui — native GPUI port of Bugscope (`sage-port` branch)

No Tauri, no React, no webview. A single Rust binary: LadybugDB in-process +
native GPUI window.

## Run

```sh
cargo run --release
```

It scans `../bugscope/` for `*.lbdb` files (`demo.lbdb`, `kg_history.lbdb`).
Click a database in the sidebar to load it.

## Interactions (ports of `GraphView` / captors)

| Action | Input |
|---|---|
| Pan | drag background |
| Zoom | scroll wheel (zooms at cursor) |
| Select / drag node | click / drag node |
| Expand neighborhood | double-click node, or select → “Expand neighborhood” |
| Search + focus | type in query box, Enter or Search, click a match |
| Schema view | Schema toggle (tables instead of edge graph) |
| Pause layout | Layout/Pause toggle |
| Full reload | Reload |

## Port map (`../bugscope` → here)

| Original | Here |
|---|---|
| `src-tauri/src/lib.rs` Tauri commands (`collect_edge_graph`, `search_nodes`, `get_node_neighborhood`, `collect_schema_graph`, `scan_for_databases`) | `src/backend.rs` — same Cypher, direct `lbug` calls, no IPC |
| `graph/graphStore.ts` + `graph/palette.ts` | `src/model.rs` `GraphModel` + first-encounter palette |
| `render/camera.ts` | `src/model.rs` `Camera` (world↔screen, cursor-anchored zoom, fit) |
| `hooks/useForceLayout.ts` | `src/model.rs` `tick()` (Coulomb + springs + damping, 30 Hz pump) |
| `render/nodeSizing.ts` | `src/model.rs` `node_size()` (log-scaled) |
| `render/renderer.ts` node-disc + edge-body programs | `src/ui.rs` `paint_graph()` — one rounded quad per node, one quad per edge |
| `App.tsx` + `HeaderBar` + `Sidebar` + `QueryBox` + `GraphView` | `src/ui.rs` `RootView` |

## Theme (same as disktree)

`src/theme.rs` vendors disktree's theme verbatim: `gpui-omarchy`'s
`Theme::tokyo_night()` (dark) / `Theme::flexoki_light()` (light), selected
by `window.appearance()` exactly like disktree's `appearance.rs` follows the
system setting. Node discs use disktree `palette.rs` category hues
(one muted hue per label slot); amber (`warning`) is kept apart for
selection, hover, search, and focus — as in disktree.

## Deliberately out of scope for v1

Summary-space PageRank sidecars, Leiden cluster levels, Voronoi overlay,
antigravity mode, lever panel (50 render levers), LLM cluster naming,
Arrow IPC transport, touch captors — all were web-renderer or sidecar
concerns. The native port loads the edge graph directly and lays it out live.
Cluster/color extensions can build on `GraphModel` without an IPC boundary.
