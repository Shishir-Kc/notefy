//! Obsidian-style "live preview": the editor shows the raw Markdown source, but
//! styled in place (big headings, bold, italic, code, links…). Syntax markers
//! (`#`, `**`, `[[`, …) are hidden everywhere except on the line(s) the cursor
//! is on, so you can still edit them.

use eframe::egui::{
    Color32, FontFamily, FontId, Stroke,
    text::{LayoutJob, TextFormat},
};
use std::ops::{Range, RangeInclusive};

pub struct Theme {
    pub text: Color32,
    pub muted: Color32,
    pub link: Color32,
    pub code_bg: Color32,
    pub mark_bg: Color32,
    pub size: f32,
    /// Advance of one character (the font is monospace); used to align table columns.
    pub char_w: f32,
}

/// Horizontal padding inside a table cell.
const PAD: f32 = 10.0;

/// A table shown as a grid (cursor is not in it): source line range and column edges
/// (x offsets from the line start, one more than the column count).
pub struct Table {
    pub first: usize,
    pub last: usize,
    pub sep: usize,
    pub cols: Vec<f32>,
}

// shortcut: escaped `\|` inside cells is treated as a column break; handle it if anyone needs it.
fn cells(line: &str) -> Vec<&str> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').collect()
}

fn is_sep(line: &str) -> bool {
    line.starts_with('|')
        && cells(line)
            .iter()
            .all(|c| c.contains('-') && c.trim().chars().all(|ch| ch == '-' || ch == ':'))
}

/// Tables in `src`; always rendered as an editable grid.
pub fn tables(src: &str, char_w: f32) -> Vec<Table> {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut out = vec![];
    let mut i = 0;
    while i + 1 < lines.len() {
        if !(lines[i].starts_with('|') && is_sep(lines[i + 1])) {
            i += 1;
            continue;
        }
        let mut last = i + 1;
        while last + 1 < lines.len() && lines[last + 1].starts_with('|') {
            last += 1;
        }
        {
            let n = cells(lines[i]).len();
            let mut w = vec![0.0f32; n];
            for (k, l) in lines[i..=last].iter().enumerate() {
                if k == 1 {
                    continue;
                }
                for (c, cell) in cells(l).iter().take(n).enumerate() {
                    w[c] = w[c].max(cell.chars().count() as f32 * char_w);
                }
            }
            let mut cols = vec![0.0];
            for x in w {
                cols.push(cols.last().unwrap() + x + 2.0 * PAD);
            }
            out.push(Table {
                first: i,
                last,
                sep: i + 1,
                cols,
            });
        }
        i = last + 1;
    }
    out
}

pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}
pub fn italic_family() -> FontFamily {
    FontFamily::Name("italic".into())
}

