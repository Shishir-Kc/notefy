//! Left panel: view switcher (files / search / bookmarks), file-tree toolbar, the vault
//! file tree with drag-and-drop moves, the bookmarks list and the vault switcher.

use super::widgets::{ICON_BTN, icon_button, truncated};
use super::*;
use crate::{icon, theme::*, vault::*};
use eframe::egui::{self, Color32, FontId, Margin, RichText, Stroke};
use std::{collections::HashSet, path::PathBuf};

/// Tree geometry: row height, indent per level, chevron centre and text start
/// (both measured from the row's left edge + level indent).
const ROW_H: f32 = 27.0;
const INDENT: f32 = 17.0;
const CHEVRON_X: f32 = 10.0;
const TEXT_X: f32 = 22.0;

impl Notefy {
    pub(crate) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let full = ui.max_rect();
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);

        // top row lines up with the tab strip (40px, border below)
        ui.allocate_ui_with_layout(
            egui::vec2(full.width(), 40.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(6.0);
                if icon_button(ui, icon::FILES, "Files", self.side_view == SideView::Files)
                    .clicked()
                {
                    self.side_view = SideView::Files;
                }
                if icon_button(ui, icon::SEARCH, "Search (Ctrl+Space)", self.search.open).clicked()
                {
                    self.toggle_search();
                }
                let on = self.side_view == SideView::Bookmarks;
                if icon_button(ui, icon::BOOKMARK, "Bookmarks", on).clicked() {
                    self.side_view = SideView::Bookmarks;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    if icon_button(ui, icon::SIDEBAR, "Hide sidebar (Ctrl+\\)", false).clicked() {
                        self.sidebar_open = false;
                    }
                });
            },
        );
        ui.painter().hline(
            full.x_range(),
            full.top() + 39.5,
            Stroke::new(1.0, p.border),
        );

        // vault switcher pinned to the bottom
        egui::Panel::bottom("vault-row")
            .exact_size(40.0)
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ui, |ui| self.vault_row(ui));

        match self.side_view {
            SideView::Files => self.files_view(ui),
            SideView::Bookmarks => self.bookmarks_view(ui),
        }
    }

    fn vault_row(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        ui.horizontal_centered(|ui| {
            let name = self
                .vault
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.vault.display().to_string());
            let label = RichText::new(format!("{}  {name}", icon::VAULT))
                .size(13.5)
                .color(p.text);
            let tip = format!("{}\nClick to open another folder", self.vault.display());
            if ui
                .add(egui::Button::new(label).frame(false))
                .on_hover_text(tip)
                .clicked()
            {
                self.pick_vault();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, icon::REFRESH, "Rescan folder", false).clicked() {
                    self.refresh();
                }
            });
        });
    }

    fn files_view(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        // toolbar, centred like Obsidian's
        let w = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(w, 40.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let icons = 5.0;
                ui.add_space(((w - icons * ICON_BTN - (icons - 1.0) * 4.0) / 2.0).max(0.0));
                if icon_button(ui, icon::NEW_NOTE, "New note (Ctrl+N)", false).clicked() {
                    self.new_note();
                }
                if icon_button(ui, icon::NEW_FOLDER, "New folder", false).clicked() {
                    self.new_folder = Some(String::new());
                }
                let tip = if self.sort_desc {
                    "Sorted Z → A (click for A → Z)"
                } else {
                    "Sorted A → Z (click for Z → A)"
                };
                if icon_button(ui, icon::SORT, tip, self.sort_desc).clicked() {
                    self.sort_desc = !self.sort_desc;
                }
                let on = self.graph.is_some();
                if icon_button(ui, icon::GRAPH, "Graph view: linked notes", on).clicked() {
                    // the graph reads notes from disk: flush the open note's unsaved links first
                    self.commit_title();
                    self.save();
                    self.graph = if on {
                        None
                    } else {
                        Some(Graph::new(&self.notes))
                    };
                }
                let any_open = !self.expanded.is_empty();
                let (ic, tip) = if any_open {
                    (icon::COLLAPSE_ALL, "Collapse all")
                } else {
                    (icon::EXPAND_ALL, "Expand all")
                };
                if icon_button(ui, ic, tip, false).clicked() {
                    if any_open {
                        self.expanded.clear();
                    } else {
                        collect_dirs(&self.tree, &mut self.expanded);
                    }
                }
            },
        );

        let mut to_open: Option<(PathBuf, bool)> = None;
        let mut move_req: Option<(PathBuf, PathBuf)> = None;
        let mut delete_req: Option<PathBuf> = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        let root = &self.tree;
                        if root.dirs.is_empty() && root.files.is_empty() {
                            ui.label(RichText::new("No notes here yet").color(p.muted));
                        }
                        let mut tc = TreeCtx {
                            current: self.current.as_ref(),
                            expanded: &mut self.expanded,
                            to_open: &mut to_open,
                            move_req: &mut move_req,
                            delete_req: &mut delete_req,
                            sort_desc: self.sort_desc,
                            p,
                        };
                        render_tree(ui, root, &mut tc, 0);

                        // empty space below the tree = drop into the vault root
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ui.available_height().max(40.0)),
                            egui::Sense::hover(),
                        );
                        if resp.dnd_hover_payload::<DragItem>().is_some() {
                            ui.painter().rect_stroke(
                                rect.shrink(2.0),
                                4.0,
                                Stroke::new(1.0, p.link),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if let Some(item) = resp.dnd_release_payload::<DragItem>() {
                            move_req = Some((item.0.clone(), root.path.clone()));
                        }
                    });
            });
        match to_open {
            Some((path, true)) => self.open_new_tab(path),
            Some((path, false)) => self.open(path),
            None => {}
        }
        if let Some((src, dest)) = move_req {
            self.move_item(&src, &dest);
        }
        if let Some(path) = delete_req {
            if self.skip_delete_confirm {
                self.delete_file(&path);
            } else {
                self.pending_delete = Some(path);
            }
        }
        drag_label(ui, p);
    }

    fn bookmarks_view(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        ui.add_space(8.0);
        let marks: Vec<PathBuf> = self
            .bookmarks
            .iter()
            .filter(|b| b.exists())
            .cloned()
            .collect();
        if marks.is_empty() {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(16, 8))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("No bookmarks yet. Use the bookmark icon above a note.")
                            .size(13.0)
                            .color(p.muted),
                    );
                });
            return;
        }
        let (mut to_open, mut remove) = (None, None);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        for path in &marks {
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), ROW_H),
                                egui::Sense::click(),
                            );
                            let selected = self.current.as_ref() == Some(path);
                            if selected || resp.hovered() {
                                ui.painter().rect_filled(
                                    rect.shrink2(egui::vec2(0.0, 1.0)),
                                    4.0,
                                    p.surface,
                                );
                            }
                            let cy = rect.center().y;
                            ui.painter().text(
                                egui::pos2(rect.left() + CHEVRON_X, cy),
                                egui::Align2::CENTER_CENTER,
                                icon::BOOKMARK,
                                FontId::proportional(13.0),
                                p.muted,
                            );
                            let name = path.file_stem().unwrap_or_default().to_string_lossy();
                            let g =
                                truncated(ui, &name, 14.0, p.text, rect.width() - TEXT_X - 30.0);
                            ui.painter().galley(
                                egui::pos2(rect.left() + TEXT_X, cy - g.size().y / 2.0),
                                g,
                                p.text,
                            );
                            if resp.contains_pointer()
                                && row_button(
                                    ui,
                                    rect,
                                    icon::CLOSE,
                                    "Remove bookmark",
                                    ("unmark", path),
                                )
                            {
                                remove = Some(path.clone());
                            } else if resp.clicked() {
                                to_open = Some(path.clone());
                            }
                        }
                    });
            });
        if let Some(path) = remove {
            self.bookmarks.retain(|b| b != &path);
            save_paths("bookmarks", &self.bookmarks);
        } else if let Some(path) = to_open {
            self.open(path);
        }
    }
}

