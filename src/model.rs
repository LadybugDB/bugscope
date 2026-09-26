//! Owned graph container + camera + layout.
//!
//! Ports `graph/graphStore.ts` (retention by reference → arena indices),
//! `render/camera.ts` (pan/zoom world↔screen), and `hooks/useForceLayout.ts`
//! (a small Fruchterman–Reingold tick run on the visible graph).

use crate::backend::GraphData;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone)]
pub struct SimNode {
    pub id: String,
    pub name: String,
    pub label: String,
    pub color: usize,
    pub pos: Vec2,
    pub vel: Vec2,
    pub degree: usize,
}

#[derive(Debug, Clone)]
pub struct SimLink {
    pub source: usize,
    pub target: usize,
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct GraphModel {
    pub nodes: Vec<SimNode>,
    pub links: Vec<SimLink>,
    pub index: HashMap<String, usize>,
}

impl GraphModel {
    pub fn load(&mut self, data: &GraphData) {
        self.nodes.clear();
        self.links.clear();
        self.index.clear();
        let n = data.nodes.len().max(1) as f32;
        let radius = (220.0 * n.sqrt()).clamp(300.0, 4000.0);
        let mut degrees: HashMap<&str, usize> = HashMap::new();
        for l in &data.links {
            *degrees.entry(l.source.as_str()).or_insert(0) += 1;
            *degrees.entry(l.target.as_str()).or_insert(0) += 1;
        }
        // First-encounter palette assignment — port of `graph/palette.ts`.
        let mut color_of: HashMap<&str, usize> = HashMap::new();
        for node in &data.nodes {
            let idx = self.nodes.len();
            self.index.insert(node.id.clone(), idx);
            let color = if let Some(&c) = color_of.get(node.label.as_str()) {
                c
            } else {
                let c = color_of.len();
                color_of.insert(node.label.as_str(), c);
                c
            };
            let angle = (idx as f32) * 2.399963; // golden angle
            let r = radius * (0.2 + 0.8 * (idx as f32 / n).sqrt());
            self.nodes.push(SimNode {
                id: node.id.clone(),
                name: node.name.clone(),
                label: node.label.clone(),
                color,
                pos: Vec2 {
                    x: angle.cos() * r,
                    y: angle.sin() * r,
                },
                vel: Vec2 { x: 0.0, y: 0.0 },
                degree: *degrees.get(node.id.as_str()).unwrap_or(&0),
            });
        }
        for l in &data.links {
            if let (Some(&s), Some(&t)) = (self.index.get(l.source.as_str()), self.index.get(l.target.as_str())) {
                self.links.push(SimLink {
                    source: s,
                    target: t,
                    label: l.label.clone(),
                });
            }
        }
    }

    /// One force tick: Coulomb repulsion (sampled) + spring attraction + damping.
    pub fn tick(&mut self) {
        let n = self.nodes.len();
        if n == 0 {
            return;
        }
        let repulsion = 9000.0;
        let spring_len = 90.0;
        let spring_k = 0.02;
        // Repulsion: full O(n²) under 800 nodes, sampled above.
        let stride = if n > 800 { n / 400 } else { 1 }.max(1);
        for i in (0..n).step_by(stride) {
            for j in (i + 1..n).step_by(stride) {
                let dx = self.nodes[i].pos.x - self.nodes[j].pos.x;
                let dy = self.nodes[i].pos.y - self.nodes[j].pos.y;
                let d2 = (dx * dx + dy * dy).max(100.0);
                let d = d2.sqrt();
                let f = repulsion / d2;
                let fx = f * dx / d;
                let fy = f * dy / d;
                self.nodes[i].vel.x += fx;
                self.nodes[i].vel.y += fy;
                self.nodes[j].vel.x -= fx;
                self.nodes[j].vel.y -= fy;
            }
        }
        for link in self.links.clone() {
            let (s, t) = (link.source, link.target);
            let dx = self.nodes[t].pos.x - self.nodes[s].pos.x;
            let dy = self.nodes[t].pos.y - self.nodes[s].pos.y;
            let d = (dx * dx + dy * dy).sqrt().max(1.0);
            let f = spring_k * (d - spring_len);
            let fx = f * dx / d;
            let fy = f * dy / d;
            self.nodes[s].vel.x += fx;
            self.nodes[s].vel.y += fy;
            self.nodes[t].vel.x -= fx;
            self.nodes[t].vel.y -= fy;
        }
        for node in &mut self.nodes {
            node.vel.x *= 0.85;
            node.vel.y *= 0.85;
            node.vel.x = node.vel.x.clamp(-40.0, 40.0);
            node.vel.y = node.vel.y.clamp(-40.0, 40.0);
            node.pos.x += node.vel.x;
            node.pos.y += node.vel.y;
        }
    }

