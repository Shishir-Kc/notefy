//! The live-preview editor: text input, `[[` link suggestions, link clicks,
//! Ctrl+click previews and inserting text/images/tables at the cursor.

use super::*;
use crate::{icon, live, vault::*};
use eframe::egui::{self, Color32, FontId, Key, Margin, RichText, Stroke};
use std::{fs, path::Path, time::Instant};

impl Notefy {
    pub(crate) fn live_theme(&self, ctx: &egui::Context) -> live::Theme {
        let p = self.pal();
        live::Theme {
            text: p.text,
            muted: p.muted,
            link: p.link,
            code_bg: p.surface,
            mark_bg: p.link.gamma_multiply(0.25),
            size: 16.0,
            char_w: ctx.fonts_mut(|f| f.glyph_width(&FontId::proportional(16.0), '0')),
        }
    }

    /// Single live-preview editor: markdown is styled in place as you type.
    pub(crate) fn editor(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let id = editor_id();
        let ctx = ui.ctx().clone();

        // Lines touched by the cursor/selection show their raw markdown markers
        // (never in reading mode, which always shows the rendered note).
        let reading = self.reading();
        let focused = ctx.memory(|m| m.has_focus(id)) && !reading;
        let reveal = egui::text_edit::TextEditState::load(&ctx, id)
            .and_then(|st| st.cursor.char_range())
            .filter(|_| focused)
            .map(|r| {
                let line_of =
                    |c: usize| self.buffer.chars().take(c).filter(|&ch| ch == '\n').count();
                let (a, b) = (
                    line_of(r.primary.index.into()),
                    line_of(r.secondary.index.into()),
                );
                a.min(b)..=a.max(b)
            });

        // cursor/selection as a byte range: links touching it show their raw markdown
        let sel_bytes = egui::text_edit::TextEditState::load(&ctx, id)
            .and_then(|st| st.cursor.char_range())
            .filter(|_| focused)
            .map(|r| {
                let (a, b) = (r.primary.index.0, r.secondary.index.0);
                char_to_byte(&self.buffer, a.min(b))..char_to_byte(&self.buffer, a.max(b))
            });

        let theme = self.live_theme(&ctx);

        // `[[query` right before the cursor -> suggestion popup. Esc already dropped
        // focus this pass (egui does that first), hence "had focus last frame" too.
        let had_focus = focused || (!reading && ctx.memory(|m| m.had_focus_last_frame(id)));
        let raw_query = egui::text_edit::TextEditState::load(&ctx, id)
            .and_then(|st| st.cursor.char_range())
            .filter(|r| had_focus && r.primary.index == r.secondary.index)
            .and_then(|r| {
                let at = char_to_byte(&self.buffer, r.primary.index.0);
                live::link_query(&self.buffer, at).map(|(start, q)| (start, at, q.to_string()))
            });
        if raw_query.is_none() {
            self.link.dismissed = None;
        }
        let query = raw_query.filter(|(start, ..)| self.link.dismissed != Some(*start));
        if let Some((_, _, q)) = &query
            && self.link.last_query.as_deref() != Some(q)
        {
            self.link.results = if q.trim().is_empty() {
                self.notes.iter().take(50).map(|n| n.path.clone()).collect()
            } else {
                search_notes(&self.notes, q)
            };
            self.link.last_query = Some(q.clone());
            self.link.sel = 0;
        }
        let popup = query.is_some() && !self.link.results.is_empty();
        let (mut up, mut down, mut pick, mut esc) = (false, false, false, false);
        if popup {
            let n = self.link.results.len();
            (up, down, pick, esc) = ctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, Key::Enter)
                        || i.consume_key(egui::Modifiers::NONE, Key::Tab),
                    i.consume_key(egui::Modifiers::NONE, Key::Escape),
                )
            });
            if down {
                self.link.sel = (self.link.sel + 1) % n;
            }
            if up {
                self.link.sel = (self.link.sel + n - 1) % n;
            }
            if esc {
                self.link.dismissed = query.as_ref().map(|q| q.0);
                ctx.memory_mut(|m| m.request_focus(id));
            }
        }
        let popup = popup && !esc;
        let mut chosen = if pick {
            self.link.results.get(self.link.sel).cloned()
        } else {
            None
        };
        let mut preview = None;

        let tables = live::tables(&self.buffer, theme.char_w);
        // Enter inside a cell would split the row and break the table
        if reveal.as_ref().is_some_and(|r| {
            tables
                .iter()
                .any(|t| (t.first..=t.last).contains(r.start()))
        }) {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        }
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            let job = live::layout(
                text.as_str(),
                reveal.clone(),
                sel_bytes.clone(),
                &theme,
                wrap,
            );
            ui.fonts_mut(|f| f.layout_job(job))
        };

        // after "New note": focus the inline title with its text selected
        if std::mem::take(&mut self.focus_title) {
            let mut st = egui::text_edit::TextEditState::load(&ctx, title_id()).unwrap_or_default();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(self.title_edit.chars().count()),
            )));
            st.store(&ctx, title_id());
            ctx.memory_mut(|m| m.request_focus(title_id()));
        }
        let mut title_resp = None;

        egui::ScrollArea::vertical()
            .id_salt("editor")
            .auto_shrink(false)
            .show(ui, |ui| {
                // Obsidian-style readable line length, centered
                let avail = ui.available_width();
                let pad = ((avail - MAX_TEXT_W) / 2.0).max(32.0);
                let width = (avail - 2.0 * pad).max(120.0);
                ui.add_space(36.0);
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    title_resp = Some(
                        ui.add(
                            egui::TextEdit::singleline(&mut self.title_edit)
                                .id(title_id())
                                .frame(egui::Frame::NONE)
                                .margin(Margin::ZERO)
                                .font(FontId::new(28.0, live::bold_family()))
                                .text_color(p.text)
                                .hint_text("Untitled")
                                .interactive(!reading)
                                .desired_width(width),
                        ),
                    );
                });
                ui.add_space(22.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(pad);
                    // reading mode: a `&str` is a read-only buffer (select/click, no typing)
                    let mut view: &str;
                    let text: &mut dyn egui::TextBuffer = if reading {
                        view = self.buffer.as_str();
                        &mut view
                    } else {
                        &mut self.buffer
                    };
                    let out = egui::TextEdit::multiline(text)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .desired_width(width)
                        .desired_rows(30)
                        .lock_focus(true)
                        .layouter(&mut layouter)
                        .show(ui);
                    if !tables.is_empty() {
                        draw_table_grids(ui, &out, &tables, p.border);
                    }
                    let r = &out.response.response;
                    if r.changed() {
                        self.dirty = true;
                        self.last_edit = Instant::now();
                    }
                    // pointing hand over links a click would follow
                    if let Some(pos) = r.hover_pos() {
                        let c = out.galley.cursor_from_pos(pos - out.galley_pos);
                        let byte = char_to_byte(&self.buffer, c.index.0);
                        if let Some((_, span)) = live::link_at(&self.buffer, byte)
                            && !sel_bytes
                                .as_ref()
                                .is_some_and(|s| s.start <= span.end && s.end >= span.start)
                        {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    // Links: click opens a [[note]] in a new tab, Ctrl+click previews it
                    // (or opens a [url] in the browser). A click on a link the cursor is
                    // already in (shown expanded for editing) just moves the cursor.
                    if r.clicked()
                        && let Some(c) = out.cursor_range
                    {
                        let ctrl = ui.input(|i| i.modifiers.command);
                        let byte = char_to_byte(&self.buffer, c.primary.index.0);
                        if let Some((hit, span)) = live::link_at(&self.buffer, byte) {
                            let editing = sel_bytes
                                .as_ref()
                                .is_some_and(|s| s.start <= span.end && s.end >= span.start);
                            match hit {
                                live::LinkHit::Wiki(t) if ctrl => preview = Some(t),
                                live::LinkHit::Wiki(t) if !editing => self.pending_link = Some(t),
                                live::LinkHit::Url(u) if ctrl => {
                                    ui.ctx().open_url(egui::OpenUrl::new_tab(u))
                                }
                                _ => {}
                            }
                        }
                    }

                    if popup && let Some(c) = out.cursor_range {
                        let pos = out.galley_pos
                            + out
                                .galley
                                .pos_from_cursor(c.primary)
                                .left_bottom()
                                .to_vec2()
                            + egui::vec2(0.0, 4.0);
                        egui::Area::new(egui::Id::unique("notefy-link-popup"))
                            .fixed_pos(pos)
                            .order(egui::Order::Foreground)
                            .constrain(true)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::new()
                                    .fill(p.panel)
                                    .stroke(Stroke::new(1.0, p.border))
                                    .corner_radius(6)
                                    .inner_margin(Margin::same(6))
                                    .show(ui, |ui| {
                                        ui.set_width(300.0);
                                        // explicit height (see search_popup): grows with the results
                                        let list_h =
                                            (self.link.results.len() as f32 * 32.0).min(260.0);
                                        egui::ScrollArea::vertical()
                                            .max_height(list_h)
                                            .min_scrolled_height(list_h)
                                            .show(ui, |ui| {
                                                for (i, path) in
                                                    self.link.results.iter().enumerate()
                                                {
                                                    let name = path
                                                        .file_stem()
                                                        .unwrap_or_default()
                                                        .to_string_lossy();
                                                    let folder = path
                                                        .parent()
                                                        .and_then(|d| {
                                                            d.strip_prefix(&self.vault).ok()
                                                        })
                                                        .map(|d| d.to_string_lossy().into_owned())
                                                        .unwrap_or_default();
                                                    let active = i == self.link.sel;
                                                    let r = ui.add_sized(
                                                        [ui.available_width(), 26.0],
                                                        egui::Button::selectable(
                                                            active,
                                                            RichText::new(format!(
                                                                "{}  {name}",
                                                                icon::FILE
                                                            ))
                                                            .color(if active {
                                                                p.link
                                                            } else {
                                                                p.text
                                                            }),
                                                        ),
                                                    );
                                                    // folder hint tells same-named notes apart
                                                    if !folder.is_empty() {
                                                        ui.painter().text(
                                                            r.rect.right_center()
                                                                - egui::vec2(8.0, 0.0),
                                                            egui::Align2::RIGHT_CENTER,
                                                            &folder,
                                                            FontId::proportional(11.5),
                                                            p.muted,
                                                        );
                                                    }
                                                    if active && (up || down) {
                                                        r.scroll_to_me(None);
                                                    }
                                                    if r.clicked() {
                                                        chosen = Some(path.clone());
                                                    }
                                                }
                                            });
                                    });
                            });
                    }
                });
            });
        // inline title: Enter/click away renames the file, Esc reverts
        if let Some(r) = title_resp
            && r.lost_focus()
        {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.title_edit = self.current_title();
            } else {
                self.commit_title();
            }
            if ctx.input(|i| i.key_pressed(Key::Enter)) {
                ctx.memory_mut(|m| m.request_focus(id));
            }
        }
        if let (Some(path), Some((start, end, _))) = (chosen, &query) {
            self.complete_link(&ctx, *start, *end, &path);
        }
        if let Some(t) = preview {
            self.show_preview(&ctx, &t);
        }
        if let Some(t) = self.pending_link.take() {
            self.open_wikilink(&t);
        }
    }

    /// Page shown in an empty tab: quick actions, like Obsidian's "New tab".
    pub(crate) fn empty_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let mut action = None;
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.32).max(40.0));
            ui.spacing_mut().item_spacing.y = 10.0;
            for (i, (label, keys)) in [
                ("Create new note", "Ctrl+N"),
                ("Go to file", "Ctrl+Space"),
                ("Close", "Ctrl+W"),
            ]
            .into_iter()
            .enumerate()
            {
                let text = RichText::new(format!("{label}  ({keys})"))
                    .size(15.0)
                    .color(p.link);
                if ui.add(egui::Button::new(text).frame(false)).clicked() {
                    action = Some(i);
                }
            }
        });
        match action {
            Some(0) => self.new_note(),
            Some(1) => self.toggle_search(),
            Some(2) => self.close_tab(self.active),
            _ => {}
        }
    }

    /// Replace the typed `[[query` (bytes `start..end`) with a link to `path`.
    pub(crate) fn complete_link(
        &mut self,
        ctx: &egui::Context,
        start: usize,
        mut end: usize,
        path: &Path,
    ) {
        if self.buffer[end..].starts_with("]]") {
            end += 2;
        }
        let link = wikilink(&self.notes, path);
        let (a, b) = (
            self.buffer[..start].chars().count(),
            self.buffer[..end].chars().count(),
        );
        let mut st = egui::text_edit::TextEditState::load(ctx, editor_id()).unwrap_or_default();
        st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(a),
            egui::text::CCursor::new(b),
        )));
        st.store(ctx, editor_id());
        self.insert_at_cursor(ctx, &link, false);
        self.link.last_query = None;
    }

    /// Insert `text` at the editor cursor (replacing any selection), then refocus the editor.
    /// `block` puts it on its own lines, separated by a blank line (needed for tables).
    pub(crate) fn insert_at_cursor(&mut self, ctx: &egui::Context, text: &str, block: bool) {
        let mut st = egui::text_edit::TextEditState::load(ctx, editor_id()).unwrap_or_default();
        let total = self.buffer.chars().count();
        let (a, b) = st
            .cursor
            .char_range()
            .map(|r| {
                let (x, y) = (r.primary.index.0, r.secondary.index.0);
                (x.min(y).min(total), x.max(y).min(total))
            })
            .unwrap_or((total, total));
        let byte = |c: usize| {
            self.buffer
                .char_indices()
                .nth(c)
                .map_or(self.buffer.len(), |(i, _)| i)
        };
        let (ba, bb) = (byte(a), byte(b));
        let mut ins = String::new();
        if block {
            let before = &self.buffer[..ba];
            if !before.is_empty() && !before.ends_with("\n\n") {
                ins.push_str(if before.ends_with('\n') { "\n" } else { "\n\n" });
            }
        }
        ins.push_str(text);
        if block && !self.buffer[bb..].starts_with('\n') {
            ins.push('\n');
        }
        self.buffer.replace_range(ba..bb, &ins);
        let cur = egui::text::CCursor::new(a + ins.chars().count());
        st.cursor
            .set_char_range(Some(egui::text::CCursorRange::one(cur)));
        st.store(ctx, editor_id());
        ctx.memory_mut(|m| m.request_focus(editor_id()));
        self.dirty = true;
        self.last_edit = Instant::now();
    }

    /// Pick an image file and insert a Markdown link to it. Images outside the
    /// vault are copied into `<vault>/attachments/` first.
    pub(crate) fn insert_image(&mut self, ctx: &egui::Context) {
        let Some(note) = self.current.clone() else {
            return;
        };
        let Some(src) = rfd::FileDialog::new()
            .set_title("Insert image")
            .add_filter(
                "Images",
                &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"],
            )
            .pick_file()
        else {
            return;
        };
        let target = if src.starts_with(&self.vault) {
            src.clone()
        } else {
            let dir = self.vault.join("attachments");
            let stem = src
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let ext = src
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut dest = dir.join(src.file_name().unwrap_or_default());
            let mut i = 1;
            while dest.exists() {
                dest = dir.join(format!("{stem} {i}.{ext}"));
                i += 1;
            }
            if let Err(e) = fs::create_dir_all(&dir).and_then(|_| fs::copy(&src, &dest)) {
                self.status = format!("Image copy failed: {e}");
                return;
            }
            dest
        };
        self.cloud_push(&target);
        let rel = relative_path(note.parent().unwrap_or(&self.vault), &target);
        let rel = rel.to_string_lossy().replace('\\', "/");
        let alt = target
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let link = if rel.contains(' ') {
            format!("<{rel}>")
        } else {
            rel
        };
        self.insert_at_cursor(ctx, &format!("![{alt}]({link})"), false);
    }

    pub(crate) fn show_preview(&mut self, ctx: &egui::Context, target: &str) {
        let path = self.find_note(target);
        let body = match &path {
            Some(p) => fs::read_to_string(p).unwrap_or_default(),
            None => "_This note doesn't exist yet. Click the link to create it._".into(),
        };
        let title = path
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| target.to_string());
        let pos = ctx.pointer_latest_pos().unwrap_or_default();
        self.preview = Some(Preview {
            title,
            body,
            pos,
            opened: ctx.cumulative_pass_nr(),
        });
    }

    pub(crate) fn preview_popup(&mut self, ctx: &egui::Context) {
        let Some(pv) = &self.preview else { return };
        let p = self.pal();
        let mut theme = self.live_theme(ctx);
        theme.size = 14.0;
        // open below the link, or above it when there's more room there
        let screen = ctx.content_rect();
        let (below, above) = (screen.bottom() - pv.pos.y, pv.pos.y - screen.top());
        let (pivot, pos, room) = if below >= PREVIEW_H + 90.0 || below >= above {
            (
                egui::Align2::LEFT_TOP,
                pv.pos + egui::vec2(8.0, 14.0),
                below,
            )
        } else {
            (
                egui::Align2::LEFT_BOTTOM,
                pv.pos + egui::vec2(8.0, -10.0),
                above,
            )
        };
        // title, separator and margins take ~90px; the rest is the scrollable body
        let body_h = PREVIEW_H.min(room - 90.0).max(120.0);
        let area = egui::Area::new(egui::Id::unique("notefy-preview"))
            .pivot(pivot)
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(6)
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(PREVIEW_W);
                        ui.label(RichText::new(&pv.title).size(15.0).strong());
                        ui.separator();
                        // lay out first so the popup is sized to this note, not the last one
                        let job = live::layout(&pv.body, None, None, &theme, PREVIEW_W);
                        let galley = ui.fonts_mut(|f| f.layout_job(job));
                        let h = galley.size().y.min(body_h);
                        egui::ScrollArea::vertical()
                            .max_height(h)
                            .min_scrolled_height(h)
                            .show(ui, |ui| {
                                ui.label(galley);
                            });
                    });
            });
        let fresh = pv.opened == ctx.cumulative_pass_nr();
        if !fresh
            && (area.response.clicked_elsewhere() || ctx.input(|i| i.key_pressed(Key::Escape)))
        {
            self.preview = None;
        }
    }
}

