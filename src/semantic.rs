//! Meaning-based note search with a local EmbeddingGemma model, run on a background thread
//! so the UI never waits for the model. No UI code here.

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Max characters per embedded piece of a note (paragraphs are packed up to this).
const CHUNK: usize = 1500;
/// How many semantic hits to return.
const TOP: usize = 10;
/// Index file inside the vault (hidden, so the file tree skips it).
const INDEX_FILE: &str = ".notefy/embeddings.bin";
/// First bytes of the index file; change it when the model or format changes so old
/// files are ignored and rebuilt.
const MAGIC: &[u8] = b"notefy-emb v1 embeddinggemma-300m-q4\n";
/// Save the index after this many newly embedded notes, so a quit mid-way keeps progress.
const SAVE_EVERY: usize = 50;

/// Embeddings per note, with the mtime they were computed for.
type Index = HashMap<PathBuf, (SystemTime, Vec<Vec<f32>>)>;
/// A query and its ranked notes (or why semantic search failed).
pub(crate) type Reply = (String, Result<Vec<PathBuf>, String>);

/// Work for the thread: vault, its notes, query.
type Job = (PathBuf, Vec<PathBuf>, String);

/// Handle to the worker thread; dropping it stops the thread.
pub(crate) struct Semantic {
    tx: Sender<Job>,
    rx: Receiver<Reply>,
}

impl Semantic {
    pub(crate) fn new() -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (reply_tx, reply_rx) = channel();
        std::thread::spawn(move || {
            let (mut model, mut index) = (None, Index::new());
            let mut loaded: Option<PathBuf> = None;
            while let Ok(mut job) = job_rx.recv() {
                // only the newest query matters; skip ones typed while we were busy
                while let Ok(newer) = job_rx.try_recv() {
                    job = newer;
                }
                let (vault, notes, query) = job;
                if loaded.as_ref() != Some(&vault) {
                    index = load_index(&vault);
                    loaded = Some(vault.clone());
                }
                let out = run(&mut model, &mut index, &vault, &notes, &query);
                if reply_tx.send((query, out)).is_err() {
                    return;
                }
            }
        });
        Self {
            tx: job_tx,
            rx: reply_rx,
        }
    }

    /// (Re)index `notes` of `vault` and rank them for `query`; an empty query only indexes.
    pub(crate) fn request(&self, vault: PathBuf, notes: Vec<PathBuf>, query: String) {
        let _ = self.tx.send((vault, notes, query));
    }

    /// Next finished answer, if any.
    pub(crate) fn poll(&self) -> Option<Reply> {
        self.rx.try_recv().ok()
    }
}

/// Load the model on first use, embed notes that changed since last time (saving the
/// index into `vault`), rank for `query`.
fn run(
    model: &mut Option<TextEmbedding>,
    index: &mut Index,
    vault: &Path,
    notes: &[PathBuf],
    query: &str,
) -> Result<Vec<PathBuf>, String> {
    let m = match model {
        Some(m) => m,
        None => model.insert(
            TextEmbedding::try_new(
                TextInitOptions::new(EmbeddingModel::EmbeddingGemma300MQ4)
                    .with_cache_dir(cache_dir())
                    .with_show_download_progress(false),
            )
            .map_err(|e| format!("Semantic search unavailable: {e}"))?,
        ),
    };
    let keep: HashSet<&PathBuf> = notes.iter().collect();
    let before = index.len();
    index.retain(|p, _| keep.contains(p));
    let mut changed = (index.len() != before) as usize;
    for p in notes {
        let Ok(mtime) = fs::metadata(p).and_then(|md| md.modified()) else {
            continue;
        };
        if index.get(p).is_some_and(|(t, _)| *t == mtime) {
            continue;
        }
        let text = fs::read_to_string(p).unwrap_or_default();
        let title = p.file_stem().unwrap_or_default().to_string_lossy();
        match m.embed(chunks(&title, &text), None) {
            Ok(vecs) => index.insert(p.clone(), (mtime, vecs)),
            // skip it and keep going; it is retried on the next search
            Err(e) => {
                eprintln!("notefy: indexing {} failed: {e}", p.display());
                continue;
            }
        };
        changed += 1;
        if changed.is_multiple_of(SAVE_EVERY) {
            save_index(vault, index);
        }
    }
    if changed > 0 {
        save_index(vault, index);
    }
    if query.trim().is_empty() {
        return Ok(vec![]);
    }
    let q = m
        .embed(
            [format!("task: search result | query: {}", query.trim())],
            None,
        )
        .map_err(|e| format!("Semantic search failed: {e}"))?;
    Ok(rank(index, &q[0], TOP))
}