    pub fn pick(&self, world: Vec2, radius: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, node) in self.nodes.iter().enumerate() {
            let size = node_size(node.degree) + 4.0;
            let dx = node.pos.x - world.x;
            let dy = node.pos.y - world.y;
            let d = (dx * dx + dy * dy).sqrt() - size;
            if d <= radius && best.map(|(_, b)| d < b).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }
}

pub fn node_size(degree: usize) -> f32 {
    // Port of `render/nodeSizing.ts` realNodeSize (log-scaled).
    5.0 + 3.0 * ((degree + 1) as f32).ln()
}

/// Camera — port of `render/camera.ts`: world↔screen with zoom + pan.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub center_x: f64,
    pub center_y: f64,
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center_x: 0.0,
            center_y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Camera {
    pub fn world_to_screen(&self, wx: f32, wy: f32, vw: f32, vh: f32) -> (f32, f32) {
        (
            ((wx as f64 - self.center_x) * self.zoom + vw as f64 / 2.0) as f32,
            ((wy as f64 - self.center_y) * self.zoom + vh as f64 / 2.0) as f32,
        )
    }

    pub fn screen_to_world(&self, sx: f32, sy: f32, vw: f32, vh: f32) -> Vec2 {
        Vec2 {
            x: ((sx as f64 - vw as f64 / 2.0) / self.zoom + self.center_x) as f32,
            y: ((sy as f64 - vh as f64 / 2.0) / self.zoom + self.center_y) as f32,
        }
    }

    pub fn zoom_by(&mut self, factor: f64, sx: f32, sy: f32, vw: f32, vh: f32) {
        let before = self.screen_to_world(sx, sy, vw, vh);
        self.zoom = (self.zoom * factor).clamp(0.05, 40.0);
        let after = self.screen_to_world(sx, sy, vw, vh);
        self.center_x += (before.x - after.x) as f64;
        self.center_y += (before.y - after.y) as f64;
    }

    pub fn fit(&mut self, model: &GraphModel, vw: f32, vh: f32) {
        if model.nodes.is_empty() {
            return;
        }
        let (mut x0, mut x1) = (f32::MAX, f32::MIN);
        let (mut y0, mut y1) = (f32::MAX, f32::MIN);
        for n in &model.nodes {
            x0 = x0.min(n.pos.x);
            x1 = x1.max(n.pos.x);
            y0 = y0.min(n.pos.y);
            y1 = y1.max(n.pos.y);
        }
        let w = (x1 - x0).max(1.0) as f64;
        let h = (y1 - y0).max(1.0) as f64;
        self.center_x = ((x0 + x1) / 2.0) as f64;
        self.center_y = ((y0 + y1) / 2.0) as f64;
        self.zoom = ((vw as f64 / w).min(vh as f64 / h) * 0.9).clamp(0.05, 40.0);
    }
}

/// First-encounter node palette — port of `graph/palette.ts` NODE_COLOR_PALETTE.
pub const NODE_PALETTE: [u32; 12] = [
    0x5B8FF9, 0x5AD8A6, 0xF6BD16, 0xE8684A, 0x6DC8EC, 0x9270CA, 0xFF9D4D, 0x269A99, 0xFF99C3,
    0xA3B1C6, 0xD9E021, 0x6DD8D8,
];
