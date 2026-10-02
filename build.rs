use std::path::PathBuf;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let target = std::env::var("TARGET").unwrap_or_default();
    let is_msvc = target.contains("msvc");

    // The lbug docs require binaries (not just rlibs) to act like a library
    // so that `LOAD EXTENSION` can resolve ladybug symbols at load time.
    // `-rdynamic` is an ELF-linker flag: it is a no-op error on Windows/MSVC
    // and actively harmful on Apple targets (clang parses it as `-r`,
    // producing a relocatable object and dropping framework linkage).
    // macOS extensions are built `-undefined dynamic_lookup`, so nothing is
    // needed there. https://docs.ladybugdb.com/extensions/
    if !is_msvc && !target.contains("apple") {
        println!("cargo:rustc-link-arg=-rdynamic");
    }

    // The lbug crate links the shared liblbug from LBUG_LIBRARY_DIR (see
    // .cargo/config.toml); fail fast with a helpful message instead of a
    // cryptic link error when the prebuilt hasn't been downloaded.
    let lbug_dir = std::env::var_os("LBUG_LIBRARY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&manifest_dir).join("liblbug"));
    let lbug_dir = if lbug_dir.is_absolute() {
        lbug_dir
    } else {
        PathBuf::from(&manifest_dir).join(&lbug_dir)
    };
    let lbug_lib = ["liblbug.dylib", "liblbug.so", "lbug_shared.dll", "lbug.lib"]
        .iter()
        .any(|n| lbug_dir.join(n).exists());
    if !lbug_lib {
        panic!(
            "\n\n\
            ============================================================\n\
            ERROR: Shared liblbug prebuilt not found at {}\n\n\
            Run `bash scripts/download-liblbug.sh` to download it from\n\
            GitHub releases.\n\
            ============================================================\n\n",
            lbug_dir.display()
        );
    }

    // The icebug Rust crate links libnetworkit from ICEBUG_DIR (see
    // .cargo/config.toml); fail fast with a helpful message instead of a
    // cryptic link error when the prebuilt hasn't been downloaded.
    if std::env::var("CARGO_FEATURE_ICEBUG_ANALYTICS").is_ok() {
        let icebug_dir = std::env::var_os("ICEBUG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&manifest_dir).join("icebug"));
        // ICEBUG_DIR from .cargo/config.toml is relative to the config file
        // (repo root == manifest dir here), so join when relative.
        let icebug_dir = if icebug_dir.is_absolute() {
            icebug_dir
        } else {
            PathBuf::from(&manifest_dir).join(&icebug_dir)
        };
        let lib_dir = icebug_dir.join("lib");
        let has_lib = ["libnetworkit.dylib", "libnetworkit.so", "networkit.lib"]
            .iter()
            .any(|n| lib_dir.join(n).exists());
        if !has_lib || !icebug_dir.join("include").join("networkit").exists() {
            panic!(
                "\n\n\
                ============================================================\n\
                ERROR: Icebug prebuilt not found at {}\n\n\
                Run `bash scripts/download_icebug.sh` to download it from\n\
                GitHub releases (or build with `--no-default-features` to\n\
                skip icebug analytics).\n\
                ============================================================\n\n",
                icebug_dir.display()
            );
        }

        // Windows-specific (mirrors ../bugscope/src-tauri/build.rs): the
        // icebug Windows release splits Networkit into lib/networkit.lib
        // (static, with dllimport for GlobalState) and
        // lib/networkit/networkit_state.lib (DLL import lib exporting
        // GlobalState). The icebug crate's build.rs only links
        // networkit.lib, missing GlobalState — add it here.
        if target.contains("windows") {
            let search = lib_dir.join("networkit");
            if search.join("networkit_state.lib").exists() {
                println!("cargo:rustc-link-search=native={}", search.display());
                println!("cargo:rustc-link-lib=dylib=networkit_state");
                println!("cargo:warning=Linked networkit_state.lib for GlobalState symbols");
            }
        }
    }
}
