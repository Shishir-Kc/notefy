//! The `Notefy` app: state, note/tab/history management and the per-frame `ui` loop.
//! UI pieces live in submodules, each adding methods to `Notefy`.

mod dialogs;
mod editor;
mod graph;
mod header;
mod sidebar;
mod widgets;

use crate::{config::*, semantic::Semantic, theme::*, vault::*};
use dialogs::SearchState;
use eframe::egui::{self, Key, Margin, Stroke};
use graph::Graph;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How long a status message stays in the bottom-right corner.
const STATUS_SECS: u64 = 4;
/// Wait this long after launch before loading the embedding model, so the window shows first.
const INDEX_DELAY: Duration = Duration::from_secs(2);

pub(crate) struct Notefy {
    vault: PathBuf,
    notes: Vec<Note>,
    tree: Dir,
    expanded: HashSet<PathBuf>,
    /// Note shown in the active tab. Mirrors `tabs[active].path`; only `load` sets them.
    current: Option<PathBuf>,
    /// Open tabs (never empty) and the index of the active one.
    tabs: Vec<Tab>,
    active: usize,
    buffer: String,
    dirty: bool,
    last_edit: Instant,
    /// Inline title above the editor; renames the file when committed.
    title_edit: String,
    /// Focus and select the inline title next frame (right after creating a note).
    focus_title: bool,
    sidebar_open: bool,
    side_view: SideView,
    /// File tree sorted Z→A instead of A→Z.
    sort_desc: bool,
    bookmarks: Vec<PathBuf>,
    /// Name typed in the "New folder" dialog (`Some` = dialog open).
    new_folder: Option<String>,
    /// File waiting for the "are you sure?" popup.
    pending_delete: Option<PathBuf>,
    /// User picked "Don't remind me again".
    skip_delete_confirm: bool,
    search: SearchState,
    /// Background embedding model for meaning-based search results.
    semantic: Semantic,
    /// Link graph shown instead of the editor (`None` = closed; opening a note closes it).
    graph: Option<Graph>,
    /// When to start the background indexing after launch (`None` = started).
    index_at: Option<Instant>,
    /// Wikilink clicked in the editor, opened in a new tab after the frame.
    pending_link: Option<String>,
    /// `[[` suggestion popup in the editor.
    link: LinkPopup,
    /// Ctrl+click preview of a linked note.
    preview: Option<Preview>,
    /// Transient message for the status corner, and when it was set.
    status: String,
    status_at: Instant,
}

/// One editor tab: the note it shows (`None` = empty "New tab") and its own history.
#[derive(Default)]
struct Tab {
    path: Option<PathBuf>,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
    /// Reading mode: the note can't be edited; a click on any link follows it.
    reading: bool,
}

/// What the sidebar lists.
#[derive(Clone, Copy, PartialEq)]
enum SideView {
    Files,
    Bookmarks,
}

#[derive(Default)]
struct LinkPopup {
    last_query: Option<String>,
    results: Vec<PathBuf>,
    sel: usize,
    /// Byte offset of the `[[` the user closed the popup for with Esc.
    dismissed: Option<usize>,
}

struct Preview {
    title: String,
    body: String,
    pos: egui::Pos2,
    /// Pass it opened on, so the opening click doesn't close it.
    opened: u64,
}