/// Ctrl+click preview popup size (body height before scrolling).
const PREVIEW_W: f32 = 520.0;
const PREVIEW_H: f32 = 520.0;

/// Grid lines over tables that `live::layout` rendered as aligned cells.
fn draw_table_grids(
    ui: &egui::Ui,
    out: &egui::text_edit::TextEditOutput,
    tables: &[live::Table],
    color: Color32,
) {
    // screen rect of every source line (a wrapped line spans several galley rows)
    let mut lines: Vec<egui::Rect> = vec![];
    let mut cur: Option<egui::Rect> = None;
    for row in &out.galley.rows {
        let r = row.rect().translate(out.galley_pos.to_vec2());
        cur = Some(cur.map_or(r, |c| c.union(r)));
        if row.ends_with_newline {
            lines.extend(cur.take());
        }
    }
    lines.extend(cur);
    let stroke = Stroke::new(1.0, color);
    let painter = ui.painter();
    for t in tables {
        let (Some(top), Some(bottom)) = (lines.get(t.first), lines.get(t.last)) else {
            continue;
        };
        let (top, bottom, left) = (top.top(), bottom.bottom(), out.galley_pos.x);
        for &x in &t.cols {
            painter.vline(left + x, top..=bottom, stroke);
        }
        let span = left..=left + t.cols.last().copied().unwrap_or(0.0);
        painter.hline(span.clone(), top, stroke);
        for l in (t.first..=t.last).filter(|&l| l != t.first) {
            if let Some(r) = lines.get(l) {
                painter.hline(span.clone(), r.bottom(), stroke);
            }
        }
    }
}

pub(crate) fn char_to_byte(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map_or(s.len(), |(b, _)| b)
}

/// Widest the text column gets before it is centred with side margins.
const MAX_TEXT_W: f32 = 720.0;

fn title_id() -> egui::Id {
    egui::Id::unique("notefy-title")
}

pub(crate) fn editor_id() -> egui::Id {
    egui::Id::unique("notefy-editor")
}
