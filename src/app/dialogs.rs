//! Modal popups: Ctrl+Space file search and the delete confirmation.

use super::*;
use crate::{config::*, icon, vault::*};
use eframe::egui::{self, Color32, FontId, Key, Margin, RichText, Stroke};
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct SearchState {
    pub(crate) open: bool,
    pub(crate) query: String,
    /// Query the cached `results` were computed for.
    pub(crate) last_query: Option<String>,
    pub(crate) results: Vec<PathBuf>,
    /// Waiting for the semantic worker to answer `query`.
    pub(crate) pending: bool,
    pub(crate) sel: usize,
    pub(crate) focus: bool,
    /// Files recently opened from the search popup (newest first).
    pub(crate) recent: Vec<PathBuf>,
}

impl Notefy {
    /// Ctrl+Space search popup.
    pub(crate) fn search_popup(&mut self, ctx: &egui::Context) {
        if !self.search.open {
            return;
        }
        let p = self.pal();

        // recompute matches only when the query changes
        if self.search.last_query.as_deref() != Some(self.search.query.as_str()) {
            self.search.results = search_notes(&self.notes, &self.search.query);
            self.search.last_query = Some(self.search.query.clone());
            self.search.sel = 0;
            self.search.pending = !self.search.query.trim().is_empty();
            if self.search.pending {
                let paths = self.notes.iter().map(|n| n.path.clone()).collect();
                self.semantic
                    .request(self.vault.clone(), paths, self.search.query.clone());
            }
        }
        // semantic hits go after the name/text matches; answers for older queries are dropped
        while let Some((q, res)) = self.semantic.poll() {
            if q != self.search.query || !self.search.pending {
                continue;
            }
            self.search.pending = false;
            match res {
                Ok(hits) => {
                    for h in hits {
                        if !self.search.results.contains(&h) {
                            self.search.results.push(h);
                        }
                    }
                }
                Err(e) => self.flash(e),
            }
        }
        if self.search.pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let showing_recent = self.search.query.trim().is_empty();
        let list: Vec<PathBuf> = if showing_recent {
            self.search
                .recent
                .iter()
                .filter(|p| p.exists())
                .cloned()
                .collect()
        } else {
            self.search.results.clone()
        };

        // keyboard nav (consumed before the text box sees it)
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        if !list.is_empty() {
            if down {
                self.search.sel = (self.search.sel + 1) % list.len();
            }
            if up {
                self.search.sel = (self.search.sel + list.len() - 1) % list.len();
            }
        }
        self.search.sel = self.search.sel.min(list.len().saturating_sub(1));
        let mut chosen = if enter {
            list.get(self.search.sel).cloned()
        } else {
            None
        };
        let mut close = esc;

        let modal = egui::Modal::new(egui::Id::unique("notefy-search-modal"))
            .area(
                egui::Modal::default_area(egui::Id::unique("notefy-search-area"))
                    .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 110.0)),
            )
            .backdrop_color(Color32::from_black_alpha(110))
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .stroke(Stroke::new(2.0, p.text))
                    .corner_radius(0)
                    .inner_margin(Margin::symmetric(18, 18)),
            )
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

                // --- prompt (no box, like the Omarchy menu)
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.search.query)
                        .hint_text(RichText::new("Search…").color(p.muted))
                        .frame(egui::Frame::NONE)
                        .margin(Margin::symmetric(2, 4))
                        .font(FontId::proportional(15.0))
                        .text_color(p.text)
                        .desired_width(f32::INFINITY),
                );
                if self.search.focus {
                    r.request_focus();
                    self.search.focus = false;
                }
                ui.add_space(14.0);

                if list.is_empty() {
                    let msg = if showing_recent {
                        "Type to search your notes"
                    } else if self.search.pending {
                        "Searching…"
                    } else {
                        "No matching notes"
                    };
                    ui.add_space(6.0);
                    ui.label(RichText::new(msg).size(13.0).color(p.muted));
                    ui.add_space(8.0);
                    return;
                }

                const ROW: f32 = 46.0;
                // explicit height: an Area lays out inside last frame's size, so the
                // list would stay as small as the "type to search" message otherwise
                let list_h = (list.len() as f32 * ROW).min(ROW * 9.0);
                egui::ScrollArea::vertical()
                    .max_height(list_h)
                    .min_scrolled_height(list_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, path) in list.iter().enumerate() {
                            let name = path
                                .file_stem()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let folder = path
                                .parent()
                                .and_then(|d| d.strip_prefix(&self.vault).ok())
                                .map(|d| d.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), ROW),
                                egui::Sense::click(),
                            );
                            if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                            {
                                self.search.sel = i;
                            }
                            let active = i == self.search.sel;
                            let painter = ui.painter();
                            if active {
                                painter.rect_filled(rect, 0.0, p.surface);
                                if down || up {
                                    resp.scroll_to_me(None);
                                }
                            }
                            let cy = rect.center().y;
                            let color = if active { p.link } else { p.text };
                            painter.text(
                                egui::pos2(rect.left() + 16.0, cy),
                                egui::Align2::LEFT_CENTER,
                                if showing_recent {
                                    icon::CLOCK
                                } else {
                                    icon::FILE
                                },
                                FontId::proportional(15.0),
                                color,
                            );
                            painter.text(
                                egui::pos2(rect.left() + 48.0, cy),
                                egui::Align2::LEFT_CENTER,
                                &name,
                                FontId::proportional(15.0),
                                color,
                            );
                            let mut right = rect.right() - 16.0;
                            painter.text(
                                egui::pos2(right, cy),
                                egui::Align2::RIGHT_CENTER,
                                icon::CHEVRON,
                                FontId::proportional(11.0),
                                p.muted,
                            );
                            right -= 22.0;
                            if !folder.is_empty() {
                                painter.text(
                                    egui::pos2(right, cy),
                                    egui::Align2::RIGHT_CENTER,
                                    &folder,
                                    FontId::proportional(12.0),
                                    p.muted,
                                );
                            }
                            if resp.clicked() {
                                chosen = Some(path.clone());
                            }
                        }
                    });
            });
        if modal.should_close() {
            close = true;
        }

        if let Some(path) = chosen {
            self.search.recent.retain(|r| r != &path);
            self.search.recent.insert(0, path.clone());
            self.search.recent.truncate(8);
            save_paths("recent", &self.search.recent);
            self.search.open = false;
            self.open(path);
        } else if close {
            self.search.open = false;
        }
    }

    /// "Are you sure?" popup for `pending_delete`.
    pub(crate) fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.pending_delete.clone() else {
            return;
        };
        let p = self.pal();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut choice = None; // Some(true) = delete, Some(false) = cancel
        let mut remember = false;
        let modal = egui::Modal::new(egui::Id::unique("notefy-delete-modal"))
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(
                    RichText::new(format!("{}  Delete note", icon::TRASH))
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(8.0);
                ui.label(format!("Are you sure you want to delete {name} ?"));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("Don't remind me again").color(p.muted))
                        .on_hover_text("Delete now and skip this popup from now on")
                        .clicked()
                    {
                        remember = true;
                        choice = Some(true);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let del = egui::Button::new(
                            RichText::new("Delete anyway")
                                .color(Color32::from_rgb(0xf3, 0xef, 0xe6)),
                        )
                        .fill(Color32::from_rgb(0x8c, 0x3b, 0x34));
                        if ui.add(del).clicked() {
                            choice = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            choice = Some(false);
                        }
                    });
                });
            });
        if modal.should_close() && choice.is_none() {
            choice = Some(false); // Esc or click outside
        }
        match choice {
            Some(true) => {
                if remember {
                    self.skip_delete_confirm = true;
                    write_config(config_file("skip_delete_confirm"), "1");
                }
                self.pending_delete = None;
                self.delete_file(&path);
            }
            Some(false) => self.pending_delete = None,
            None => {}
        }
    }

    /// "New folder" name prompt; the folder is created next to the open note.
    pub(crate) fn new_folder_dialog(&mut self, ctx: &egui::Context) {
        if self.new_folder.is_none() {
            return;
        }
        let p = self.pal();
        let base = self.base_dir();
        let place = match base.strip_prefix(&self.vault) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
            _ => "the vault root".into(),
        };
        let mut choice = None; // Some(true) = create, Some(false) = cancel
        let name = self.new_folder.as_mut().expect("checked above");
        let modal = egui::Modal::new(egui::Id::unique("notefy-folder-modal"))
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(
                    RichText::new(format!("{}  New folder", icon::NEW_FOLDER))
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("in {place}"))
                        .size(12.5)
                        .color(p.muted),
                );
                ui.add_space(12.0);
                let r = ui.add(
                    egui::TextEdit::singleline(name)
                        .hint_text("Folder name")
                        .desired_width(f32::INFINITY),
                );
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    choice = Some(true);
                }
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Create").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
        if modal.should_close() && choice.is_none() {
            choice = Some(false); // Esc or click outside
        }
        match choice {
            Some(true) => {
                let name = self.new_folder.take().unwrap_or_default();
                self.create_folder(&name);
            }
            Some(false) => self.new_folder = None,
            None => {}
        }
    }
}
