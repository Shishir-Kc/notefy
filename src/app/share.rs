//! "Share" dialog (link for the open note or the whole vault, optional access code and
//! editing), plus applying cloud events: share-link replies and edits made on the web.

use super::*;
use crate::{
    cloud::{self, Event, Job, ShareInfo},
    icon,
};
use eframe::egui::{self, Margin, RichText, Stroke};

/// Choices for a link's lifetime: label and seconds per unit (0 = never).
const EXPIRY_UNITS: [(&str, u64); 5] = [
    ("Never", 0),
    ("Minutes", 60),
    ("Hours", 3600),
    ("Days", 86_400),
    ("Weeks", 604_800),
];

/// State of the open Share dialog.
#[derive(Default)]
pub(crate) struct ShareDialog {
    /// Share the whole vault instead of the open note.
    whole_vault: bool,
    /// Require an access code on top of the link.
    code: bool,
    /// Visitors may edit (read-only otherwise).
    edit: bool,
    /// Index into `EXPIRY_UNITS` (0 = never expires) and how many of that unit.
    expiry_unit: usize,
    expiry_amount: u32,
    /// This vault's links as last listed by the backend (`None` = still loading).
    links: Option<Vec<ShareInfo>>,
    /// A create/delete request is on its way.
    busy: bool,
}

impl Notefy {
    /// Queue a cloud job (no-op when the cloud isn't configured).
    pub(crate) fn cloud(&self, job: Job) {
        if let Some(c) = &self.cloud {
            c.send(job);
        }
    }

    /// Upload `path` (non-forced: the backend refuses if it has unseen web edits).
    pub(crate) fn cloud_push(&self, path: &Path) {
        self.cloud(Job::Push {
            vault: self.vault.clone(),
            path: path.to_path_buf(),
            force: false,
        });
    }

    pub(crate) fn open_share(&mut self) {
        self.share = Some(ShareDialog {
            whole_vault: self.current.is_none(),
            expiry_amount: 1,
            ..Default::default()
        });
        self.cloud(Job::ListShares {
            vault: self.vault.clone(),
        });
    }

    /// Apply finished cloud work; called every frame.
    pub(crate) fn cloud_events(&mut self, ctx: &egui::Context) {
        while let Some(ev) = self.cloud.as_ref().and_then(|c| c.poll()) {
            match ev {
                Event::WebEdit {
                    path,
                    text,
                    base_mtime,
                } => self.take_web_edit(path, text, &base_mtime),
                Event::Created(s) => {
                    ctx.copy_text(s.url.clone());
                    let code = if s.code.is_empty() {
                        String::new()
                    } else {
                        format!(" · access code {}", s.code)
                    };
                    self.flash(format!("Link copied{code}"));
                }
                Event::Shares(links) => {
                    if let Some(d) = &mut self.share {
                        d.links = Some(links);
                        d.busy = false;
                    }
                }
                Event::Error(e) => {
                    if let Some(d) = &mut self.share {
                        d.busy = false;
                        d.links.get_or_insert_default();
                    }
                    self.flash(e);
                }
            }
        }
    }

    /// Bring in a note edited through a share link without losing anything: take the web
    /// text if our copy is unchanged since we uploaded it, else keep ours and save the web
    /// version next to it as "<name> (web edit).md".
    fn take_web_edit(&mut self, path: PathBuf, text: String, base_mtime: &str) {
        let Ok(local) = fs::read_to_string(&path) else {
            return; // deleted here meanwhile; the delete reaches the cloud on its own
        };
        let open_dirty = self.current.as_ref() == Some(&path) && self.dirty;
        let force = |app: &Self, p: &Path| {
            app.cloud(Job::Push {
                vault: app.vault.clone(),
                path: p.to_path_buf(),
                force: true,
            })
        };
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if local == text && !open_dirty {
            force(self, &path); // same text: just mark the cloud copy as seen
        } else if !open_dirty && cloud::mtime_ms(&path) == base_mtime {
            if let Err(e) = fs::write(&path, &text) {
                self.flash(format!("Can't save web edits to {name}: {e}"));
                return;
            }
            if self.current.as_ref() == Some(&path) {
                self.buffer = text;
            }
            force(self, &path);
            self.flash(format!("Got web edits to {name}"));
        } else {
            let mut i = 0;
            let copy = loop {
                let n = if i == 0 {
                    format!("{name} (web edit).md")
                } else {
                    format!("{name} (web edit {i}).md")
                };
                let p = path.with_file_name(n);
                if !p.exists() {
                    break p;
                }
                i += 1;
            };
            if let Err(e) = fs::write(&copy, &text) {
                self.flash(format!("Can't save web edits to {name}: {e}"));
                return;
            }
            self.refresh();
            self.cloud_push(&copy);
            force(self, &path);
            self.flash(format!(
                "{name} was also edited on the web, saved as {}",
                copy.display()
            ));
        }
    }

