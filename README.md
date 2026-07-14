# Bugscope Graph Visualizer

<p align="center">
  <img src="assets/img/bugscope.png" alt="Bugscope graph visualizer screenshot" width="760">
</p>

An interactive graph visualization tool for ladybugdb. Explore relationships between bugs, files, and other entities in your databases through an intuitive visual interface.

## Features

- **Interactive Graph View** - Navigate through connected data using a force-directed graph. Drag nodes to rearrange, zoom in/out, and pan around the canvas.
- **Database Selection** - Choose from available ladybugdb databases in the sidebar to visualize their relationships.
- **Visual Encoding** - Node size reflects connection count (more connections = larger nodes), and colors differentiate entity types.
- **Dark/Light Mode** - Toggle between dark and light themes for comfortable viewing.
- **Relationship Labels** - Hover over edges to see the type of relationship between connected nodes.
- **Optional LLM Cluster Names** - Provide an LLM access token to name Leiden clusters from a random sample of 15 node labels in each cluster.

## Building

Bugscope is a [Tauri 2](https://v2.tauri.app) desktop app: a React/Vite frontend plus a Rust backend that links two prebuilt native libraries. What you need, on any platform:

1. **Node.js ≥ 22 and npm** - frontend tooling (Vite 7).
2. **Rust toolchain** ([rustup](https://rustup.rs)) **and the Tauri CLI**: `cargo install tauri-cli --version "^2"`. The CLI is a separate cargo subcommand; without it `cargo tauri` fails with `no such command: tauri`.
3. **Tauri system libraries** - on Linux: WebKitGTK 4.1, GTK 3, libsoup 3, and Pango development packages; see the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your distribution. On macOS: Xcode command line tools.
4. **JS dependencies**: `npm install`.
5. **Vendored and prebuilt dependencies**: `npm run setup` provisions all of:
   - the **sigma.js fork** with the `sigma/icebug` entry point, cloned from [adsharma/sigma.js](https://github.com/adsharma/sigma.js) (`icebug-arrow-graph` branch) into `.deps/sigma.js/` and built there (`package.json` references it as `file:.deps/sigma.js/packages/sigma`);
   - **liblbug** (LadybugDB C API) into `src-tauri/liblbug/`, at the version pinned by the `lbug` crate in `src-tauri/Cargo.toml`;
   - **icebug** (NetworKit-based graph analytics) into `src-tauri/icebug/` - only linked when building with `--features=icebug-analytics`.
6. **Apache Arrow C++ and OpenMP** (only for `--features=icebug-analytics`) - the icebug build script locates Arrow with `pkg-config arrow`, so the Arrow development package must be installed system-wide, and the icebug prebuilt additionally needs the LLVM OpenMP runtime (`libomp.so.5`). The Arrow **major version must match** the one the icebug prebuilt was linked against; check it with `objdump -p src-tauri/icebug/lib/libnetworkit.so | grep NEEDED` (currently `libarrow.so.2400`, i.e. Arrow 24). On macOS: `brew install apache-arrow libomp`.
7. **Run**:
   ```bash
   cargo tauri dev                              # without analytics (no Arrow needed)
   cargo tauri dev --features=icebug-analytics  # with Leiden clustering etc.
   ```
   Append `-- -- /path/to/database.lbdb` to load a database at startup (the extra `--` separates tauri args from cargo args from app args).

### Debian 13 (trixie)

Verified end to end on a stock trixie installation:

```bash
# 1. Tauri system libraries and build tools
sudo apt install libgtk-3-dev libsoup-3.0-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libpango1.0-dev pkg-config \
  wget ca-certificates lsb-release

# 2. Rust toolchain via rustup, then the Tauri CLI
cargo install tauri-cli --version "^2"

# 3. Apache Arrow apt repository (Debian ships no libarrow packages)
wget https://packages.apache.org/artifactory/arrow/$(lsb_release --id --short | tr 'A-Z' 'a-z')/apache-arrow-apt-source-latest-$(lsb_release --codename --short).deb
sudo apt install -y -V ./apache-arrow-apt-source-latest-$(lsb_release --codename --short).deb
sudo apt update

# 4. Arrow C++ pinned to the major version the icebug prebuilt links (currently 24),
#    plus the LLVM OpenMP runtime needed by the icebug prebuilt
sudo apt install libarrow-dev=24.0.0-1 libarrow2400 libomp5-19
sudo apt-mark hold libarrow-dev   # keep apt upgrade from moving to a mismatched major

# 5. Dependencies and prebuilt libraries
npm install
npm run setup

# 6. Run
cargo tauri dev --features=icebug-analytics
```

### Ubuntu 26.04 (resolute)

The same steps should work unchanged: the package names match and the Arrow repository serves `ubuntu/resolute` (verified to contain `libarrow-dev`). Not yet tested end to end - please report findings.

### Troubleshooting

- `error: no such command: tauri` - the Tauri CLI is not installed: `cargo install tauri-cli --version "^2"`.
- `Failed to resolve import "sigma"` in the app window - the vendored sigma.js fork in `.deps/sigma.js` is missing or unbuilt; run `npm run setup`.
- `The system library arrow required by crate icebug was not found` - `libarrow-dev` is missing (steps 3-4 above), or you are building with `--features=icebug-analytics` unintentionally; plain `cargo tauri dev` does not need Arrow.
- `error while loading shared libraries: libarrow.so.NN00` - the installed Arrow major version differs from the one the icebug prebuilt was linked against. Install the matching runtime and dev packages (e.g. `libarrow2400` + `libarrow-dev=24.0.0-1`), then `cargo clean -p icebug` and rebuild.
- `error while loading shared libraries: libomp.so.5` - LLVM OpenMP runtime missing: `sudo apt install libomp5-19` (Debian/Ubuntu), `brew install libomp` (macOS).
- Prebuilt liblbug and the `lbug` crate version in `src-tauri/Cargo.toml` must match; `npm run setup` re-downloads automatically when the pin changes.

## Usage

1. Select a database from the sidebar on the left
2. The graph will load and display nodes (entities) and edges (relationships)
3. Click and drag nodes to rearrange the layout
4. Scroll to zoom in/out, click and drag the canvas to pan
5. Hover over nodes to see their labels
6. Hover over edges to see relationship types
7. Use the theme toggle button to switch between dark and light modes

## Optional LLM Cluster Naming

Cluster naming is disabled by default. To enable it, expand the sidebar's Cluster Names section, enter an LLM access token, and click Apply. Cluster computation still starts with local fallback names; the backend only sends a random sample of 15 labels for clusters that are currently visible in the drill view, then falls back to `Cluster N` if a request fails.

The endpoint defaults to `https://openrouter.ai`, and the model field defaults to `deepseek-v4-flash`. Base endpoints are sent through their `/api/v1/chat/completions` path, and both fields can be changed in the sidebar before applying.