impl Notefy {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, vault: PathBuf) -> Self {
        install_fonts(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);
        let mut app = Self::with_vault(vault);
        app.index_at = Some(Instant::now() + INDEX_DELAY);
        app
    }

    /// Start embedding the vault in the background so the first search is quick.
    fn index_notes(&self) {
        let paths = self.notes.iter().map(|n| n.path.clone()).collect();
        self.semantic
            .request(self.vault.clone(), paths, String::new());
    }

    /// App state for `vault`, without any egui setup (also used by tests).
    fn with_vault(vault: PathBuf) -> Self {
        let _ = fs::create_dir_all(&vault);
        let mut app = Self {
            vault,
            notes: vec![],
            tree: Dir::default(),
            expanded: HashSet::new(),
            current: None,
            tabs: vec![Tab::default()],
            active: 0,
            buffer: String::new(),
            dirty: false,
            last_edit: Instant::now(),
            title_edit: String::new(),
            focus_title: false,
            sidebar_open: true,
            side_view: SideView::Files,
            sort_desc: false,
            bookmarks: load_paths("bookmarks"),
            new_folder: None,
            pending_delete: None,
            pending_link: None,
            link: LinkPopup::default(),
            preview: None,
            search: SearchState {
                recent: load_paths("recent"),
                ..Default::default()
            },
            semantic: Semantic::new(),
            index_at: None,
            graph: None,
            skip_delete_confirm: config_file("skip_delete_confirm").exists(),
            status: String::new(),
            status_at: Instant::now(),
        };
        app.refresh();
        if app.notes.is_empty() {
            let p = app.vault.join("Welcome.md");
            let _ = fs::write(&p, WELCOME);
            app.refresh();
        }
        if let Some(first) = app.notes.first().map(|n| n.path.clone()) {
            app.open(first);
        }
        app
    }

    pub(crate) fn refresh(&mut self) {
        let mut notes = vec![];
        self.tree = scan_dir(&self.vault, &self.vault, &mut notes).unwrap_or_else(|| Dir {
            path: self.vault.clone(),
            ..Default::default()
        });
        notes.sort_by_key(|n| n.name.to_lowercase());
        self.notes = notes;
    }

    /// Show `msg` in the status corner for a few seconds.
    pub(crate) fn flash(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_at = Instant::now();
    }

    /// Let the user choose a new root folder, then rescan it.
    pub(crate) fn pick_vault(&mut self) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title("Choose notes folder")
            .set_directory(&self.vault)
            .pick_folder()
        else {
            return;
        };
        self.commit_title();
        self.save();
        self.vault = dir;
        save_vault_config(&self.vault);
        self.expanded.clear();
        self.tabs = vec![Tab::default()];
        self.active = 0;
        self.current = None;
        self.buffer.clear();
        self.dirty = false;
        self.refresh();
        self.index_notes();
        if let Some(first) = self.notes.first().map(|n| n.path.clone()) {
            self.open(first);
        }
        self.flash(format!("Opened {}", self.vault.display()));
    }

    pub(crate) fn save(&mut self) {
        if let (Some(p), true) = (&self.current, self.dirty) {
            match fs::write(p, &self.buffer) {
                Ok(_) => self.dirty = false,
                Err(e) => self.flash(format!("Save failed: {e}")),
            }
        }
    }

    // ---------- tabs & history ----------
    // Every change of the open note goes through `load`, which keeps
    // `current`, `tabs[active].path`, `buffer` and `title_edit` in sync.

    /// Show `path` (or an empty tab) in the active tab, saving the current note first.
    fn load(&mut self, path: Option<PathBuf>) {
        let before = self.current.clone();
        self.graph = None;
        self.commit_title();
        self.save();
        // `path` may name the note the title edit just renamed (same note in two tabs)
        let mut path = match (path, before) {
            (Some(p), Some(old)) if p == old => self.current.clone(),
            (p, _) => p,
        };
        self.buffer = match path.as_deref().map(fs::read_to_string) {
            Some(Ok(text)) => text,
            // never open a note we couldn't read: saving would overwrite it with ""
            Some(Err(e)) => {
                self.flash(format!("Can't open note: {e}"));
                path = None;
                String::new()
            }
            None => String::new(),
        };
        if let Some(p) = &path {
            self.expand_to(p);
        }
        self.tabs[self.active].path = path.clone();
        self.current = path;
        self.title_edit = self.current_title();
        self.dirty = false;
    }

    /// Navigate the active tab to `path`, recording it in the tab's history.
    pub(crate) fn open(&mut self, path: PathBuf) {
        if self.current.as_ref() == Some(&path) {
            return;
        }
        if let Some(old) = self.current.clone() {
            let tab = &mut self.tabs[self.active];
            tab.back.push(old);
            tab.fwd.clear();
        }
        self.load(Some(path));
    }

    /// Go back (`true`) or forward (`false`) in the active tab's history.
    pub(crate) fn step_history(&mut self, back: bool) {
        let tab = &mut self.tabs[self.active];
        let (from, to) = if back {
            (&mut tab.back, &mut tab.fwd)
        } else {
            (&mut tab.fwd, &mut tab.back)
        };
        let Some(target) = from.pop() else { return };
        if let Some(cur) = self.current.clone() {
            to.push(cur);
        }
        self.load(Some(target));
    }

    pub(crate) fn switch_tab(&mut self, i: usize) {
        if i == self.active || i >= self.tabs.len() {
            return;
        }
        // load() saves the note we leave before `current` changes
        self.active = i;
        let path = self.tabs[i].path.clone();
        self.load(path);
    }

    /// Open a tab right after the active one, showing `path` (or empty).
    fn insert_tab(&mut self, path: Option<PathBuf>) {
        let i = (self.active + 1).min(self.tabs.len());
        self.tabs.insert(i, Tab::default());
        self.active = i;
        self.load(path);
    }

    /// Is the active tab in reading mode?
    pub(crate) fn reading(&self) -> bool {
        self.tabs[self.active].reading
    }

    pub(crate) fn toggle_reading(&mut self) {
        self.commit_title();
        let tab = &mut self.tabs[self.active];
        tab.reading = !tab.reading;
    }

    pub(crate) fn new_tab(&mut self) {
        self.insert_tab(None);
    }

    /// Open `path` in a new tab, or switch to the tab already showing it.
    pub(crate) fn open_new_tab(&mut self, path: PathBuf) {
        match self
            .tabs
            .iter()
            .position(|t| t.path.as_ref() == Some(&path))
        {
            Some(i) => self.switch_tab(i),
            None => self.insert_tab(Some(path)),
        }
    }

    pub(crate) fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if i == self.active {
            self.commit_title();
            self.save();
        }
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::default());
        }
        if i < self.active {
            self.active -= 1;
        } else if i == self.active {
            self.active = i.min(self.tabs.len() - 1);
            let path = self.tabs[self.active].path.clone();
            self.current = None; // already saved; don't let load() save into the closed note
            self.load(path);
        }
    }

    // ---------- notes & folders ----------

    /// Folder for new notes/folders: the open note's folder, else the vault root.
    pub(crate) fn base_dir(&self) -> PathBuf {
        self.current
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.vault.clone())
    }

    /// Expand every folder above `path` in the tree.
    pub(crate) fn expand_to(&mut self, path: &Path) {
        for a in path.ancestors().skip(1) {
            if !a.starts_with(&self.vault) || a == self.vault {
                break;
            }
            self.expanded.insert(a.to_path_buf());
        }
    }

    pub(crate) fn new_note(&mut self) {
        let dir = self.base_dir();
        let mut i = 0;
        let path = loop {
            let name = if i == 0 {
                "Untitled.md".to_string()
            } else {
                format!("Untitled {i}.md")
            };
            let p = dir.join(name);
            if !p.exists() {
                break p;
            }
            i += 1;
        };
        if let Err(e) = fs::write(&path, "") {
            self.flash(format!("Can't create note: {e}"));
            return;
        }
        self.refresh();
        self.open(path);
        self.focus_title = true;
    }

    pub(crate) fn create_folder(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
            self.flash("Invalid folder name");
            return;
        }
        let dir = self.base_dir().join(name);
        if dir.exists() {
            self.flash(format!("\"{name}\" already exists"));
            return;
        }
        if let Err(e) = fs::create_dir(&dir) {
            self.flash(format!("Can't create folder: {e}"));
            return;
        }
        self.refresh();
        self.expand_to(&dir);
        self.expanded.insert(dir);
    }

    /// Rename the open note to the inline title if it was edited.
    pub(crate) fn commit_title(&mut self) {
        if self.current.is_some() && self.title_edit != self.current_title() {
            let t = self.title_edit.clone();
            self.rename_current(&t);
        }
    }

    pub(crate) fn rename_current(&mut self, new_name: &str) {
        let new_name = new_name.trim().trim_end_matches(".md");
        if let Some(old) = self.current.clone() {
            let new = old.with_file_name(format!("{new_name}.md"));
            if new_name.is_empty() || new_name.contains(['/', '\\']) {
                self.flash("Invalid note name");
            } else if new != old && new.exists() {
                self.flash(format!("\"{new_name}\" already exists"));
            } else if new != old {
                self.save();
                match fs::rename(&old, &new) {
                    Ok(_) => {
                        self.remap_paths(|p| Some(rebase(p, &old, &new).unwrap_or(p.into())));
                        self.refresh();
                    }
                    Err(e) => self.flash(format!("Rename failed: {e}")),
                }
            }
        }
        self.title_edit = self.current_title();
    }

    pub(crate) fn current_title(&self) -> String {
        self.current
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub(crate) fn find_note(&self, target: &str) -> Option<PathBuf> {
        find_note(&self.notes, target).map(|n| n.path.clone())
    }

    /// Open the linked note in a new tab, creating it if missing.
    pub(crate) fn open_wikilink(&mut self, target: &str) {
        let path = self.find_note(target).unwrap_or_else(|| {
            let target = target.trim().trim_end_matches(".md");
            let p = self.vault.join(format!("{target}.md"));
            let _ = fs::write(&p, "");
            self.refresh();
            p
        });
        self.open_new_tab(path);
    }

    pub(crate) fn toggle_bookmark(&mut self) {
        let Some(cur) = self.current.clone() else {
            return;
        };
        if self.bookmarks.contains(&cur) {
            self.bookmarks.retain(|b| b != &cur);
        } else {
            self.bookmarks.push(cur);
        }
        save_paths("bookmarks", &self.bookmarks);
    }

    /// Rewrite every remembered note path: open note, tabs + their history, bookmarks.
    /// `f` returns the new path, or `None` to forget it (deleted file).
    fn remap_paths(&mut self, f: impl Fn(&Path) -> Option<PathBuf>) {
        let map = |v: &mut Vec<PathBuf>| *v = v.iter().filter_map(|p| f(p)).collect();
        for t in &mut self.tabs {
            t.path = t.path.as_deref().and_then(&f);
            map(&mut t.back);
            map(&mut t.fwd);
        }
        map(&mut self.bookmarks);
        self.current = self.current.as_deref().and_then(&f);
        save_paths("bookmarks", &self.bookmarks);
    }

    pub(crate) fn delete_file(&mut self, path: &Path) {
        if let Err(e) = fs::remove_file(path) {
            self.flash(format!("Delete failed: {e}"));
            return;
        }
        if self.current.as_deref() == Some(path) {
            // nothing left to save; tabs showing it become empty "New tab"s
            self.dirty = false;
            self.buffer.clear();
        }
        self.remap_paths(|p| (p != path).then(|| p.to_path_buf()));
        self.title_edit = self.current_title();
        self.refresh();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.flash(format!("Deleted {name}"));
    }

    /// Move a file or folder `src` into the folder `dest_dir`.
    pub(crate) fn move_item(&mut self, src: &Path, dest_dir: &Path) {
        self.save();
        let dest = match move_path(src, dest_dir) {
            Ok(Some(d)) => d,
            Ok(None) => return, // dropped where it already is
            Err(e) => {
                self.flash(e);
                return;
            }
        };
        // keep open notes, history, bookmarks and expanded folders pointing at the new place
        self.remap_paths(|p| Some(rebase(p, src, &dest).unwrap_or(p.into())));
        self.expanded = self
            .expanded
            .iter()
            .map(|p| rebase(p, src, &dest).unwrap_or(p.clone()))
            .collect();
        self.expanded.insert(dest_dir.to_path_buf());
        self.title_edit = self.current_title();
        self.refresh();
        let name = dest.file_name().unwrap_or_default().to_string_lossy();
        self.flash(format!("Moved {name}"));
    }

    pub(crate) fn toggle_search(&mut self) {
        if self.search.open {
            self.search.open = false;
        } else {
            self.search.open = true;
            self.search.query.clear();
            self.search.sel = 0;
            self.search.focus = true;
            self.search.last_query = None;
        }
    }

    pub(crate) fn pal(&self) -> &'static Palette {
        &DARK
    }

    /// Global keyboard shortcuts (consumed before any widget sees them).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::Modifiers as M;
        let ctrl_shift = M::COMMAND | M::SHIFT;
        let k = ctx.input_mut(|i| {
            [
                i.consume_key(M::COMMAND, Key::S),
                i.consume_key(M::COMMAND, Key::N),
                i.consume_key(M::COMMAND, Key::Space),
                i.consume_key(M::COMMAND, Key::T),
                i.consume_key(M::COMMAND, Key::W),
                i.consume_key(M::ALT, Key::ArrowLeft),
                i.consume_key(M::ALT, Key::ArrowRight),
                i.consume_key(ctrl_shift, Key::Tab),
                i.consume_key(M::COMMAND, Key::Tab),
                i.consume_key(M::COMMAND, Key::Backslash),
                i.consume_key(M::COMMAND, Key::E),
            ]
        });
        let [
            save,
            new,
            search,
            tab,
            close,
            back,
            fwd,
            prev,
            next,
            sidebar,
            read,
        ] = k;
        if read {
            self.toggle_reading();
        }
        let n = self.tabs.len();
        if save {
            self.commit_title();
            self.save();
            self.flash("Saved");
        }
        if new {
            self.new_note();
        }
        if search {
            self.toggle_search();
        }
        if tab {
            self.new_tab();
        }
        if close {
            self.close_tab(self.active);
        }
        if back || fwd {
            self.step_history(back);
        }
        if next {
            self.switch_tab((self.active + 1) % n);
        }
        if prev {
            self.switch_tab((self.active + n - 1) % n);
        }
        if sidebar {
            self.sidebar_open = !self.sidebar_open;
        }
    }

    /// Obsidian-style status corner: word count plus the latest message.
    fn status_corner(&self, ui: &egui::Ui) {
        let p = self.pal();
        let mut parts = vec![];
        if self.status_at.elapsed() < Duration::from_secs(STATUS_SECS) && !self.status.is_empty() {
            parts.push(self.status.clone());
            ui.ctx()
                .request_repaint_after(Duration::from_secs(STATUS_SECS));
        }
        if self.current.is_some() {
            let words = self.buffer.split_whitespace().count();
            let chars = self.buffer.chars().count();
            parts.push(format!("{words} words  {chars} characters"));
        }
        if parts.is_empty() {
            return;
        }
        let painter = ui.painter();
        let galley = painter.layout_no_wrap(
            parts.join("   ·   "),
            egui::FontId::proportional(12.0),
            p.muted,
        );
        let rect = egui::Rect::from_min_size(
            ui.max_rect().right_bottom() - galley.size() - egui::vec2(20.0, 10.0),
            galley.size(),
        )
        .expand2(egui::vec2(10.0, 5.0));
        painter.rect(
            rect,
            egui::CornerRadius {
                nw: 6,
                ne: 0,
                sw: 0,
                se: 0,
            },
            p.panel,
            Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + egui::vec2(10.0, 5.0), galley, p.muted);
    }
}

