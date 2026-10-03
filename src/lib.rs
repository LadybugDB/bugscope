//! Bugscope — native port of the ../bugscope `sage-port` branch.
//!
//! The Tauri+React app is replaced by a single Rust binary:
//!   * `backend` talks to LadybugDB (`lbug`) directly in-process — no Tauri IPC,
//!     no JSON bridge, no webview. Same Cypher queries as `src-tauri/src/lib.rs`.
//!   * `model` is the owned graph container + camera + force layout
//!     (ports of `graphStore.ts`, `camera.ts`, `useForceLayout.ts`).
//!   * GPUI renders natively: header bar, sidebar, query box, canvas graph view
//!     (ports of `App.tsx`, `HeaderBar`, `Sidebar`, `QueryBox`, `GraphView`,
//!     `renderer.ts` node-disc + edge-body programs drawn on a `canvas`).

pub mod backend;
pub mod cli;
pub mod clusters;
pub mod model;
pub mod theme;
pub mod ui;
pub mod windows_dll;
