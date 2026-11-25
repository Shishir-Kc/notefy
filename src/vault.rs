//! The notes folder on disk: scanning it into a tree, moving files, searching notes.
//! No UI code here.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) struct Note {
    pub(crate) path: PathBuf,
    /// Path relative to the vault, without `.md` (used for wikilinks).
    pub(crate) name: String,
}

/// A folder in the vault tree.
#[derive(Default)]
pub(crate) struct Dir {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) dirs: Vec<Dir>,
    pub(crate) files: Vec<PathBuf>,
}

pub(crate) const SKIP_DIRS: &[&str] = &["node_modules", "target"];

/// Recursively scan `dir` into a tree of folders and `.md` files.
pub(crate) fn scan_dir(root: &Path, dir: &Path, notes: &mut Vec<Note>) -> Option<Dir> {
    let entries = fs::read_dir(dir).ok()?;
    let mut node = Dir {
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: dir.to_path_buf(),
        ..Default::default()
    };
    for e in entries.flatten() {
        let path = e.path();
        let fname = e.file_name().to_string_lossy().into_owned();
        if fname.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if SKIP_DIRS.contains(&fname.as_str()) {
                continue;
            }
            if let Some(child) = scan_dir(root, &path, notes) {
                node.dirs.push(child);
            }
        } else if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("md"))
        {
            let rel = path.strip_prefix(root).unwrap_or(&path).with_extension("");
            notes.push(Note {
                name: rel.to_string_lossy().into_owned(),
                path: path.clone(),
            });
            node.files.push(path);
        }
    }
    node.dirs.sort_by_key(|d| d.name.to_lowercase());
    node.files
        .sort_by_key(|f| f.to_string_lossy().to_lowercase());
    Some(node)
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Resolve a `[[target]]`: the vault-relative path (`Folder/Note`) first, else the bare
/// file name (`Note`) anywhere in the vault, case-insensitive.
pub(crate) fn find_note<'a>(notes: &'a [Note], target: &str) -> Option<&'a Note> {
    let t = target.trim().trim_end_matches(".md");
    notes
        .iter()
        .find(|n| n.name.eq_ignore_ascii_case(t))
        .or_else(|| notes.iter().find(|n| stem(&n.path).eq_ignore_ascii_case(t)))
}

/// Link text for `path`: just `[[Name]]`, or `[[Folder/Name|Name]]` when another note
/// has the same file name (so the link still points at the right one).
pub(crate) fn wikilink(notes: &[Note], path: &Path) -> String {
    let name = stem(path);
    let clash = notes
        .iter()
        .any(|n| n.path != path && stem(&n.path).eq_ignore_ascii_case(&name));
    match notes.iter().find(|n| n.path == path) {
        Some(n) if clash => format!("[[{}|{name}]]", n.name.replace('\\', "/")),
        _ => format!("[[{name}]]"),
    }
}

/// Where `p` ends up after `src` was moved/renamed to `dest` (`None` if `p` is not under `src`).
/// Avoids `dest.join("")`, which adds a trailing slash that makes writes fail.
pub(crate) fn rebase(p: &Path, src: &Path, dest: &Path) -> Option<PathBuf> {
    let rest = p.strip_prefix(src).ok()?;
    Some(if rest.as_os_str().is_empty() {
        dest.to_path_buf()
    } else {
        dest.join(rest)
    })
}

/// Move `src` (file or folder) into `dest_dir`. `Ok(None)` = nothing to do.
/// Never overwrites an existing file/folder.
pub(crate) fn move_path(src: &Path, dest_dir: &Path) -> Result<Option<PathBuf>, String> {
    let name = src.file_name().ok_or("Nothing to move")?;
    let dest = dest_dir.join(name);
    if src.parent() == Some(dest_dir) {
        return Ok(None);
    }
    if dest_dir.starts_with(src) {
        return Err("Can't move a folder into itself".into());
    }
    if dest.exists() {
        return Err(format!("{} already exists there", name.to_string_lossy()));
    }
    fs::rename(src, &dest).map_err(|e| format!("Move failed: {e}"))?;
    Ok(Some(dest))
}

/// Path to `to` relative to the directory `from` (both absolute).
pub(crate) fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let (f, t): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..f.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c);
    }
    out
}