/// Build the styled layout for `src`. Lines in `reveal` show their markdown markers;
/// links only show theirs when the cursor/selection (`cursor`, byte range) touches them.
pub fn layout(
    src: &str,
    reveal: Option<RangeInclusive<usize>>,
    cursor: Option<Range<usize>>,
    th: &Theme,
    wrap: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let base = TextFormat {
        font_id: FontId::proportional(th.size),
        color: th.text,
        line_height: Some(th.size * 1.6),
        ..Default::default()
    };
    let mut in_code = false;
    let tables = tables(src, th.char_w);
    let hidden = TextFormat {
        font_id: FontId::proportional(0.5),
        color: Color32::TRANSPARENT,
        ..base.clone()
    };
    let hidden_w = th.char_w * 0.5 / th.size;

    for (idx, raw) in src.split_inclusive('\n').enumerate() {
        let (line, nl) = match raw.strip_suffix('\n') {
            Some(l) => (l, "\n"),
            None => (raw, ""),
        };
        // table shown as a grid: hide pipes/whitespace, pad cells to the column edges
        if let Some(t) = tables.iter().find(|t| (t.first..=t.last).contains(&idx)) {
            if idx == t.sep {
                let thin = TextFormat {
                    line_height: Some(6.0),
                    ..hidden.clone()
                };
                job.append(raw, 0.0, thin);
                continue;
            }
            let f = if idx == t.first {
                TextFormat {
                    font_id: FontId::new(th.size, bold_family()),
                    ..base.clone()
                }
            } else {
                base.clone()
            };
            let n = t.cols.len() - 1;
            let mut x = 0.0;
            for (k, seg) in line.split('|').enumerate() {
                if k > 0 {
                    job.append("|", 0.0, hidden.clone());
                    x += hidden_w;
                }
                if seg.is_empty() {
                    continue;
                }
                if k == 0 || k > n {
                    job.append(seg, 0.0, hidden.clone());
                    x += seg.chars().count() as f32 * hidden_w;
                    continue;
                }
                // whole cell incl. its spaces at normal size: real click targets, full-height cursor
                let space = (t.cols[k - 1] + PAD - x).max(0.0);
                job.append(seg, space, f.clone());
                x += space + seg.chars().count() as f32 * th.char_w;
            }
            if !nl.is_empty() {
                job.append(nl, 0.0, base.clone());
            }
            continue;
        }

        let mut w = Writer {
            job: &mut job,
            th,
            show: reveal.as_ref().is_some_and(|r| r.contains(&idx)),
            src,
            cursor: cursor.clone(),
        };

        // fenced code blocks
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            let f = TextFormat {
                font_id: FontId::monospace(th.size * 0.85),
                color: th.muted,
                background: th.code_bg,
                ..base.clone()
            };
            w.push(line, f.clone());
            w.push(nl, f);
            continue;
        }
        if in_code {
            let f = TextFormat {
                font_id: FontId::monospace(th.size * 0.95),
                background: th.code_bg,
                ..base.clone()
            };
            w.push(line, f.clone());
            w.push(nl, f);
            continue;
        }

        let indent_len = line.len() - line.trim_start().len();
        let (indent, body) = line.split_at(indent_len);
        w.push(indent, base.clone());

        // headings
        let hashes = body.bytes().take_while(|&b| b == b'#').count();
        if (1..=6).contains(&hashes) && body[hashes..].starts_with(' ') {
            let scale = [1.85, 1.55, 1.3, 1.15, 1.0, 0.95][hashes - 1];
            let size = th.size * scale;
            let f = TextFormat {
                font_id: FontId::new(size, bold_family()),
                color: if hashes == 6 { th.muted } else { th.text },
                line_height: Some(size * 1.5),
                ..base.clone()
            };
            w.marker(&body[..=hashes], &f);
            w.inline(&body[hashes + 1..], &f);
            w.push(nl, f);
            continue;
        }

        // block quote
        if let Some(rest) = body.strip_prefix("> ").or_else(|| body.strip_prefix('>')) {
            let f = TextFormat {
                font_id: FontId::new(th.size, italic_family()),
                color: th.muted,
                ..base.clone()
            };
            let marker_len = body.len() - rest.len();
            let bar = TextFormat {
                color: th.link,
                ..base.clone()
            };
            if w.show {
                w.push(&body[..marker_len], bar);
            } else {
                // keep the '>' visible as a quote bar
                w.push(
                    &body[..1],
                    TextFormat {
                        font_id: FontId::new(th.size, bold_family()),
                        ..bar
                    },
                );
                w.push(&body[1..marker_len], base.clone());
            }
            w.inline(rest, &f);
            w.push(nl, base.clone());
            continue;
        }

        // horizontal rule
        let t = body.trim_end();
        if t.len() >= 3 && (t.bytes().all(|b| b == b'-') || t.bytes().all(|b| b == b'*')) {
            let f = TextFormat {
                color: th.muted,
                strikethrough: Stroke::new(1.0, th.muted),
                ..base.clone()
            };
            w.push(body, f);
            w.push(nl, base.clone());
            continue;
        }

        // task lists
        let bullet = TextFormat {
            color: th.link,
            font_id: FontId::new(th.size, bold_family()),
            ..base.clone()
        };
        for (pre, done) in [("- [ ] ", false), ("- [x] ", true), ("- [X] ", true)] {
            if let Some(rest) = body.strip_prefix(pre) {
                w.push(pre, bullet.clone());
                let f = if done {
                    TextFormat {
                        color: th.muted,
                        strikethrough: Stroke::new(1.0, th.muted),
                        ..base.clone()
                    }
                } else {
                    base.clone()
                };
                w.inline(rest, &f);
                w.push(nl, base.clone());
                break;
            }
        }
        if ["- [ ] ", "- [x] ", "- [X] "]
            .iter()
            .any(|p| body.starts_with(p))
        {
            continue;
        }

        // bullet / numbered lists
        let list_marker_len = if ["- ", "* ", "+ "].iter().any(|p| body.starts_with(p)) {
            2
        } else {
            let digits = body.bytes().take_while(|b| b.is_ascii_digit()).count();
            if digits > 0 && body[digits..].starts_with(". ") {
                digits + 2
            } else {
                0
            }
        };
        if list_marker_len > 0 {
            let marker = &body[..list_marker_len];
            if marker.starts_with(['-', '*', '+']) && !w.show {
                // render the bullet char as a dot-ish marker
                w.push(&marker[..1], bullet.clone());
                w.push(&marker[1..], base.clone());
            } else {
                w.push(marker, bullet.clone());
            }
            w.inline(&body[list_marker_len..], &base);
            w.push(nl, base.clone());
            continue;
        }

        w.inline(body, &base);
        w.push(nl, base.clone());
    }
    job
}

