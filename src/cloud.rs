//! Cloud copy of the vault on the Notefy backend (`backend/`: Cloudflare Worker + Backblaze B2)
//! and share links. All network calls run on one worker thread: the UI sends `Job`s and polls
//! `Event`s. No UI code here.

use crate::{config::config_file, vault::vault_files};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, UNIX_EPOCH},
};

/// Config file: backend URL on line 1, API token on line 2. No file = cloud off.
pub(crate) const CONFIG: &str = "cloud";

/// A share link as the backend lists it.
#[derive(Clone)]
pub(crate) struct ShareInfo {
    pub(crate) id: String,
    /// Shared file relative to the vault (`/`-separated), or "" for the whole vault.
    pub(crate) path: String,
    pub(crate) edit: bool,
    /// Unix seconds when the link stops working (0 = never).
    pub(crate) expires: u64,
    /// Access code ("" = anyone with the link).
    pub(crate) code: String,
    pub(crate) url: String,
}

pub(crate) enum Job {
    /// Upload a vault file; `force` replaces it even if it has unseen web edits.
    Push {
        vault: PathBuf,
        path: PathBuf,
        force: bool,
    },
    Delete {
        vault: PathBuf,
        path: PathBuf,
    },
    /// Check a note for edits made through an editable share link.
    Pull {
        vault: PathBuf,
        path: PathBuf,
    },
    /// Upload the note (`Some`) or every vault file (`None`), then create a link for it.
    Share {
        vault: PathBuf,
        path: Option<PathBuf>,
        edit: bool,
        code: bool,
        /// Seconds until the link expires (0 = never).
        expires_in: u64,
    },
    ListShares {
        vault: PathBuf,
    },
    Unshare {
        vault: PathBuf,
        id: String,
    },
}

pub(crate) enum Event {
    /// `path` was edited in the browser: the web text and the app file time it started from.
    WebEdit {
        path: PathBuf,
        text: String,
        base_mtime: String,
    },
    /// A link was just created.
    Created(ShareInfo),
    /// All links of the vault (after listing, creating or deleting).
    Shares(Vec<ShareInfo>),
    Error(String),
}

/// Handle to the worker thread; dropping it stops the thread.
pub(crate) struct Cloud {
    tx: Sender<Job>,
    rx: Receiver<Event>,
}

impl Cloud {
    /// Start the worker if the config file exists; `wake` is called after each event (repaint).
    pub(crate) fn from_config(wake: impl Fn() + Send + 'static) -> Option<Self> {
        let cfg = fs::read_to_string(config_file(CONFIG)).ok()?;
        let mut lines = cfg.lines().map(str::trim);
        let base = lines.next()?.trim_end_matches('/').to_string();
        let token = lines.next()?.to_string();
        if base.is_empty() || token.is_empty() {
            return None;
        }
        let api = Api {
            base,
            token,
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .timeout_global(Some(Duration::from_secs(60)))
                    .build(),
            ),
        };
        let (tx, jobs) = channel();
        let (out, rx) = channel();
        std::thread::spawn(move || {
            while let Ok(job) = jobs.recv() {
                let events = api.run(job).unwrap_or_else(|e| vec![Event::Error(e)]);
                for ev in events {
                    if out.send(ev).is_err() {
                        return;
                    }
                }
                wake();
            }
        });
        Some(Self { tx, rx })
    }

    pub(crate) fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    pub(crate) fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

struct Api {
    base: String,
    token: String,
    agent: ureq::Agent,
}

type Resp = ureq::http::Response<ureq::Body>;

impl Api {
    fn run(&self, job: Job) -> Result<Vec<Event>, String> {
        Ok(match job {
            Job::Push { vault, path, force } => {
                self.push(&vault, &path, force)?.into_iter().collect()
            }
            Job::Pull { vault, path } => self.pull(&vault, &path)?.into_iter().collect(),
            Job::Delete { vault, path } => {
                let r = self.call(self.agent.delete(self.file_url(&vault, &path)?))?;
                check(&r, "Cloud delete", &path)?;
                vec![]
            }
            Job::Share {
                vault,
                path,
                edit,
                code,
                expires_in,
            } => {
                // shortcut: re-uploads every file on each vault share; compare times first if vaults get big
                let files = path
                    .clone()
                    .map_or_else(|| vault_files(&vault), |p| vec![p]);
                let mut events = vec![];
                for f in &files {
                    events.extend(self.push(&vault, f, false)?);
                }
                let rel = match &path {
                    Some(p) => rel_key(&vault, p).ok_or("Note is outside the vault")?,
                    None => String::new(),
                };
                let body = serde_json::json!({
                    "vault": vault_name(&vault), "path": rel, "edit": edit, "code": code, "expires_in": expires_in,
                });
                let mut r = self.call_body(
                    self.agent
                        .post(format!("{}/shares", self.base))
                        .header("content-type", "application/json"),
                    body.to_string().into_bytes(),
                )?;
                check(&r, "Creating the link", &vault)?;
                let v: Value = serde_json::from_str(&read(&mut r)?).map_err(|e| e.to_string())?;
                events.push(Event::Created(
                    self.share_info(&v).ok_or("Bad reply from cloud")?,
                ));
                events.push(Event::Shares(self.list(&vault)?));
                events
            }
            Job::ListShares { vault } => vec![Event::Shares(self.list(&vault)?)],
            Job::Unshare { vault, id } => {
                let r = self.call(self.agent.delete(format!("{}/shares/{id}", self.base)))?;
                check(&r, "Stopping the share", &vault)?;
                vec![Event::Shares(self.list(&vault)?)]
            }
        })
    }