    /// The Share modal: what to share, who can open it, editing, and the existing links.
    pub(crate) fn share_dialog(&mut self, ctx: &egui::Context) {
        if self.share.is_none() {
            return;
        }
        let p = self.pal();
        let note = self.current.clone();
        let note_name = self.current_title();
        let vault_name = self
            .vault
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let has_cloud = self.cloud.is_some();
        let (mut close, mut create, mut stop) = (false, false, None::<String>);
        let d = self.share.as_mut().expect("checked above");
        let modal = egui::Modal::new(egui::Id::unique("notefy-share-modal"))
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new(format!("{}  Share", icon::SHARE)).size(16.0).strong());
                ui.add_space(10.0);
                if !has_cloud {
                    ui.label("Sharing needs the Notefy cloud backend. Put its URL on the first line and the API token on the second line of:");
                    ui.add_space(4.0);
                    ui.label(RichText::new(cloud_config_path()).monospace().color(p.link));
                    ui.add_space(4.0);
                    ui.label(RichText::new("Then restart Notefy.").color(p.muted));
                } else {
                    let section = |ui: &mut egui::Ui, t: &str| {
                        ui.label(RichText::new(t).size(12.5).color(p.muted));
                    };
                    section(ui, "WHAT");
                    ui.add_enabled_ui(note.is_some(), |ui| {
                        ui.radio_value(&mut d.whole_vault, false, format!("{}  This note — {note_name}", icon::FILE));
                    });
                    ui.radio_value(&mut d.whole_vault, true, format!("{}  Whole vault — {vault_name}", icon::VAULT));
                    ui.add_space(8.0);
                    section(ui, "WHO CAN OPEN IT");
                    ui.radio_value(&mut d.code, false, format!("{}  Anyone with the link", icon::GLOBE));
                    ui.radio_value(&mut d.code, true, format!("{}  Only people with an access code", icon::LOCK));
                    ui.add_space(8.0);
                    section(ui, "PERMISSION");
                    ui.checkbox(&mut d.edit, "Allow editing");
                    ui.label(
                        RichText::new("Off: visitors can only read. Web edits come back into Notefy when you open the note.")
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.add_space(8.0);
                    section(ui, "EXPIRES");
                    ui.horizontal(|ui| {
                        let never = d.expiry_unit == 0;
                        ui.add_enabled(
                            !never,
                            egui::DragValue::new(&mut d.expiry_amount).range(1..=999).speed(0.2),
                        );
                        egui::ComboBox::from_id_salt("share-expiry")
                            .selected_text(if never { "Never" } else { EXPIRY_UNITS[d.expiry_unit].0 })
                            .show_ui(ui, |ui| {
                                for (i, (label, _)) in EXPIRY_UNITS.iter().enumerate() {
                                    ui.selectable_value(&mut d.expiry_unit, i, *label);
                                }
                            });
                        if !never {
                            ui.label(RichText::new("after creating the link").size(12.5).color(p.muted));
                        }
                    });
                    ui.add_space(12.0);
                    // `horizontal` keeps the row one line high: a bare right-to-left layout in a
                    // modal takes all the remaining height and pushes the dialog off-screen
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = if d.busy { "Uploading…" } else { "Create link" };
                            if ui.add_enabled(!d.busy, egui::Button::new(format!("{}  {label}", icon::LINK))).clicked() {
                                create = true;
                            }
                            // what the button will share, so the choice is visible at a glance
                            let what = if d.whole_vault {
                                format!("{}  {vault_name} (whole vault)", icon::VAULT)
                            } else {
                                format!("{}  {note_name}", icon::FILE)
                            };
                            let who = if d.code { "access code" } else { "anyone with the link" };
                            let perm = if d.edit { "can edit" } else { "read only" };
                            let secs = expiry_secs(d.expiry_unit, d.expiry_amount);
                            let ends = if secs == 0 { "no expiry".into() } else { format!("expires in {}", human(secs)) };
                            ui.add(egui::Label::new(RichText::new(format!("{what}  ·  {who}  ·  {perm}  ·  {ends}")).size(12.5).color(p.muted)).truncate());
                        });
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    section(ui, "SHARED LINKS");
                    match &d.links {
                        None => {
                            ui.label(RichText::new("Loading…").color(p.muted));
                        }
                        Some(links) if links.is_empty() => {
                            ui.label(RichText::new("Nothing shared yet.").color(p.muted));
                        }
                        Some(links) => {
                            // modals lay out in last frame's size: size the list from its rows
                            let h = (links.len() as f32 * 56.0).min(220.0);
                            egui::ScrollArea::vertical()
                                .max_height(h)
                                .min_scrolled_height(h)
                                .show(ui, |ui| {
                                for s in links {
                                    share_row(ui, p, s, &mut stop);
                                }
                            });
                        }
                    }
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
            });
        if modal.should_close() {
            close = true; // Esc or click outside
        }

        let vault = self.vault.clone();
        if create {
            d.busy = true;
            let path = if d.whole_vault { None } else { note };
            let job = Job::Share {
                vault,
                path,
                edit: d.edit,
                code: d.code,
                expires_in: expiry_secs(d.expiry_unit, d.expiry_amount),
            };
            self.save(); // upload what's on screen
            self.cloud(job);
        } else if let Some(id) = stop {
            d.busy = true;
            self.cloud(Job::Unshare { vault, id });
        }
        if close {
            self.share = None;
        }
    }
}

