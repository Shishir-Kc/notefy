# Notefy

A fast, Obsidian-inspired Markdown notes app written in Rust (egui/eframe).
Dark theme: charcoal black with milky-white text.

## Run

```sh
cargo run --release            # vault defaults to ~/Notefy
cargo run --release -- ~/notes # or point it at any folder
```

## Features
- **Obsidian-style layout**: sidebar (files / search / bookmarks), browser-style tabs, back/forward + breadcrumb bar, editable inline title, word count in the corner
- **Vault**: open any folder (vault name at the bottom-left) — scanned recursively into a collapsible tree; remembered in `~/.config/notefy/vault`
- **File tree**: new note / new folder (next to the open note), A→Z / Z→A sort, graph view of `[[linked]]` notes, collapse/expand all, drag & drop to move, hover trash to delete; Ctrl+click or middle-click a note to open it in a new tab
- **Tabs**: each tab has its own back/forward history; `+` opens an empty tab; middle-click or × closes
- **Live preview editor**: Markdown is styled in place as you type; syntax markers only show on the line you are editing (links only expand to raw `[[target|title]]` when the cursor touches them)
- `[[Wiki links]]`: type `[[` for note suggestions (↑↓, Enter/Tab, Esc); click a link to open it in a new tab (creates it if missing), Ctrl+click to preview it; Ctrl+click `[text](url)` opens the browser
- **Reading mode** (book icon above a note, Ctrl+E, per tab): the note is read-only and fully rendered; clicking any link opens it in a new tab, Ctrl+click previews it
- **Bookmarks**: bookmark icon above a note; listed in the sidebar's bookmarks view
- Autosave (1s idle); rename by editing the title (Enter or click away, Esc reverts)

## Shortcuts
| Key | Action |
|---|---|
| Ctrl+N | New note |
| Ctrl+S | Save |
| Ctrl+Space | Search files (↑↓, Enter, Esc) |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Alt+← / Alt+→ | Back / forward |
| Ctrl+\\ | Show / hide sidebar |
| Ctrl+E | Toggle reading mode |

## Font
Bundles **JetBrainsMono Nerd Font** (`assets/`, SIL Open Font License), the same font Omarchy uses, for text and icons (Regular, Bold, Italic). UI icons are the Codicons set inside it. The editor relies on it being monospace to line up table columns.