/// Rank notes for `query`: name prefix > name substring > fuzzy name > content match.
pub(crate) fn search_notes(notes: &[Note], query: &str) -> Vec<PathBuf> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let mut scored: Vec<(u32, &Note)> = notes
        .iter()
        .filter_map(|n| {
            let name = n.path.file_stem()?.to_string_lossy().to_lowercase();
            let rel = n.name.to_lowercase();
            let score = if name.starts_with(&q) {
                0
            } else if name.contains(&q) {
                1
            } else if rel.contains(&q) {
                2
            } else if is_subsequence(&q, &name) {
                3
            } else if fs::read_to_string(&n.path).is_ok_and(|c| c.to_lowercase().contains(&q)) {
                4
            } else {
                return None;
            };
            Some((score, n))
        })
        .collect();
    scored.sort_by_key(|(s, n)| (*s, n.name.len(), n.name.to_lowercase()));
    scored
        .into_iter()
        .take(50)
        .map(|(_, n)| n.path.clone())
        .collect()
}

/// Undirected `[[link]]` edges between notes, as index pairs into `notes` (`a < b`,
/// no duplicates, no self-links). Links to missing notes are ignored.
pub(crate) fn link_edges(notes: &[Note]) -> Vec<(usize, usize)> {
    let mut edges = std::collections::BTreeSet::new();
    for (i, n) in notes.iter().enumerate() {
        let Ok(text) = fs::read_to_string(&n.path) else {
            continue;
        };
        for target in crate::live::wikilinks(&text) {
            let Some(t) = find_note(notes, target) else {
                continue;
            };
            if let Some(j) = notes.iter().position(|m| m.path == t.path)
                && j != i
            {
                edges.insert((i.min(j), i.max(j)));
            }
        }
    }
    edges.into_iter().collect()
}

