fn main() {
    // LBUG_LIBRARY_DIR, LBUG_INCLUDE_DIR, and LBUG_SHARED are set in .cargo/config.toml
    // to point at the prebuilt liblbug in src-tauri/liblbug/. This tells the lbug crate's
    // build script to skip CMake compilation and link against the prebuilt shared library.

    // Check that the prebuilt library exists before building.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let liblbug_dir = std::path::Path::new(&manifest_dir).join("liblbug");
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

    tauri_build::build()
}