/// Where the model is downloaded to (`$XDG_CACHE_HOME/notefy/models`, else `~/.cache/...`).
fn cache_dir() -> PathBuf {
    std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(&std::env::var("HOME").unwrap_or_default()).join(".cache"))
        .join("notefy/models")
}

/// Write `index` to `vault/INDEX_FILE` (paths relative to the vault). Written to a temp
/// file first so a crash never leaves a half-written index.
fn save_index(vault: &Path, index: &Index) {
    let mut b = MAGIC.to_vec();
    for (p, (t, vecs)) in index {
        let rel = p.strip_prefix(vault).unwrap_or(p).to_string_lossy();
        let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
        b.extend((rel.len() as u32).to_le_bytes());
        b.extend(rel.as_bytes());
        b.extend(d.as_secs().to_le_bytes());
        b.extend(d.subsec_nanos().to_le_bytes());
        b.extend((vecs.len() as u32).to_le_bytes());
        b.extend((vecs.first().map_or(0, Vec::len) as u32).to_le_bytes());
        b.extend(vecs.iter().flatten().flat_map(|x| x.to_le_bytes()));
    }
    let file = vault.join(INDEX_FILE);
    let tmp = file.with_extension("tmp");
    let res = fs::create_dir_all(file.parent().unwrap_or(vault))
        .and_then(|_| fs::write(&tmp, b))
        .and_then(|_| fs::rename(&tmp, &file));
    if let Err(e) = res {
        eprintln!("notefy: saving {} failed: {e}", file.display());
    }
}

/// Read the index saved in `vault`; missing, old or corrupt files give an empty index
/// (everything is simply re-embedded).
fn load_index(vault: &Path) -> Index {
    fs::read(vault.join(INDEX_FILE))
        .ok()
        .and_then(|b| parse_index(&b, vault))
        .unwrap_or_default()
}

fn parse_index(b: &[u8], vault: &Path) -> Option<Index> {
    fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
        let (head, rest) = b.split_at_checked(n)?;
        *b = rest;
        Some(head)
    }
    fn u32_(b: &mut &[u8]) -> Option<u32> {
        Some(u32::from_le_bytes(take(b, 4)?.try_into().ok()?))
    }
    let mut b = b.strip_prefix(MAGIC)?;
    let mut index = Index::new();
    while !b.is_empty() {
        let n = u32_(&mut b)? as usize;
        let rel = std::str::from_utf8(take(&mut b, n)?).ok()?;
        let secs = u64::from_le_bytes(take(&mut b, 8)?.try_into().ok()?);
        let mtime = UNIX_EPOCH + Duration::new(secs, u32_(&mut b)?);
        let (count, dim) = (u32_(&mut b)? as usize, u32_(&mut b)? as usize);
        let floats = take(&mut b, count.checked_mul(dim)?.checked_mul(4)?)?;
        let vecs = floats
            .chunks_exact(4 * dim.max(1))
            .map(|v| {
                v.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|x| f32::from_le_bytes(*x))
                    .collect()
            })
            .collect();
        index.insert(vault.join(rel), (mtime, vecs));
    }
    Some(index)
}

/// Split a note into pieces of about `CHUNK` chars on blank lines, each with the document
/// prefix EmbeddingGemma was trained with. An empty note still gets one piece (its title).
pub(crate) fn chunks(title: &str, text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if !cur.is_empty() && cur.len() + para.len() > CHUNK {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|c| format!("title: {title} | text: {c}"))
        .collect()
}

