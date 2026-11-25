//! Top of the editor pane: the browser-style tab strip and the navigation bar
//! (back/forward, breadcrumb, bookmark toggle, "more" menu with Insert/Delete).

use super::widgets::{ICON_BTN, icon_button, icon_button_at, truncated};
use super::*;
use crate::{icon, theme::*};
use eframe::egui::{self, CornerRadius, RichText, Stroke};

/// Widest a tab gets; tabs shrink (down to `TAB_MIN`) when many are open.
const TAB_MAX: f32 = 220.0;
const TAB_MIN: f32 = 70.0;

impl Notefy {
    pub(crate) fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let strip = ui.max_rect();
        let border = Stroke::new(1.0, p.border);
        ui.painter()
            .hline(strip.x_range(), strip.bottom() - 0.5, border);
        let mut x = strip.left() + 8.0;
        let cy = strip.center().y;

        if !self.sidebar_open {
            let r = egui::Rect::from_center_size(
                egui::pos2(x + ICON_BTN / 2.0, cy),
                egui::Vec2::splat(ICON_BTN),
            );
            if icon_button_at(ui, r, icon::SIDEBAR, "Show sidebar (Ctrl+\\)", false).clicked() {
                self.sidebar_open = true;
            }
            x += ICON_BTN + 8.0;
        }

        let n = self.tabs.len();
        let room = strip.right() - x - ICON_BTN - 16.0;
        let tab_w = (room / n as f32).clamp(TAB_MIN, TAB_MAX);
        let (mut switch, mut close) = (None, None);
        for i in 0..n {
            let r = egui::Rect::from_min_max(
                egui::pos2(x, strip.top() + 6.0),
                egui::pos2(x + tab_w, strip.bottom()),
            );
            x += tab_w;
            let resp = ui.allocate_rect(r, egui::Sense::click());
            let active = i == self.active;
            let painter = ui.painter();
            if active {
                let round = CornerRadius {
                    nw: 8,
                    ne: 8,
                    sw: 0,
                    se: 0,
                };
                painter.rect_filled(r, round, p.bg);
                painter.rect_stroke(r, round, border, egui::StrokeKind::Inside);
                // open bottom edge: the active tab flows into the page below
                painter.hline(
                    r.left() + 1.0..=r.right() - 1.0,
                    r.bottom() - 0.5,
                    Stroke::new(1.0, p.bg),
                );
            } else {
                if resp.hovered() {
                    painter.rect_filled(r.shrink2(egui::vec2(3.0, 4.0)), 6.0, p.surface);
                }
                // thin divider between two inactive tabs
                if i + 1 < n && i + 1 != self.active {
                    let y = r.center().y;
                    painter.vline(r.right(), y - 8.0..=y + 8.0, border);
                }
            }

            let title = self.tabs[i]
                .path
                .as_ref()
                .and_then(|t| t.file_stem())
                .map_or("New tab".into(), |s| s.to_string_lossy().into_owned());
            let color = if active { p.text } else { p.muted };
            let galley = truncated(ui, &title, 13.5, color, tab_w - 44.0);
            let pos = egui::pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0);
            ui.painter().galley(pos, galley, color);