    /// Upload one file. A 409 means it has web edits we haven't seen: hand them to the app instead.
    fn push(&self, vault: &Path, path: &Path, force: bool) -> Result<Option<Event>, String> {
        let bytes =
            fs::read(path).map_err(|e| format!("Cloud upload of {}: {e}", path.display()))?;
        let is_md = path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("md"));
        let mut req = self
            .agent
            .put(self.file_url(vault, path)?)
            .header("x-mtime", mtime_ms(path))
            .header(
                "content-type",
                if is_md {
                    "text/markdown; charset=utf-8"
                } else {
                    "application/octet-stream"
                },
            );
        if force {
            req = req.header("x-force", "1");
        }
        let r = self.call_body(req, bytes)?;
        if r.status() == 409 {
            return self.pull(vault, path);
        }
        check(&r, "Cloud upload", path)?;
        Ok(None)
    }

    /// The note's web edits, if its cloud copy was last written by a share link.
    fn pull(&self, vault: &Path, path: &Path) -> Result<Option<Event>, String> {
        let mut r = self.call(self.agent.get(self.file_url(vault, path)?))?;
        if r.status() == 404 {
            return Ok(None);
        }
        check(&r, "Cloud download", path)?;
        let header = |r: &Resp, h: &str| {
            r.headers()
                .get(h)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string()
        };
        if header(&r, "x-amz-meta-src") != "web" {
            return Ok(None);
        }
        Ok(Some(Event::WebEdit {
            path: path.to_path_buf(),
            base_mtime: header(&r, "x-amz-meta-mtime"),
            text: read(&mut r)?,
        }))
    }

    fn list(&self, vault: &Path) -> Result<Vec<ShareInfo>, String> {
        let url = format!("{}/shares?vault={}", self.base, encode(&vault_name(vault)));
        let mut r = self.call(self.agent.get(url))?;
        check(&r, "Listing links", vault)?;
        let v: Value = serde_json::from_str(&read(&mut r)?).map_err(|e| e.to_string())?;
        let mut out: Vec<ShareInfo> = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| self.share_info(s))
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    fn share_info(&self, v: &Value) -> Option<ShareInfo> {
        let id = v["id"].as_str()?.to_string();
        Some(ShareInfo {
            url: format!("{}/s/{id}/", self.base),
            path: v["path"].as_str()?.into(),
            edit: v["edit"].as_bool()?,
            code: v["code"].as_str()?.into(),
            expires: v["expires"].as_u64().unwrap_or(0),
            id,
        })
    }

    fn file_url(&self, vault: &Path, path: &Path) -> Result<String, String> {
        let rel = rel_key(vault, path)
            .ok_or_else(|| format!("{} is outside the vault", path.display()))?;
        Ok(format!(
            "{}/files/{}/{}",
            self.base,
            encode(&vault_name(vault)),
            encode(&rel)
        ))
    }

    fn call(
        &self,
        req: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    ) -> Result<Resp, String> {
        req.header("authorization", format!("Bearer {}", self.token))
            .call()
            .map_err(|e| format!("Cloud unreachable: {e}"))
    }

    fn call_body(
        &self,
        req: ureq::RequestBuilder<ureq::typestate::WithBody>,
        body: Vec<u8>,
    ) -> Result<Resp, String> {
        req.header("authorization", format!("Bearer {}", self.token))
            .send(&body[..])
            .map_err(|e| format!("Cloud unreachable: {e}"))
    }
}

fn check(r: &Resp, what: &str, path: &Path) -> Result<(), String> {
    match r.status().as_u16() {
        200..=299 | 404 => Ok(()), // 404 on delete: already gone
        401 => Err("Cloud rejected the API token (check ~/.config/notefy/cloud)".into()),
        s => Err(format!("{what} failed for {} ({s})", path.display())),
    }
}

fn read(r: &mut Resp) -> Result<String, String> {
    r.body_mut()
        .read_to_string()
        .map_err(|e| format!("Cloud read failed: {e}"))
}

/// Folder name of the vault: the first segment of every cloud key.
// shortcut: two vaults with the same folder name share one cloud copy; add a vault id if that matters
fn vault_name(vault: &Path) -> String {
    vault
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// `path` relative to the vault with `/` separators, as stored in the bucket.
pub(crate) fn rel_key(vault: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(vault).ok()?;
    let parts: Vec<_> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Percent-encode for a URL path, keeping `/`.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// File modification time in ms, the version stamp the app uploads with a file.
pub(crate) fn mtime_ms(path: &Path) -> String {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_keep_vault_hierarchy() {
        let v = Path::new("/home/me/My Vault");
        assert_eq!(
            rel_key(v, &v.join("Projects/Big plan.md")).as_deref(),
            Some("Projects/Big plan.md")
        );
        assert_eq!(rel_key(v, v), None);
        assert_eq!(rel_key(v, Path::new("/elsewhere/a.md")), None);
        assert_eq!(
            encode("My Vault/Ünï #1.md"),
            "My%20Vault/%C3%9Cn%C3%AF%20%231.md"
        );
    }
}
