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

        // Copy runtime DLLs next to the exe so `cargo tauri dev` / `cargo run`
        // work on Windows. (The MSI/NSIS installers get the same DLLs via
        // `bundle.resources`; see scripts/stage_windows_dlls.sh for issue #23.)
        // Best-effort: warn instead of failing when a DLL can't be located.
        let exe_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap())
            .ancestors()
            .nth(3)
            .map(|p| p.to_path_buf());
        if let Some(exe_dir) = exe_dir {
            let mut dlls: Vec<PathBuf> = Vec::new();
            // Staged bundle DLLs (populated by stage_windows_dlls.sh).
            let stage = PathBuf::from(&manifest_dir).join("windows-dlls");
            if stage.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&stage) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.extension().map(|x| x == "dll").unwrap_or(false) {
                            dlls.push(p);
                        }
                    }
                }
            }
            // Fallbacks when the staging dir is empty (e.g. local dev without
            // running the staging script first).
            if dlls.is_empty() {
                if let Some(lbug_dir) = std::env::var_os("LBUG_LIBRARY_DIR") {
                    let lbug_dll = PathBuf::from(&lbug_dir).join("lbug_shared.dll");
                    if lbug_dll.exists() {
                        dlls.push(lbug_dll);
                    }
                }
                if let Some(icebug_dir) = std::env::var_os("ICEBUG_DIR") {
                    let net_dir = PathBuf::from(&icebug_dir).join("lib").join("networkit");
                    if let Ok(entries) = std::fs::read_dir(&net_dir) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if p.extension().map(|x| x == "dll").unwrap_or(false) {
                                dlls.push(p);
                            }
                        }
                    }
                }
            }
            for dll in &dlls {
                let dest = exe_dir.join(dll.file_name().unwrap());
                if std::fs::copy(dll, &dest).is_ok() {
                    println!("cargo:warning=Copied {} next to exe", dll.display());
                } else {
                    println!(
                        "cargo:warning=Could not copy {} to {}",
                        dll.display(),
                        dest.display()
                    );
                }
            }
        }
    }

    tauri_build::build()
}
