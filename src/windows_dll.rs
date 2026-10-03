//! Windows DLL search-path bootstrap.
//!
//! Issue [LadybugDB/bugscope#5](https://github.com/LadybugDB/bugscope/issues/5):
//! `bugscope.exe` failed to start, first with `arrow.dll` / `lbug_shared.dll`
//! / `networkit_state.dll` not found, then — after hand-copying those — with
//! `bz2.dll` / `brotlidec.dll` / `brotlienc.dll` / `lz4.dll` not found.
//!
//! Root cause (packaging): the Windows loader resolves load-time DLLs
//! *before* `main` runs and searches only the exe directory, System32, the
//! Windows directory, and `PATH` — there is no `RPATH`/`@rpath` equivalent,
//! so DLLs sitting in our `icebug/lib` + `liblbug` subdirs were invisible.
//! The release zip must therefore stage every runtime DLL flat next to the
//! exe (see `scripts/stage_windows_bundle.sh`, invoked by the Windows
//! `Package` CI step), including *all* of vcpkg's Arrow DLLs rather than
//! just `arrow*.dll`.
//!
//! This module is defense-in-depth for everything loaded *after* startup —
//! `LOAD EXTENSION` (`libalgo`, `libfts`, …) plus their transitive deps. It
//! prepends the exe dir and the bundled subdirs to `PATH` (plain
//! `LoadLibrary` consults `PATH` at call time), so a non-flat layout (dev
//! tree, old zip) still resolves runtime loads. It cannot help load-time
//! DLLs — nothing running inside `main` can.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Bundled subdirs (relative to the exe dir) that may hold runtime DLLs.
const BUNDLED_DLL_SUBDIRS: &[&str] = &["icebug/lib", "icebug/lib/networkit", "liblbug"];

/// Candidate DLL search dirs for an exe living in `exe_dir`: the exe dir
/// itself first (flat release-zip layout), then the bundled subdirs (dev
/// tree / old layout). Pure so it is unit-testable on any platform.
pub fn dll_search_dirs(exe_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(BUNDLED_DLL_SUBDIRS.len() + 1);
    out.push(exe_dir.to_path_buf());
    for sub in BUNDLED_DLL_SUBDIRS {
        let dir = exe_dir.join(sub);
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// Join `dirs` in front of `current_path` (`None` = no `PATH` set).
/// Returns `None` when no dir should be prepended.
fn join_with_path(dirs: &[PathBuf], current_path: Option<&OsStr>) -> Option<OsString> {
    if dirs.is_empty() {
        return None;
    }
    let mut all = Vec::with_capacity(dirs.len() + 1);
    all.extend(dirs.iter().cloned());
    if let Some(current) = current_path {
        if !current.is_empty() {
            all.extend(std::env::split_paths(current));
        }
    }
    std::env::join_paths(all).ok()
}

/// Compute the `PATH` value for `exe_dir` (existing candidate dirs first,
/// then `current_path`), or `None` when nothing should be prepended. Pure;
/// the thin [`prepend_dll_dirs_to_path`] wrapper applies it to the process.
fn new_path_for_exe_dir(exe_dir: &Path, current_path: Option<&OsStr>) -> Option<OsString> {
    let existing: Vec<PathBuf> = dll_search_dirs(exe_dir)
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    join_with_path(&existing, current_path)
}

/// Prepend every *existing* candidate dir to `PATH` so later `LoadLibrary`
/// calls (e.g. `LOAD EXTENSION` and its transitive deps) resolve them.
pub fn prepend_dll_dirs_to_path(exe_dir: &Path) {
    if let Some(joined) = new_path_for_exe_dir(exe_dir, std::env::var_os("PATH").as_deref()) {
        std::env::set_var("PATH", joined);
    }
}

/// Call first in `main` on Windows. Best-effort: never panics, never fails
/// startup when dirs are absent.
pub fn init() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            prepend_dll_dirs_to_path(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_dirs_are_exe_dir_then_bundled_subdirs() {
        let exe_dir = Path::new("C:/dist/bugscope-0.21.2-windows");
        assert_eq!(
            dll_search_dirs(exe_dir),
            vec![
                exe_dir.to_path_buf(),
                exe_dir.join("icebug/lib"),
                exe_dir.join("icebug/lib/networkit"),
                exe_dir.join("liblbug"),
            ]
        );
    }

    #[test]
    fn prepend_keeps_old_path_tail() {
        let dirs = vec![PathBuf::from("/opt/app"), PathBuf::from("/opt/app/liblbug")];
        // Build the fake old PATH with join_paths so the test is
        // separator-agnostic (`:` on Unix, `;` on Windows).
        let old = std::env::join_paths(["/usr/bin", "/bin"]).unwrap();
        let joined = join_with_path(&dirs, Some(&old)).unwrap();
        let mut parts = std::env::split_paths(&joined);
        assert_eq!(parts.next().unwrap(), PathBuf::from("/opt/app"));
        assert_eq!(parts.next().unwrap(), PathBuf::from("/opt/app/liblbug"));
        let tail: Vec<PathBuf> = parts.collect();
        assert_eq!(tail, vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    }

    #[test]
    fn prepend_without_old_path_yields_dirs_only() {
        let dirs = vec![PathBuf::from("/opt/app")];
        let joined = join_with_path(&dirs, None).unwrap();
        assert_eq!(joined, OsString::from("/opt/app"));
    }

    #[test]
    fn prepend_with_no_dirs_leaves_path_alone() {
        let old = std::env::join_paths(["/usr/bin"]).unwrap();
        assert_eq!(join_with_path(&[], Some(&old)), None);
    }

    #[test]
    fn new_path_skips_missing_dirs() {
        let base = tempfile::tempdir().unwrap();
        let exe_dir = base.path();
        std::fs::create_dir_all(exe_dir.join("icebug/lib")).unwrap();
        // `icebug/lib/networkit` + `liblbug` intentionally absent.
        let old = std::env::join_paths(["/usr/bin"]).unwrap();
        let joined = new_path_for_exe_dir(exe_dir, Some(&old)).unwrap();
        let parts: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert!(parts.contains(&exe_dir.to_path_buf()));
        assert!(parts.contains(&exe_dir.join("icebug/lib")));
        assert!(!parts.contains(&exe_dir.join("liblbug")));
        assert!(parts.contains(&PathBuf::from("/usr/bin")));
    }

    #[test]
    fn new_path_is_none_when_nothing_exists() {
        let base = tempfile::tempdir().unwrap();
        // No subdirs created, and the exe dir itself always exists — so
        // point at a guaranteed-absent dir instead.
        let exe_dir = base.path().join("no-such-dir");
        let old = std::env::join_paths(["/usr/bin"]).unwrap();
        // The exe dir itself is missing too, so every candidate is absent.
        assert_eq!(new_path_for_exe_dir(&exe_dir, Some(&old)), None);
    }
}
