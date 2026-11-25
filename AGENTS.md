# Notefy — guide for contributors and AI agents

Obsidian-style Markdown notes app in Rust, built on `eframe`/`egui` 0.37 (immediate mode:
the whole UI is redrawn every frame from `Notefy`'s state). Notes are plain `.md` files in
a "vault" folder.

## Commands

```sh
cargo run -- ~/notes   # run against a folder (default: last used, else ~/Notefy; a CLI path is not remembered)
cargo test             # unit tests (pure logic: parsing, scanning, search)
cargo fmt              # always format before committing
cargo clippy           # keep it warning-free
```

## Module map

| File | What lives there |
|---|---|
| `src/main.rs` | Entry point only: picks the vault path, starts eframe. |
| `src/app/mod.rs` | `Notefy` struct (all app state), tabs + per-tab history (`load`, `open`, `step_history`, `switch_tab`, `open_new_tab`, `close_tab`), notes/folders (new, rename, move, delete, bookmarks), shortcuts, the per-frame `ui()` layout. |
| `src/app/editor.rs` | Inline title + live-preview editor: `[[` suggestion popup, link click (new tab) / Ctrl+click (preview), insert at cursor, image insert, table grid lines, empty "New tab" page. |
| `src/app/header.rs` | Browser-style tab strip; nav bar (back/forward, breadcrumb, reading-mode and bookmark toggles, ⋯ menu with Insert image/table and Delete). |
| `src/app/sidebar.rs` | View switcher (files/search/bookmarks), tree toolbar, file tree with drag-and-drop, bookmarks list, vault switcher. |
| `src/app/graph.rs` | Graph view of `[[links]]` (force layout, pan/zoom, click to open), drawn in place of the editor. |
| `src/app/dialogs.rs` | Ctrl+Space search modal, delete confirmation, "New folder" prompt. |
| `src/app/widgets.rs` | Shared widgets: `icon_button`, `truncated` labels. |
| `src/live.rs` | Markdown → styled `LayoutJob` (no egui widgets), link detection (`link_at`, `link_query`). Pure functions, well tested. |
| `src/vault.rs` | Disk side: `scan_dir` (tree + note list), `move_path`, `search_notes`. No UI. |
| `src/semantic.rs` | Meaning-based search: EmbeddingGemma (via `fastembed`) on a worker thread, index saved to `<vault>/.notefy/embeddings.bin`. No UI. |
| `src/theme.rs` | Palette (`DARK`: `bg` page, `panel` sidebar/tab strip, `surface` hover), egui style, bundled fonts, `icon::*` (Codicons from the Nerd Font). |
| `src/config.rs` | Tiny files in `$XDG_CONFIG_HOME/notefy/` (vault path, recent searches, bookmarks, flags). |

UI submodules each add `impl Notefy { ... }` blocks, so all state is `self.<field>`.
Keep that pattern: new UI piece → new file in `src/app/`, methods on `Notefy`.

## Rules that are easy to break

- **`live::layout` must reproduce the source text exactly** (`job.text == src`). Hidden
  markdown markers are drawn tiny/transparent, never removed, otherwise cursor positions
  drift. The `layout_preserves_text` test guards this; extend it when adding syntax.
- **Reveal rules:** most markers show on the cursor's line (`reveal`); links show raw only
  when the cursor/selection touches them (`cursor`, bytes). Clicking a collapsed link opens
  it; clicking one the cursor is already in just edits it (`editor()` click handler).
  Reading mode (`Tab::reading`) disables both reveals and hands the `TextEdit` a `&str`
  (read-only buffer), so clicks still give a cursor for link hit-testing.
- **Char vs byte offsets:** egui cursors are char indices, Rust string slicing is bytes.
  Convert with `editor::char_to_byte`. Non-ASCII notes will panic if you mix them up.
- **Key handling order:** popups that steal keys (Enter, arrows, Esc) must
  `consume_key` *before* the `TextEdit` is shown in the same frame (see `editor()`).
- **Popups/modals (`egui::Area`, `Modal`) lay out inside last frame's size**, so a
  `ScrollArea` in one never grows when its content does. Give it an explicit height from
  the content (`max_height(h).min_scrolled_height(h)`), like `search_popup` does.
- **Never mutate `self` while a borrow from a UI closure is live** — collect the action
  (e.g. `pending_link`, `chosen`, `to_open`) and apply it after the closure.
- **Tabs:** `tabs` is never empty and `current == tabs[active].path`. Only `load()` changes
  the open note — go through `open`/`switch_tab`/`step_history`/`close_tab`, outside `mod.rs` never set
  `current` by hand. Anything that renames, moves or deletes files must call
  `remap_paths` so tabs, their history and bookmarks follow (see `rename_current`,
  `move_item`, `delete_file`). `tabs_history_rename_delete` tests this.
- **Inline title:** `title_edit` is committed (= file rename) by `commit_title()`, which
  `load()` calls first — so switching notes never loses a half-typed title.
- Never overwrite a user's file on move/rename (`move_path` refuses; keep it that way).
- Saving: `open()` saves the current note first; autosave fires 1s after the last edit.

## Style

- Small and direct: no traits/abstractions for single uses, no new dependencies for a
  few lines of code.
- Doc-comment (`///`) every struct field and function whose purpose isn't obvious; each
  file starts with a `//!` line saying what it owns.
- Pure logic goes in `live.rs` / `vault.rs` with a unit test; UI code stays in `app/`.
