//! Small settings files in `$XDG_CONFIG_HOME/notefy/` (vault path, recent files, bookmarks, flags).

use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn config_file(name: &str) -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        });
    base.join("notefy").join(name)
}

pub(crate) fn write_config(f: PathBuf, value: &str) {
    if let Some(dir) = f.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(f, value);
}

pub(crate) fn save_vault_config(vault: &Path) {
    write_config(config_file("vault"), &vault.to_string_lossy());
}

pub(crate) fn load_vault_config() -> Option<PathBuf> {
    let s = fs::read_to_string(config_file("vault")).ok()?;
    let p = PathBuf::from(s.trim());
    p.is_dir().then_some(p)
}

/// A list of paths stored one per line (recent searches, bookmarks).
pub(crate) fn load_paths(name: &str) -> Vec<PathBuf> {
    fs::read_to_string(config_file(name))
        .map(|s| {
            s.lines()
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn save_paths(name: &str, paths: &[PathBuf]) {
    let s: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    write_config(config_file(name), &s.join("\n"));
}