impl eframe::App for Notefy {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let p = self.pal();
        self.shortcuts(&ctx);

        // load the model only once the window is up (a search before that starts it anyway)
        if let Some(at) = self.index_at {
            let now = Instant::now();
            if now >= at {
                self.index_at = None;
                self.index_notes();
            } else {
                ctx.request_repaint_after(at - now);
            }
        }

        // autosave after 1s idle
        if self.dirty {
            if self.last_edit.elapsed() > Duration::from_millis(1000) {
                self.save();
            } else {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }

        // the panel may close itself (dragged shut); the hide button sets the field directly
        let before = self.sidebar_open;
        let mut open = before;
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(290.0)
            .size_range(200.0..=520.0)
            .frame(egui::Frame::new().fill(p.panel))
            .show_collapsible(ui, &mut open, |ui| self.sidebar(ui));
        if open != before {
            self.sidebar_open = open;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg))
            .show(ui, |ui| {
                egui::Panel::top("tab-strip")
                    .exact_size(40.0)
                    .show_separator_line(false)
                    .frame(egui::Frame::new().fill(p.panel))
                    .show(ui, |ui| self.tab_strip(ui));
                egui::Panel::top("nav-bar")
                    .exact_size(36.0)
                    .show_separator_line(false)
                    .frame(
                        egui::Frame::new()
                            .fill(p.bg)
                            .inner_margin(Margin::symmetric(12, 0)),
                    )
                    .show(ui, |ui| self.nav_bar(ui));
                if self.graph.is_some() {
                    self.graph_view(ui);
                } else if self.current.is_some() {
                    self.editor(ui);
                } else {
                    self.empty_tab(ui);
                }
                self.status_corner(ui);
            });