/// Drag-and-drop payload: the file or folder being dragged.
struct DragItem(PathBuf);

struct TreeCtx<'a> {
    current: Option<&'a PathBuf>,
    expanded: &'a mut HashSet<PathBuf>,
    /// (note, open in a new tab?)
    to_open: &'a mut Option<(PathBuf, bool)>,
    /// (source, destination folder)
    move_req: &'a mut Option<(PathBuf, PathBuf)>,
    delete_req: &'a mut Option<PathBuf>,
    sort_desc: bool,
    /// Scroll the open note's row into view.
    p: &'a Palette,
}

/// Every folder in the tree (for "expand all").
fn collect_dirs(dir: &Dir, out: &mut HashSet<PathBuf>) {
    for d in &dir.dirs {
        out.insert(d.path.clone());
        collect_dirs(d, out);
    }
}

/// Draw `dir`'s children: folders (with chevrons) first, then notes, each level indented
/// with a vertical guide line under the parent's chevron.
fn render_tree(ui: &mut egui::Ui, dir: &Dir, tc: &mut TreeCtx, depth: usize) {
    let p = tc.p;
    let mut items: Vec<(&PathBuf, String, Option<&Dir>)> = dir
        .dirs
        .iter()
        .map(|d| (&d.path, d.name.clone(), Some(d)))
        .collect();
    let mut files: Vec<(&PathBuf, String, Option<&Dir>)> = dir
        .files
        .iter()
        .map(|f| {
            (
                f,
                f.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                None,
            )
        })
        .collect();
    if tc.sort_desc {
        items.reverse();
        files.reverse();
    }
    items.extend(files);

    for (path, label, sub) in items {
        let is_dir = sub.is_some();
        let open = is_dir && tc.expanded.contains(path);
        let selected = !is_dir && tc.current == Some(path);

        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_H),
            egui::Sense::click_and_drag(),
        );
        resp.dnd_set_drag_payload(DragItem(path.clone()));
        // dropping on a folder moves into it; dropping on a note moves next to it
        let drop_dir = if is_dir {
            path.clone()
        } else {
            dir.path.clone()
        };
        let drop_hover = resp.dnd_hover_payload::<DragItem>().is_some();
        if let Some(item) = resp.dnd_release_payload::<DragItem>() {
            *tc.move_req = Some((item.0.clone(), drop_dir));
        }

        let painter = ui.painter();
        let row = rect.shrink2(egui::vec2(0.0, 1.0));
        if drop_hover {
            painter.rect_filled(row, 4.0, p.surface);
            painter.rect_stroke(row, 4.0, Stroke::new(1.0, p.link), egui::StrokeKind::Inside);
        } else if selected {
            painter.rect_filled(row, 4.0, p.surface);
        } else if resp.hovered() {
            painter.rect_filled(row, 4.0, p.surface.gamma_multiply(0.6));
        }
        for k in 0..depth {
            let x = rect.left() + k as f32 * INDENT + CHEVRON_X;
            painter.vline(x, rect.y_range(), Stroke::new(1.0, p.border));
        }
        let x0 = rect.left() + depth as f32 * INDENT;
        let cy = rect.center().y;
        if is_dir {
            painter.text(
                egui::pos2(x0 + CHEVRON_X, cy),
                egui::Align2::CENTER_CENTER,
                if open {
                    icon::CHEVRON_DOWN
                } else {
                    icon::CHEVRON
                },
                FontId::proportional(13.0),
                p.muted,
            );
        }
        let color = if selected || resp.hovered() {
            p.text
        } else {
            p.text.gamma_multiply(0.82)
        };
        let g = truncated(ui, &label, 14.0, color, rect.right() - x0 - TEXT_X - 26.0);
        ui.painter()
            .galley(egui::pos2(x0 + TEXT_X, cy - g.size().y / 2.0), g, color);

        let trashed = !is_dir
            && resp.contains_pointer()
            && row_button(ui, rect, icon::TRASH, "Delete", ("trash", path));
        if trashed {
            *tc.delete_req = Some(path.clone());
        } else if resp.clicked() || resp.middle_clicked() {
            if is_dir {
                if !tc.expanded.remove(path) {
                    tc.expanded.insert(path.clone());
                }
            } else {
                let new_tab = resp.middle_clicked() || ui.input(|i| i.modifiers.command);
                *tc.to_open = Some((path.clone(), new_tab));
            }
        }

        if open && let Some(d) = sub {
            render_tree(ui, d, tc, depth + 1);
        }
    }
}

