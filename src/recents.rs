//! Recently-opened databases + shell history persistence.
//!
//! Both are plain files under the home directory so recents survive
//! restarts without a config dependency:
//!   * `~/.bugscope_recent.json` — `{ "local": [paths…], "remote": [uris…] }`
//!   * `~/.bugscope_shell_history` — one command per line (like `lbug -p`).

use std::path::PathBuf;

const MAX_RECENT: usize = 12;
const MAX_HISTORY: usize = 500;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct RecentFile {
    #[serde(default)]
    local: Vec<String>,
    #[serde(default)]
    remote: Vec<String>,
}

fn recent_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".bugscope_recent.json"))
}

fn shell_history_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".bugscope_shell_history"))
}

fn load_file() -> RecentFile {
    let Some(p) = recent_path() else {
        return RecentFile::default();
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_file(r: &RecentFile) {
    let Some(p) = recent_path() else {
        return;
    };
    if let Ok(s) = serde_json::to_string_pretty(r) {
        let _ = std::fs::write(p, s);
    }
}

fn push(list: &mut Vec<String>, item: &str) {
    list.retain(|x| x != item);
    list.insert(0, item.to_string());
    list.truncate(MAX_RECENT);
}

/// Recently opened local `.lbdb` paths, most recent first.
pub fn load_recent_local() -> Vec<String> {
    load_file()
        .local
        .into_iter()
        .filter(|p| std::path::Path::new(p).is_file())
        .collect()
}

/// Recently connected remote URIs, most recent first.
pub fn load_recent_remote() -> Vec<String> {
    load_file().remote
}

pub fn remember_local(path: &str) {
    let mut r = load_file();
    push(&mut r.local, path);
    save_file(&r);
}

pub fn remember_remote(uri: &str) {
    let uri = uri.trim();
    if uri.is_empty() {
        return;
    }
    let mut r = load_file();
    push(&mut r.remote, uri);
    save_file(&r);
}

pub fn forget_remote(uri: &str) {
    let mut r = load_file();
    r.remote.retain(|x| x != uri);
    save_file(&r);
}

/// Shell command history (newest last on disk, like readline HISTFILE).
pub fn load_shell_history() -> Vec<String> {
    let Some(p) = shell_history_path() else {
        return Vec::new();
    };
    std::fs::read_to_string(p)
        .map(|s| {
            s.lines()
                .map(|l| l.to_string())
                .filter(|l| !l.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn append_shell_history(cmd: &str) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return;
    }
    let Some(p) = shell_history_path() else {
        return;
    };
    let mut hist = load_shell_history();
    if hist.last().map(|l| l.as_str()) == Some(cmd) {
        return;
    }
    hist.push(cmd.to_string());
    if hist.len() > MAX_HISTORY {
        hist.drain(..hist.len() - MAX_HISTORY);
    }
    let _ = std::fs::write(p, hist.join("\n") + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_dedups_and_caps() {
        let mut v = vec!["b".to_string(), "a".to_string()];
        push(&mut v, "a");
        assert_eq!(v[0], "a");
        assert_eq!(v.len(), 2);
    }
}