/// The `n` notes whose best-matching piece is closest to `q` (cosine similarity).
fn rank(index: &Index, q: &[f32], n: usize) -> Vec<PathBuf> {
    let cos = |v: &[f32]| {
        let dot: f32 = v.iter().zip(q).map(|(a, b)| a * b).sum();
        let norm = |x: &[f32]| x.iter().map(|a| a * a).sum::<f32>().sqrt();
        dot / (norm(v) * norm(q)).max(f32::EPSILON)
    };
    let mut scored: Vec<(f32, &PathBuf)> = index
        .iter()
        .map(|(p, (_, vs))| (vs.iter().map(|v| cos(v)).fold(f32::MIN, f32::max), p))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(n).map(|(_, p)| p.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking() {
        assert_eq!(chunks("Empty", "  \n\n "), ["title: Empty | text: "]);
        assert_eq!(chunks("A", "one\n\ntwo"), ["title: A | text: one\n\ntwo"]);
        let long = format!("{}\n\n{}", "x".repeat(CHUNK), "y".repeat(10));
        assert_eq!(chunks("B", &long).len(), 2);
    }

    /// Downloads the ~200 MB model on first run: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_model_finds_by_meaning() {
        let v = std::env::temp_dir().join(format!("notefy-sem-{}", std::process::id()));
        let _ = fs::remove_dir_all(&v);
        fs::create_dir_all(&v).unwrap();
        let notes: Vec<PathBuf> = [
            ("groceries", "milk, eggs, bread, apples"),
            (
                "trip",
                "Flight to Tokyo on Friday, hotel near Shinjuku station",
            ),
            ("rust", "Borrow checker errors and lifetimes in structs"),
        ]
        .iter()
        .map(|(n, body)| {
            let p = v.join(format!("{n}.md"));
            fs::write(&p, body).unwrap();
            p
        })
        .collect();
        let (mut model, mut index) = (None, Index::new());
        let hits = run(&mut model, &mut index, &v, &notes, "vacation in Japan").unwrap();
        assert_eq!(hits[0], notes[1]);
        assert_eq!(load_index(&v), index); // saved into the vault
        let hits = run(
            &mut model,
            &mut index,
            &v,
            &notes,
            "what food do I need to buy",
        )
        .unwrap();
        assert_eq!(hits[0], notes[0]);
        fs::remove_dir_all(&v).unwrap();
    }

    #[test]
    fn index_file_roundtrip() {
        let v = std::env::temp_dir().join(format!("notefy-idx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&v);
        let t = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789);
        let index = Index::from([
            (
                v.join("a b/ü.md"),
                (t, vec![vec![0.5, -1.0], vec![2.0, 3.0]]),
            ),
            (v.join("empty.md"), (t, vec![])),
        ]);
        save_index(&v, &index);
        assert_eq!(load_index(&v), index);
        // corrupt or truncated files are ignored, not a panic
        let f = v.join(INDEX_FILE);
        let bytes = fs::read(&f).unwrap();
        fs::write(&f, &bytes[..bytes.len() - 3]).unwrap();
        assert!(load_index(&v).is_empty());
        fs::write(&f, b"junk").unwrap();
        assert!(load_index(&v).is_empty());
        fs::remove_dir_all(&v).unwrap();
    }

    #[test]
    fn ranking_uses_best_chunk() {
        let t = SystemTime::UNIX_EPOCH;
        let index = Index::from([
            ("a".into(), (t, vec![vec![0.0, 1.0]])),
            ("b".into(), (t, vec![vec![0.0, 1.0], vec![1.0, 0.1]])),
            ("c".into(), (t, vec![vec![-1.0, 0.0]])),
        ]);
        assert_eq!(
            rank(&index, &[1.0, 0.0], 2),
            [PathBuf::from("b"), PathBuf::from("a")]
        );
    }
}