/// Small hover button at the right end of a row; returns true when clicked.
fn row_button(
    ui: &mut egui::Ui,
    row: egui::Rect,
    icon: &str,
    tip: &str,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    let p = &DARK;
    let r = egui::Rect::from_center_size(
        egui::pos2(row.right() - 14.0, row.center().y),
        egui::vec2(22.0, 20.0),
    );
    let b = ui
        .interact(r, egui::Id::unique(id), egui::Sense::click())
        .on_hover_text(tip);
    let color = if b.hovered() && icon == icon::TRASH {
        Color32::from_rgb(0xe0, 0x6c, 0x60)
    } else if b.hovered() {
        p.text
    } else {
        p.muted
    };
    if b.hovered() {
        ui.painter().rect_filled(r, 4.0, p.bg);
    }
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::proportional(13.0),
        color,
    );
    b.clicked()
}

/// Floating label that follows the pointer while dragging a tree item.
fn drag_label(ui: &egui::Ui, p: &Palette) {
    let (Some(item), Some(pos)) = (
        egui::DragAndDrop::payload::<DragItem>(ui.ctx()),
        ui.ctx().pointer_latest_pos(),
    ) else {
        return;
    };
    let name = item
        .0
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("notefy-dnd-preview"),
    ));
    let ic = if item.0.is_dir() {
        icon::FOLDER
    } else {
        icon::FILE
    };
    let galley =
        painter.layout_no_wrap(format!("{ic}  {name}"), FontId::proportional(13.0), p.text);
    let r = egui::Rect::from_min_size(pos + egui::vec2(14.0, 6.0), galley.size())
        .expand2(egui::vec2(8.0, 4.0));
    painter.rect(
        r,
        4.0,
        p.surface,
        Stroke::new(1.0, p.border),
        egui::StrokeKind::Outside,
    );
    painter.galley(r.min + egui::vec2(8.0, 4.0), galley, p.text);
}
