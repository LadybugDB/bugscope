fn main() {
    // Use prebuilt liblbug from GitHub releases instead of compiling C++ from source.
    // The prebuilt library lives in src-tauri/liblbug/
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let liblbug_dir = format!("{}/liblbug", manifest_dir);
    std::env::set_var("LBUG_LIBRARY_DIR", &liblbug_dir);
    std::env::set_var("LBUG_INCLUDE_DIR", &liblbug_dir);
    std::env::set_var("LBUG_SHARED", "1");

    tauri_build::build()
}
