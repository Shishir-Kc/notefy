//! Graph view: notes as dots, `[[links]]` as lines, laid out by a small force simulation.
//! Shown in place of the editor; click a dot to open its note (Ctrl/middle-click: new tab).

use super::*;
use eframe::egui::{Align2, FontId, Sense, Vec2};

/// Length a link line settles at, in graph units.
const LINK_LEN: f32 = 90.0;

pub(crate) struct Graph {
    nodes: Vec<Node>,
    /// Index pairs into `nodes`.
    edges: Vec<(usize, usize)>,
    /// Offset of the graph origin from the view centre (screen px), and scale.
    pan: Vec2,
    zoom: f32,
    /// Node held by the mouse (pinned under the pointer).
    dragging: Option<usize>,
}

struct Node {
    path: PathBuf,
    name: String,
    pos: Vec2,
    vel: Vec2,
    /// Number of links, sets the dot size.
    degree: usize,
}

impl Graph {
    /// Notes that link or are linked (unlinked notes are left out), placed on a spiral.
    pub(crate) fn new(notes: &[Note]) -> Self {
        let mut idx = vec![usize::MAX; notes.len()];
        let (mut nodes, mut edges) = (Vec::<Node>::new(), vec![]);
        for (a, b) in link_edges(notes) {
            for i in [a, b] {
                if idx[i] == usize::MAX {
                    idx[i] = nodes.len();
                    let k = nodes.len() as f32;
                    nodes.push(Node {
                        path: notes[i].path.clone(),
                        name: notes[i]
                            .path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        // golden-angle spiral: an even, overlap-free start
                        pos: Vec2::angled(k * 2.4) * 30.0 * k.sqrt(),
                        vel: Vec2::ZERO,
                        degree: 0,
                    });
                }
                nodes[idx[i]].degree += 1;
            }
            edges.push((idx[a], idx[b]));
        }
        Self {
            nodes,
            edges,
            pan: Vec2::ZERO,
            zoom: 1.0,
            dragging: None,
        }
    }

    /// One layout step (repulsion, link springs, pull to the centre). True while moving.
    // shortcut: O(n²) repulsion, smooth up to ~1–2k linked notes; use a grid/Barnes–Hut beyond.
    fn step(&mut self) -> bool {
        let n = self.nodes.len();
        let mut force: Vec<Vec2> = self.nodes.iter().map(|n| -n.pos * 0.01).collect();
        for i in 0..n {
            for j in i + 1..n {
                let d = self.nodes[i].pos - self.nodes[j].pos;
                let d2 = d.length_sq().max(1.0);
                let f = d / d2.sqrt() * (LINK_LEN * LINK_LEN / d2);
                force[i] += f;
                force[j] -= f;
            }
        }
        for &(a, b) in &self.edges {
            let d = self.nodes[b].pos - self.nodes[a].pos;
            let len = d.length().max(1.0);
            let f = d / len * (len - LINK_LEN) * 0.05;
            force[a] += f;
            force[b] -= f;
        }
        let mut moving = false;
        for (i, (node, f)) in self.nodes.iter_mut().zip(force).enumerate() {
            if self.dragging == Some(i) {
                continue;
            }
            let v = (node.vel + f) * 0.8;
            node.vel = v * (20.0 / v.length().max(20.0)); // cap the speed
            node.pos += node.vel;
            moving |= node.vel.length_sq() > 0.01;
        }
        moving
    }
}