struct Writer<'a> {
    job: &'a mut LayoutJob,
    th: &'a Theme,
    /// Show markdown markers on this line?
    show: bool,
    /// Whole source text and the cursor/selection in it (bytes), for per-link reveal.
    src: &'a str,
    cursor: Option<Range<usize>>,
}

impl Writer<'_> {
    /// Does the cursor touch the `len` bytes starting at `sub` (a slice of `self.src`)?
    fn cursor_near(&self, sub: &str, len: usize) -> bool {
        // every slice we format is borrowed from `src`, so its address gives its offset
        let start = sub.as_ptr() as usize - self.src.as_ptr() as usize;
        self.cursor
            .as_ref()
            .is_some_and(|c| c.start <= start + len && c.end >= start)
    }

    fn push(&mut self, s: &str, f: TextFormat) {
        if !s.is_empty() {
            self.job.append(s, 0.0, f);
        }
    }

    /// Syntax characters: dimmed on the active line, (almost) invisible elsewhere.
    fn marker(&mut self, s: &str, f: &TextFormat) {
        let f = if self.show {
            TextFormat {
                color: self.th.muted,
                ..f.clone()
            }
        } else {
            TextFormat {
                font_id: FontId::proportional(0.5),
                color: Color32::TRANSPARENT,
                background: Color32::TRANSPARENT,
                ..f.clone()
            }
        };
        self.push(s, f);
    }

    /// Inline markdown: **bold**, *italic*, `code`, ~~strike~~, ==mark==, [[wiki]], [text](url).
    fn inline(&mut self, s: &str, f: &TextFormat) {
        let th = self.th;
        let mut i = 0;
        let mut plain = 0; // start of pending plain text
        while i < s.len() {
            let rest = &s[i..];
            let mut handled = None; // bytes consumed

            if rest.starts_with('`') {
                if let Some(end) = rest[1..].find('`') {
                    self.push(&s[plain..i], f.clone());
                    let code = TextFormat {
                        font_id: FontId::monospace(f.font_id.size * 0.92),
                        background: th.code_bg,
                        ..f.clone()
                    };
                    self.marker("`", &code);
                    self.push(&rest[1..=end], code.clone());
                    self.marker("`", &code);
                    handled = Some(end + 2);
                }
            } else if rest.starts_with("[[") {
                if let Some(end) = rest[2..].find("]]") {
                    self.push(&s[plain..i], f.clone());
                    let inner = &rest[2..2 + end];
                    let link = self.link_fmt(f);
                    // raw `[[target|label]]` only while the cursor is at the link
                    let line_show = {
                        let near = self.cursor_near(rest, end + 4);
                        std::mem::replace(&mut self.show, near)
                    };
                    self.marker("[[", f);
                    match inner.split_once('|') {
                        Some((target, label)) => {
                            self.marker(target, f);
                            self.marker("|", f);
                            self.push(label, link);
                        }
                        None => self.push(inner, link),
                    }
                    self.marker("]]", f);
                    self.show = line_show;
                    handled = Some(end + 4);
                }
            } else if rest.starts_with('[') {
                if let Some(close) = rest.find("](")
                    && let Some(paren) = rest[close..].find(')')
                    && !rest[1..close].contains(']')
                {
                    self.push(&s[plain..i], f.clone());
                    let link = self.link_fmt(f);
                    let len = close + paren + 1;
                    let line_show = {
                        let near = self.cursor_near(rest, len);
                        std::mem::replace(&mut self.show, near)
                    };
                    self.marker("[", f);
                    self.inline(&rest[1..close], &link);
                    self.marker(&rest[close..len], f);
                    self.show = line_show;
                    handled = Some(len);
                }
            } else {
                for (m, kind) in [
                    ("**", 0),
                    ("__", 0),
                    ("~~", 1),
                    ("==", 2),
                    ("*", 3),
                    ("_", 3),
                ] {
                    if !rest.starts_with(m) {
                        continue;
                    }
                    // `_` only counts at a word boundary (snake_case stays plain)
                    if m.starts_with('_')
                        && s[..i].chars().last().is_some_and(|c| c.is_alphanumeric())
                    {
                        break;
                    }
                    let after = &rest[m.len()..];
                    let Some(end) = after.find(m) else { break };
                    if end == 0 || after.starts_with(' ') {
                        break;
                    }
                    self.push(&s[plain..i], f.clone());
                    let inner_f = match kind {
                        0 => TextFormat {
                            font_id: FontId::new(f.font_id.size, bold_family()),
                            ..f.clone()
                        },
                        1 => TextFormat {
                            strikethrough: Stroke::new(1.0, f.color),
                            color: th.muted,
                            ..f.clone()
                        },
                        2 => TextFormat {
                            background: th.mark_bg,
                            ..f.clone()
                        },
                        _ => TextFormat {
                            font_id: FontId::new(f.font_id.size, italic_family()),
                            ..f.clone()
                        },
                    };
                    self.marker(m, f);
                    self.inline(&after[..end], &inner_f);
                    self.marker(m, f);
                    handled = Some(m.len() * 2 + end);
                    break;
                }
            }

            match handled {
                Some(n) => {
                    i += n;
                    plain = i;
                }
                None => i += rest.chars().next().map_or(1, char::len_utf8),
            }
        }
        self.push(&s[plain..], f.clone());
    }

    fn link_fmt(&self, f: &TextFormat) -> TextFormat {
        TextFormat {
            color: self.th.link,
            underline: Stroke::new(1.0, self.th.link.gamma_multiply(0.5)),
            ..f.clone()
        }
    }
}