            let xr = egui::Rect::from_center_size(
                egui::pos2(r.right() - 18.0, r.center().y),
                egui::Vec2::splat(20.0),
            );
            let xb = ui.allocate_rect(xr, egui::Sense::click());
            if active || resp.hovered() || xb.hovered() {
                if xb.hovered() {
                    ui.painter().rect_filled(xr, 4.0, p.surface);
                }
                ui.painter().text(
                    xr.center(),
                    egui::Align2::CENTER_CENTER,
                    icon::CLOSE,
                    egui::FontId::proportional(13.0),
                    if xb.hovered() { p.text } else { p.muted },
                );
            }
            if xb.on_hover_text("Close tab (Ctrl+W)").clicked() || resp.middle_clicked() {
                close = Some(i);
            } else if resp.clicked() {
                switch = Some(i);
            }
        }

        let plus = egui::Rect::from_center_size(
            egui::pos2(x + 8.0 + ICON_BTN / 2.0, cy + 3.0),
            egui::Vec2::splat(ICON_BTN),
        );
        let new_tab = icon_button_at(ui, plus, icon::ADD, "New tab (Ctrl+T)", false).clicked();

        if let Some(i) = close {
            self.close_tab(i);
        } else if let Some(i) = switch {
            self.switch_tab(i);
        } else if new_tab {
            self.new_tab();
        }
    }

    /// Back/forward, breadcrumb of the open note, bookmark toggle and the "more" menu.
    pub(crate) fn nav_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let tab = &self.tabs[self.active];
        let (can_back, can_fwd) = (!tab.back.is_empty(), !tab.fwd.is_empty());
        let (mut back, mut fwd, mut bookmark, mut delete) = (false, false, false, false);
        let mut read = false;
        let reading = self.reading();
        let mut pick_image = false;
        let mut share = false;
        let mut table: Option<(usize, usize)> = None;

        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_enabled_ui(can_back, |ui| {
                back = icon_button(ui, icon::BACK, "Navigate back (Alt+←)", false).clicked();
            });
            ui.add_enabled_ui(can_fwd, |ui| {
                fwd = icon_button(ui, icon::FORWARD, "Navigate forward (Alt+→)", false).clicked();
            });
            ui.add_space(14.0);

            let Some(cur) = &self.current else { return };
            let rel = cur
                .strip_prefix(&self.vault)
                .unwrap_or(cur)
                .with_extension("");
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            ui.spacing_mut().item_spacing.x = 6.0;
            for (k, part) in parts.iter().enumerate() {
                if k > 0 {
                    ui.label(
                        RichText::new("/")
                            .size(13.5)
                            .color(p.muted.gamma_multiply(0.6)),
                    );
                }
                let last = k + 1 == parts.len();
                ui.label(
                    RichText::new(part)
                        .size(13.5)
                        .color(if last { p.text } else { p.muted }),
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // same 28px icon button as its neighbours, so the row lines up
                let more = icon_button(ui, icon::MORE, "More options", false);
                egui::Popup::menu(&more)
                    .align(egui::RectAlign::BOTTOM_END)
                    .show(|ui| {
                        // inserting edits the note: not in reading mode
                        ui.add_enabled_ui(!reading, |ui| {
                            if ui
                                .button(format!("{}  Insert image", icon::IMAGE))
                                .clicked()
                            {
                                pick_image = true;
                                ui.close();
                            }
                            ui.menu_button(format!("{}  Insert table", icon::TABLE), |ui| {
                                if let Some(rc) = table_picker(ui, p) {
                                    table = Some(rc);
                                    ui.close();
                                }
                            });
                        });
                        ui.separator();
                        if ui.button(format!("{}  Share…", icon::SHARE)).clicked() {
                            share = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(format!("{}  Delete note", icon::TRASH)).clicked() {
                            delete = true;
                            ui.close();
                        }
                    });
                let marked = self.bookmarks.contains(cur);
                let tip = if marked {
                    "Remove bookmark"
                } else {
                    "Bookmark this note"
                };
                bookmark = icon_button(ui, icon::BOOKMARK, tip, marked).clicked();
                let tip = if reading {
                    "Reading mode — click to edit (Ctrl+E)"
                } else {
                    "Switch to reading mode (Ctrl+E)"
                };
                read = icon_button(ui, icon::READ, tip, reading).clicked();
            });
        });

        if back || fwd {
            self.step_history(back);
        }
        if bookmark {
            self.toggle_bookmark();
        }
        if read {
            self.toggle_reading();
        }
        if delete && let Some(cur) = self.current.clone() {
            if self.skip_delete_confirm {
                self.delete_file(&cur);
            } else {
                self.pending_delete = Some(cur);
            }
        }
        let ctx = ui.ctx().clone();
        if let Some((rows, cols)) = table {
            self.insert_at_cursor(&ctx, &table_markdown(rows, cols), true);
        }
        if pick_image {
            self.insert_image(&ctx);
        }
        if share {
            self.open_share();
        }
    }
}

/// Markdown table with `rows` total rows (the first is the header) and `cols` columns.
fn table_markdown(rows: usize, cols: usize) -> String {
    let line = |cell: &dyn Fn(usize) -> String| {
        let cells: Vec<String> = (1..=cols).map(cell).collect();
        format!("| {} |\n", cells.join(" | "))
    };
    let mut s = line(&|c| format!("Column {c}"));
    s += &line(&|_| "---".into());
    for _ in 1..rows.max(2) {
        s += &line(&|_| "   ".into());
    }
    s
}

/// Word-style grid: hover to choose rows × columns, click to confirm.
fn table_picker(ui: &mut egui::Ui, p: &Palette) -> Option<(usize, usize)> {
    const N: usize = 8;
    const CELL: f32 = 20.0;
    const GAP: f32 = 3.0;
    let side = N as f32 * (CELL + GAP) - GAP;
    ui.add_space(2.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let hover = resp.hover_pos().map(|pos| {
        let d = pos - rect.min;
        let at = |v: f32| ((v / (CELL + GAP)) as usize).min(N - 1) + 1;
        (at(d.y), at(d.x))
    });
    let painter = ui.painter();
    for r in 0..N {
        for c in 0..N {
            let min = rect.min + egui::vec2(c as f32, r as f32) * (CELL + GAP);
            let cell = egui::Rect::from_min_size(min, egui::vec2(CELL, CELL));
            let on = hover.is_some_and(|(hr, hc)| r < hr && c < hc);
            if on {
                painter.rect_filled(cell, 2.0, p.link.gamma_multiply(0.55));
            }
            let stroke = Stroke::new(1.0, if on { p.link } else { p.border });
            painter.rect_stroke(cell, 2.0, stroke, egui::StrokeKind::Inside);
        }
    }
    ui.add_space(8.0);
    let label = match hover {
        Some((r, c)) => format!("{r} × {c} table"),
        None => "Insert table".into(),
    };
    ui.label(RichText::new(label).size(13.0).color(p.muted));
    if resp.clicked() { hover } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_markdown_shape() {
        assert_eq!(
            table_markdown(3, 2),
            "| Column 1 | Column 2 |\n| --- | --- |\n|     |     |\n|     |     |\n"
        );
        // a 1-row pick still gets one body row so it renders as a table
        assert_eq!(table_markdown(1, 1).lines().count(), 3);
    }
}