        self.preview_popup(&ctx);
        self.delete_dialog(&ctx);
        self.new_folder_dialog(&ctx);
        self.search_popup(&ctx);
    }

    fn on_exit(&mut self) {
        self.commit_title();
        self.save();
    }
}

const WELCOME: &str = r#"A tiny, fast, **Markdown-first** notes app written in Rust.

## Shortcuts
- `Ctrl+N` — new note, `Ctrl+S` — save (autosave is on, too)
- `Ctrl+Space` — search files
- `Ctrl+T` / `Ctrl+W` — new / close tab, `Ctrl+Tab` — next tab
- `Alt+←` / `Alt+→` — back / forward
- `Ctrl+\` — toggle the sidebar, `Ctrl+E` — reading mode
- Click the title above to rename the note

## Links
Link notes Obsidian-style: type `[[` to pick a note, e.g. [[Ideas]]. **Click** it to open it in a new tab, **Ctrl+click** to preview.

Markdown renders *live* as you type: **bold**, *italic*, `code`, ~~strike~~, ==highlight==.

- [ ] a task
- [x] a done task

> Notes are plain `.md` files in your vault folder.

```rust
fn main() { println!("hello, notes"); }
```
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_history_rename_delete() {
        let root = std::env::temp_dir().join(format!("notefy-tabs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let v = root.join("vault");
        fs::create_dir_all(&v).unwrap();
        // SAFETY: the only test that touches config; keeps it out of the real ~/.config
        unsafe { std::env::set_var("XDG_CONFIG_HOME", root.join("config")) };
        for n in ["a", "b", "c"] {
            fs::write(v.join(format!("{n}.md")), n).unwrap();
        }
        let mut app = Notefy::with_vault(v.clone());
        let (a, b, c) = (v.join("a.md"), v.join("b.md"), v.join("c.md"));
        assert_eq!(app.current.as_ref(), Some(&a)); // first note opens on start

        // history in one tab
        app.open(b.clone());
        app.step_history(true);
        assert_eq!((app.current.as_ref(), app.buffer.as_str()), (Some(&a), "a"));
        app.step_history(false);
        assert_eq!(app.current.as_ref(), Some(&b));

        // new tab, switch, close
        app.open_new_tab(c.clone());
        assert_eq!((app.tabs.len(), app.active), (2, 1));
        app.open_new_tab(b.clone()); // already open -> switch, no new tab
        assert_eq!((app.tabs.len(), app.active), (2, 0));
        app.close_tab(0);
        assert_eq!((app.tabs.len(), app.active), (1, 0));
        assert_eq!(app.current.as_ref(), Some(&c));

        // unsaved edits survive a tab switch; inline title renames the file
        app.buffer = "edited".into();
        app.dirty = true;
        app.title_edit = "renamed".into();
        app.new_tab();
        let renamed = v.join("renamed.md");
        assert_eq!(fs::read_to_string(&renamed).unwrap(), "edited");
        assert!(!c.exists());
        assert_eq!(app.tabs[0].path.as_ref(), Some(&renamed));
        assert_eq!(app.current, None); // the new tab is empty

        // deleting the open note empties its tab and forgets it in history
        app.switch_tab(0);
        app.delete_file(&renamed);
        assert_eq!(
            (app.current.as_ref(), app.tabs[0].path.as_ref()),
            (None, None)
        );
        assert!(app.tabs[0].back.iter().all(|p| p != &renamed));
        assert!(!renamed.exists());

        // closing the last tab leaves one empty tab
        app.close_tab(1);
        app.close_tab(0);
        assert_eq!((app.tabs.len(), app.current.as_ref()), (1, None));
        fs::remove_dir_all(&root).unwrap();
    }
}