/// True if all chars of `needle` appear in `hay` in order (e.g. "mtg" in "meeting").
pub(crate) fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let v = Path::new("/v");
        assert_eq!(
            relative_path(v, Path::new("/v/attachments/a.png")),
            PathBuf::from("attachments/a.png")
        );
        assert_eq!(
            relative_path(&v.join("English"), Path::new("/v/attachments/a.png")),
            PathBuf::from("../attachments/a.png")
        );
    }

    #[test]
    fn scan_builds_tree_of_md_only() {
        let v = std::env::temp_dir().join(format!("notefy-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&v);
        for d in [
            "Projects/Rust",
            "Journal/2026",
            "empty",
            ".hidden",
            "Assets",
        ] {
            fs::create_dir_all(v.join(d)).unwrap();
        }
        for f in [
            "Inbox.md",
            "Projects/Roadmap.md",
            "Projects/Rust/egui.md",
            "Journal/2026/Nov.md",
            ".hidden/x.md",
        ] {
            fs::write(v.join(f), "# x").unwrap();
        }
        fs::write(v.join("Assets/pic.png"), "x").unwrap();

        let mut notes = vec![];
        let t = scan_dir(&v, &v, &mut notes).unwrap();
        let names: Vec<_> = t.dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Assets", "empty", "Journal", "Projects"]); // hidden dropped
        assert_eq!(t.files.len(), 1);
        assert!(t.dirs[0].files.is_empty()); // pic.png is not listed
        let proj = &t.dirs[3];
        assert_eq!(proj.dirs[0].name, "Rust");
        assert_eq!(proj.files.len(), 1);
        assert_eq!(notes.len(), 4);
        fs::remove_dir_all(&v).unwrap();
    }

    #[test]
    fn wikilinks_use_file_name() {
        let note = |rel: &str| Note {
            path: PathBuf::from(format!("/v/{rel}.md")),
            name: rel.into(),
        };
        let notes = [note("First_Sem/English/test"), note("a/dup"), note("b/dup")];
        let test = Path::new("/v/First_Sem/English/test.md");
        assert_eq!(wikilink(&notes, test), "[[test]]");
        assert_eq!(wikilink(&notes, Path::new("/v/b/dup.md")), "[[b/dup|dup]]"); // name clash
        assert_eq!(
            find_note(&notes, "Test").map(|n| n.path.as_path()),
            Some(test)
        );
        assert_eq!(find_note(&notes, "b/dup").unwrap().name, "b/dup");
        assert!(find_note(&notes, "nope").is_none());
    }

    #[test]
    fn rebase_paths() {
        let (src, dest) = (Path::new("/v/a"), Path::new("/v/b/a"));
        assert_eq!(rebase(src, src, dest), Some(dest.to_path_buf()));
        assert!(
            !rebase(src, src, dest)
                .unwrap()
                .to_string_lossy()
                .ends_with('/')
        );
        assert_eq!(
            rebase(Path::new("/v/a/n.md"), src, dest),
            Some(PathBuf::from("/v/b/a/n.md"))
        );
        assert_eq!(rebase(Path::new("/v/ab.md"), src, dest), None);
    }

    #[test]
    fn move_path_rules() {
        let v = std::env::temp_dir().join(format!("notefy-mv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&v);
        fs::create_dir_all(v.join("a/sub")).unwrap();
        fs::create_dir_all(v.join("b")).unwrap();
        fs::write(v.join("note.md"), "x").unwrap();
        fs::write(v.join("b/note.md"), "existing").unwrap();
        fs::write(v.join("a/other.md"), "y").unwrap();

        // same folder: no-op
        assert_eq!(move_path(&v.join("note.md"), &v), Ok(None));
        // name clash: refused, nothing overwritten
        assert!(move_path(&v.join("note.md"), &v.join("b")).is_err());
        assert_eq!(fs::read_to_string(v.join("b/note.md")).unwrap(), "existing");
        // folder into itself / its child: refused
        assert!(move_path(&v.join("a"), &v.join("a/sub")).is_err());
        // file into folder
        assert_eq!(
            move_path(&v.join("a/other.md"), &v.join("b")),
            Ok(Some(v.join("b/other.md")))
        );
        assert!(v.join("b/other.md").exists() && !v.join("a/other.md").exists());
        // folder into folder
        assert_eq!(
            move_path(&v.join("a"), &v.join("b")),
            Ok(Some(v.join("b/a")))
        );
        assert!(v.join("b/a/sub").is_dir());
        fs::remove_dir_all(&v).unwrap();
    }

    #[test]
    fn link_graph_edges() {
        let v = std::env::temp_dir().join(format!("notefy-links-{}", std::process::id()));
        let _ = fs::remove_dir_all(&v);
        fs::create_dir_all(v.join("sub")).unwrap();
        fs::write(v.join("a.md"), "[[b]] [[sub/c|C]] [[a]] [[missing]]").unwrap();
        fs::write(v.join("b.md"), "back to [[A]]").unwrap();
        fs::write(v.join("sub/c.md"), "").unwrap();
        fs::write(v.join("lonely.md"), "").unwrap();
        let mut notes = vec![];
        scan_dir(&v, &v, &mut notes);
        let names: Vec<_> = link_edges(&notes)
            .iter()
            .map(|&(i, j)| (notes[i].name.clone(), notes[j].name.clone()))
            .map(|(a, b)| if a < b { (a, b) } else { (b, a) })
            .collect();
        assert_eq!(names.len(), 2); // a–b once (both directions), a–c; no self/missing
        assert!(names.contains(&("a".into(), "b".into())));
        assert!(names.contains(&("a".into(), "sub/c".into())));
        fs::remove_dir_all(&v).unwrap();
    }

    #[test]
    fn search_ranking() {
        let v = std::env::temp_dir().join(format!("notefy-search-{}", std::process::id()));
        let _ = fs::remove_dir_all(&v);
        fs::create_dir_all(v.join("work")).unwrap();
        fs::write(v.join("meeting notes.md"), "agenda").unwrap();
        fs::write(v.join("work/team meeting.md"), "x").unwrap();
        fs::write(v.join("ideas.md"), "talk about the meeting later").unwrap();
        fs::write(v.join("todo.md"), "nothing").unwrap();
        let mut notes = vec![];
        scan_dir(&v, &v, &mut notes);
        let names = |q| -> Vec<String> {
            search_notes(&notes, q)
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        // prefix, then substring, then content
        assert_eq!(
            names("meet"),
            ["meeting notes.md", "team meeting.md", "ideas.md"]
        );
        assert_eq!(names("mtg"), ["meeting notes.md", "team meeting.md"]); // fuzzy
        assert_eq!(names("WORK"), ["team meeting.md"]); // folder path, case-insensitive
        assert!(names("zzz").is_empty());
        fs::remove_dir_all(&v).unwrap();
    }
}