/// If byte offset `at` is inside a `[[wikilink]]` or `[text](url)`, return its target
/// and the link's byte range in `src`.
pub enum LinkHit {
    Wiki(String),
    Url(String),
}

/// If the cursor (byte `at`) is right after an unclosed `[[query`, return
/// (byte offset of `[[`, query). Used to show link suggestions.
pub fn link_query(src: &str, at: usize) -> Option<(usize, &str)> {
    let before = &src[..at];
    let open = before.rfind("[[")?;
    let q = &before[open + 2..];
    (!q.contains(['\n', '[', ']', '|'])).then_some((open, q))
}

/// Targets of every `[[target]]`, `[[target|label]]` or `[[target#heading]]` in `src`.
pub fn wikilinks(src: &str) -> Vec<&str> {
    let mut out = vec![];
    for line in src.lines() {
        let mut rest = line;
        while let Some(o) = rest.find("[[") {
            rest = &rest[o + 2..];
            let Some(c) = rest.find("]]") else { break };
            let target = rest[..c].split(['|', '#']).next().unwrap_or("").trim();
            if !target.is_empty() {
                out.push(target);
            }
            rest = &rest[c + 2..];
        }
    }
    out
}

pub fn link_at(src: &str, at: usize) -> Option<(LinkHit, Range<usize>)> {
    let line_start = src[..at].rfind('\n').map_or(0, |p| p + 1);
    let line_end = src[at..].find('\n').map_or(src.len(), |p| at + p);
    let line = &src[line_start..line_end];
    let pos = at - line_start;

    let mut search = 0;
    while let Some(o) = line[search..].find("[[") {
        let open = search + o;
        let Some(c) = line[open + 2..].find("]]") else {
            break;
        };
        let close = open + 2 + c + 2;
        if (open..=close).contains(&pos) {
            let inner = &line[open + 2..close - 2];
            let target = inner.split_once('|').map_or(inner, |(t, _)| t);
            let hit = LinkHit::Wiki(target.trim().to_string());
            return Some((hit, line_start + open..line_start + close));
        }
        search = close;
    }

    let mut search = 0;
    while let Some(o) = line[search..].find('[') {
        let open = search + o;
        let Some(mid) = line[open..].find("](").map(|m| open + m) else {
            break;
        };
        // `[` of a wikilink or stray bracket: not a `[text](url)` start (same rule as layout)
        if line[open + 1..mid].contains(['[', ']']) {
            search = open + 1;
            continue;
        }
        let Some(end) = line[mid..].find(')').map(|e| mid + e) else {
            break;
        };
        if (open..=end).contains(&pos) {
            let hit = LinkHit::Url(line[mid + 2..end].to_string());
            return Some((hit, line_start + open..line_start + end + 1));
        }
        search = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn th() -> Theme {
        Theme {
            text: Color32::WHITE,
            muted: Color32::GRAY,
            link: Color32::YELLOW,
            code_bg: Color32::BLACK,
            mark_bg: Color32::BROWN,
            size: 15.0,
            char_w: 9.0,
        }
    }

    /// The layout must contain exactly the source text (cursor positions depend on it).
    #[test]
    fn layout_preserves_text() {
        let src = "# Title\n\nSome **bold**, *it*, `code`, ~~x~~, ==hl== and [[Note|alias]] + [a](http://x).\n\
                   - item\n- [ ] todo\n- [x] done\n1. one\n> quote\n---\n```rust\nlet a_b = 1;\n```\nsnake_case_name **unclosed\nü **ñ** é\n\n| A | Bé |\n|---|:-:|\n| 1 |  |\n| x | y | extra |";
        for reveal in [None, Some(0..=0), Some(0..=20)] {
            let job = layout(src, reveal, Some(5..5), &th(), 500.0);
            assert_eq!(job.text, src);
        }
    }

    #[test]
    fn markers_hidden_off_cursor_line() {
        let job = layout("**bold**", None, None, &th(), 500.0);
        let hidden = job
            .sections
            .iter()
            .filter(|s| s.format.color == Color32::TRANSPARENT)
            .count();
        assert_eq!(hidden, 2);
        let job = layout("**bold**", Some(0..=0), None, &th(), 500.0);
        assert!(
            job.sections
                .iter()
                .all(|s| s.format.color != Color32::TRANSPARENT)
        );
    }

    #[test]
    fn table_detected_unless_revealed() {
        let src = "intro\n| A | B |\n| --- | --- |\n| long cell | x |";
        let t = tables(src, 10.0);
        assert_eq!((t.len(), t[0].first, t[0].sep, t[0].last), (1, 1, 2, 3));
        assert_eq!(t[0].cols, vec![0.0, 130.0, 180.0]);
        assert!(tables("| not | a table |\ntext", 10.0).is_empty());
    }

    #[test]
    fn finds_links() {
        let s = "see [[My Note|alias]] and [site](https://a.b)";
        assert!(
            matches!(link_at(s, 8), Some((LinkHit::Wiki(t), r)) if t == "My Note" && r == (4..21))
        );
        assert!(
            matches!(link_at(s, 28), Some((LinkHit::Url(u), r)) if u == "https://a.b" && r == (26..45))
        );
        assert!(link_at(s, 1).is_none());
    }

    /// On the cursor's line, a link stays collapsed until the cursor touches it.
    #[test]
    fn link_markers_reveal_only_at_cursor() {
        let src = "see [[English/test|test]] here";
        // bytes drawn invisible (egui merges equal adjacent sections, so count bytes)
        let hidden = |job: &LayoutJob| -> usize {
            job.sections
                .iter()
                .filter(|s| s.format.color == Color32::TRANSPARENT)
                .map(|s| s.byte_range.end.0 - s.byte_range.start.0)
                .sum()
        };
        let line = Some(0..=0);
        assert_eq!(
            hidden(&layout(src, line.clone(), Some(1..1), &th(), 500.0)),
            17
        ); // "[[English/test|" + "]]" hidden though the cursor is on this line
        assert_eq!(
            hidden(&layout(src, line.clone(), Some(10..10), &th(), 500.0)),
            0
        ); // inside
        assert_eq!(hidden(&layout(src, line, Some(25..25), &th(), 500.0)), 0); // right after ]]
        assert_eq!(hidden(&layout(src, None, Some(29..29), &th(), 500.0)), 17);
    }

    #[test]
    fn finds_link_query() {
        assert_eq!(
            wikilinks("[[A]] and [[b/C|see c]]\n[[D#Intro]] [[ ]] [[open"),
            ["A", "b/C", "D"]
        );
        assert_eq!(link_query("see [[My No", 11), Some((4, "My No")));
        assert_eq!(link_query("see [[", 6), Some((4, "")));
        assert_eq!(link_query("[[a]] b", 7), None); // already closed
        assert_eq!(link_query("[[a\nb", 5), None); // other line
        assert_eq!(link_query("plain", 5), None);
    }
}
