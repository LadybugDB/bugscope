use std::path::PathBuf;

fn main() {
    // LBUG_LIBRARY_DIR, LBUG_INCLUDE_DIR, and LBUG_SHARED are set in .cargo/config.toml
    // to point at the prebuilt liblbug in src-tauri/liblbug/. This tells the lbug crate's
    // build script to skip CMake compilation and link against the prebuilt shared library.

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    // Check that the prebuilt liblbug exists before building.
    let liblbug_dir = PathBuf::from(&manifest_dir).join("liblbug");
    if !liblbug_dir.exists() {
        panic!(
            "\n\n\
            ============================================================\n\
            ERROR: Prebuilt liblbug not found at {}\n\n\
            Run `npm run setup` (or `bash scripts/download-liblbug.sh`)\n\
            to download it from GitHub releases.\n\
            ============================================================\n\n",
            liblbug_dir.display()
        );
    }

    // Windows-specific: The icebug v13 Windows release splits Networkit into
    // lib/networkit.lib (static, with dllimport for GlobalState) and
    // lib/networkit/networkit_state.lib (DLL import lib exporting GlobalState).
    // The icebug crate's build.rs only links networkit.lib, missing GlobalState.
    // We add the missing import library here.
    //
    // ICEBUG_DIR is set in .cargo/config.toml (e.g. "icebug" relative to the config dir,
    // resolved to an absolute path by cargo).
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows") {
        if let Some(icebug_dir) = std::env::var_os("ICEBUG_DIR") {
            let search = PathBuf::from(&icebug_dir).join("lib").join("networkit");
            if search.join("networkit_state.lib").exists() {
                println!("cargo:rustc-link-search=native={}", search.display());
                println!("cargo:rustc-link-lib=dylib=networkit_state");
                println!("cargo:warning=Linked networkit_state.lib for GlobalState symbols");
            }
        }
    }

    tauri_build::build()
}