impl Notefy {
    /// Draw the graph in the editor area (pan: drag, zoom: scroll).
    pub(crate) fn graph_view(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let Some(g) = &mut self.graph else { return };
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        if g.nodes.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "No linked notes yet. Link one with [[Note name]].",
                FontId::proportional(14.0),
                p.muted,
            );
            return;
        }

        // zoom around the pointer: the graph point under it stays put
        if let Some(ptr) = resp.hover_pos() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                let old = g.zoom;
                g.zoom = (g.zoom * (dy * 0.003).exp()).clamp(0.1, 6.0);
                let rel = ptr - rect.center() - g.pan;
                g.pan += rel - rel * (g.zoom / old);
            }
        }
        let (origin, zoom) = (rect.center() + g.pan, g.zoom);
        let screen = |v: Vec2| origin + v * zoom;
        let radius = |n: &Node| (4.0 + (n.degree as f32).sqrt() * 1.5) * zoom.sqrt();
        let hovered = resp.hover_pos().and_then(|ptr| {
            g.nodes
                .iter()
                .enumerate()
                .map(|(i, n)| (i, screen(n.pos).distance(ptr) - radius(n)))
                .filter(|&(_, d)| d <= 4.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        });

        if resp.drag_started() {
            g.dragging = hovered;
        }
        if resp.dragged() {
            match (g.dragging, resp.interact_pointer_pos()) {
                (Some(i), Some(ptr)) => {
                    g.nodes[i].pos = (ptr - origin) / zoom;
                    g.nodes[i].vel = Vec2::ZERO;
                }
                _ => g.pan += resp.drag_delta(),
            }
        }
        if resp.drag_stopped() {
            g.dragging = None;
        }
        let mut to_open = None;
        if let Some(i) = hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            let new_tab = resp.middle_clicked() || ui.input(|i| i.modifiers.command);
            if resp.clicked() || resp.middle_clicked() {
                to_open = Some((g.nodes[i].path.clone(), new_tab));
            }
        }

        // hovering a note lights it and its neighbours, dims the rest
        let mut lit = HashSet::new();
        if let Some(h) = hovered {
            lit.insert(h);
            for &(a, b) in &g.edges {
                if a == h || b == h {
                    lit.extend([a, b]);
                }
            }
        }
        let dim = hovered.is_some();
        for &(a, b) in &g.edges {
            let on = dim && lit.contains(&a) && lit.contains(&b);
            let color = if on {
                p.link
            } else {
                p.muted.gamma_multiply(if dim { 0.15 } else { 0.35 })
            };
            painter.line_segment(
                [screen(g.nodes[a].pos), screen(g.nodes[b].pos)],
                Stroke::new(if on { 1.5 } else { 1.0 }, color),
            );
        }
        for (i, n) in g.nodes.iter().enumerate() {
            let at = screen(n.pos);
            let color = if hovered == Some(i) {
                p.accent
            } else if self.current.as_ref() == Some(&n.path) {
                p.link
            } else if lit.contains(&i) {
                p.text
            } else if dim {
                p.muted.gamma_multiply(0.4)
            } else {
                p.muted
            };
            painter.circle_filled(at, radius(n), color);
            if zoom > 0.6 || lit.contains(&i) {
                painter.text(
                    at + Vec2::new(0.0, radius(n) + 3.0),
                    Align2::CENTER_TOP,
                    &n.name,
                    FontId::proportional(12.0),
                    if dim && !lit.contains(&i) {
                        p.muted.gamma_multiply(0.4)
                    } else {
                        p.text
                    },
                );
            }
        }
        painter.text(
            rect.left_top() + Vec2::new(16.0, 12.0),
            Align2::LEFT_TOP,
            format!(
                "{} notes · {} links  —  scroll to zoom, drag to move",
                g.nodes.len(),
                g.edges.len()
            ),
            FontId::proportional(12.0),
            p.muted,
        );

        if g.step() || g.dragging.is_some() {
            ui.ctx().request_repaint();
        }
        if let Some((path, new_tab)) = to_open {
            self.graph = None;
            if new_tab {
                self.open_new_tab(path);
            } else {
                self.open(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_settles_with_links_near_their_length() {
        let v = std::env::temp_dir().join(format!("notefy-graph-{}", std::process::id()));
        let _ = fs::remove_dir_all(&v);
        fs::create_dir_all(&v).unwrap();
        fs::write(v.join("hub.md"), "[[a]] [[b]] [[c]]").unwrap();
        for n in ["a", "b", "c", "alone"] {
            fs::write(v.join(format!("{n}.md")), "").unwrap();
        }
        let mut notes = vec![];
        scan_dir(&v, &v, &mut notes);
        let mut g = Graph::new(&notes);
        assert_eq!((g.nodes.len(), g.edges.len()), (4, 3)); // "alone" left out
        let steps = (0..2000).take_while(|_| g.step()).count();
        assert!(steps < 2000, "layout never settled");
        for &(a, b) in &g.edges {
            let d = (g.nodes[a].pos - g.nodes[b].pos).length();
            assert!(
                (LINK_LEN * 0.7..LINK_LEN * 2.0).contains(&d),
                "link length {d}"
            );
        }
        fs::remove_dir_all(&v).unwrap();
    }
}