/// One existing link: target, access, and Copy / Stop buttons.
fn share_row(ui: &mut egui::Ui, p: &Palette, s: &ShareInfo, stop: &mut Option<String>) {
    ui.horizontal(|ui| {
        let what = if s.path.is_empty() {
            format!("{}  Whole vault", icon::VAULT)
        } else {
            format!("{}  {}", icon::FILE, s.path.trim_end_matches(".md"))
        };
        ui.label(what);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(RichText::new("Stop").color(p.muted))
                .on_hover_text("Turn this link off")
                .clicked()
            {
                *stop = Some(s.id.clone());
            }
            if ui.button(format!("{}  Copy link", icon::COPY)).clicked() {
                ui.ctx().copy_text(s.url.clone());
            }
            if !s.code.is_empty()
                && ui
                    .button(format!("{}  {}", icon::LOCK, s.code))
                    .on_hover_text("Copy access code")
                    .clicked()
            {
                ui.ctx().copy_text(s.code.clone());
            }
        });
    });
    let access = match (s.edit, s.code.is_empty()) {
        (true, true) => "Anyone with the link can edit",
        (false, true) => "Anyone with the link can read",
        (true, false) => "Code required · can edit",
        (false, false) => "Code required · read only",
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let ends = match s.expires {
        0 => String::new(),
        t if t <= now => " · expired".into(),
        t => format!(" · expires in {}", human(t - now)),
    };
    ui.label(
        RichText::new(format!("{access}{ends}"))
            .size(12.0)
            .color(p.muted),
    );
    ui.add_space(4.0);
}

/// Link lifetime in seconds for the dialog's choice (0 = never).
fn expiry_secs(unit: usize, amount: u32) -> u64 {
    EXPIRY_UNITS[unit].1 * u64::from(amount.max(1))
}

/// Rough duration for people: the two largest units, e.g. "2d 3h", "45m", "under a minute".
fn human(secs: u64) -> String {
    let parts: Vec<String> = [(604_800, "w"), (86_400, "d"), (3600, "h"), (60, "m")]
        .iter()
        .scan(secs, |left, &(n, u)| {
            let k = *left / n;
            *left %= n;
            Some((k > 0).then(|| format!("{k}{u}")))
        })
        .flatten()
        .take(2)
        .collect();
    if parts.is_empty() {
        "under a minute".into()
    } else {
        parts.join(" ")
    }
}

fn cloud_config_path() -> String {
    crate::config::config_file(cloud::CONFIG)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_choices_and_labels() {
        assert_eq!(expiry_secs(0, 5), 0); // never
        assert_eq!(expiry_secs(1, 0), 60); // amount is at least 1 (KV minimum is 60s)
        assert_eq!(expiry_secs(4, 2), 2 * 604_800);
        assert_eq!(human(45 * 60), "45m");
        assert_eq!(human(2 * 86_400 + 3 * 3600 + 59), "2d 3h");
        assert_eq!(human(604_800 + 60), "1w 1m");
        assert_eq!(human(30), "under a minute");
    }
}
