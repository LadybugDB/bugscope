//! Squarified treemap over Leiden communities.
//!
//! Layout follows the approach in <https://github.com/tobi/disktree>
//! (`disktree-core/src/treemap.rs`): Bruls, Huizing and van Wijk's squarified
//! algorithm — grow a row of tiles while the worst aspect ratio keeps
//! improving, then start a new row in the remaining space. That is what yields
//! a legible mosaic instead of the slivers a naive slice-and-dice gives.
//!
//! Two levels are laid out: communities fill the viewport, and each
//! community's members fill its body below a header band that keeps the
//! community's name (so a parent's label never sits on top of a child).
//! Painting lives in `ui.rs`; everything here is pure geometry and is unit
//! tested below.

/// An axis-aligned rectangle in canvas pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Shrink on every side, never past empty. The gaps between tiles are
    /// what separate them, so no borders are needed while painting.
    #[must_use]
    pub fn inset(&self, padding: f32) -> Self {
        let w = (self.w - 2.0 * padding).max(0.0);
        let h = (self.h - 2.0 * padding).max(0.0);
        Self::new(self.x + padding, self.y + padding, w, h)
    }
}

/// One Leiden community, ready for layout.
#[derive(Clone, Debug)]
pub struct Community {
    /// Position in the layout (communities are pre-sorted, largest first);
    /// doubles as the palette slot, like disktree's category hue.
    pub slot: usize,
    /// Arbitrary Leiden community id (for status text, not layout).
    pub id: u64,
    /// Member node indices into the displayed graph, heaviest first.
    pub members: Vec<usize>,
    /// Node weights parallel to `members` (PageRank scores, or 1.0).
    pub weights: Vec<f64>,
    /// Sum of member weights — the community's share of the viewport.
    pub weight: f64,
}

/// Group per-node community assignments into communities, largest first.
///
/// `weights` is an optional per-node weight (PageRank); `None` weighs every
/// node equally. Members inside each community come out heaviest first.
pub fn build_communities(
    node_count: usize,
    assignment: &[u64],
    weights: Option<&[f64]>,
) -> Vec<Community> {
    // Renumber arbitrary Leiden ids in first-seen order, then accumulate.
    let mut renumber: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    for i in 0..node_count {
        let cid = assignment.get(i).copied().unwrap_or(0);
        let pos = *renumber.entry(cid).or_insert_with(|| {
            members.push(Vec::new());
            members.len() - 1
        });
        members[pos].push(i);
    }
    let mut out: Vec<Community> = members
        .into_iter()
        .enumerate()
        .map(|(pos, mut mem)| {
            mem.sort_by(|&a, &b| {
                let wa = weights.and_then(|w| w.get(a).copied()).unwrap_or(1.0);
                let wb = weights.and_then(|w| w.get(b).copied()).unwrap_or(1.0);
                wb.total_cmp(&wa)
            });
            let ws: Vec<f64> = mem
                .iter()
                .map(|&m| {
                    let w = weights.and_then(|w| w.get(m).copied()).unwrap_or(1.0);
                    if w > 0.0 && w.is_finite() {
                        w
                    } else {
                        0.0
                    }
                })
                .collect();
            let total: f64 = ws.iter().sum();
            // Keep the original Leiden id for status text: recover it from
            // the first member's assignment.
            let id = mem
                .first()
                .and_then(|&m| assignment.get(m).copied())
                .unwrap_or(pos as u64);
            Community {
                slot: 0, // filled in after sorting
                id,
                members: mem,
                weights: ws,
                weight: total,
            }
        })
        .collect();
    // Largest community first: stable palette slots + biggest tile top-left.
    out.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    for (slot, c) in out.iter_mut().enumerate() {
        c.slot = slot;
    }
    out
}

/// One rectangle of the mosaic.
#[derive(Clone, Debug)]
pub struct Tile {
    /// Position of the community in the layout's community list.
    pub community: usize,
    /// `Some(node_idx)` for a member cell, `None` for a community header band.
    pub node: Option<usize>,
    pub rect: Rect,
    /// Whether this rect is the header band (community label) rather than a cell.
    pub header: bool,
}

/// Lay out `communities` inside `area`: one rect per community, subdivided
/// into one cell per member below a header band.
///
/// `min_tile` drops cells too small to read or hit; `header_h` is the band
/// height a community keeps for its name when its body stays usable.
pub fn layout_treemap(
    communities: &[Community],
    area: Rect,
    min_tile: f32,
    header_h: f32,
) -> Vec<Tile> {
    let mut tiles = Vec::new();
    if communities.is_empty() || area.w <= 0.0 || area.h <= 0.0 {
        return tiles;
    }
    let values: Vec<f64> = communities.iter().map(|c| c.weight.max(0.0)).collect();
    // Communities with no weight still deserve a sliver so singletons stay
    // reachable; squarify skips non-positive values, so lift zeros slightly.
    let values: Vec<f64> = values
        .iter()
        .map(|&v| if v > 0.0 { v } else { 1e-6 })
        .collect();
    for (pos, raw) in squarify(&values, area).iter().enumerate() {
        let rect = raw.inset(2.0);
        if rect.w < min_tile || rect.h < min_tile {
            continue;
        }
        let community = &communities[pos];
        // A community that fits only its header stays whole: a name drawn
        // over its own single cell is worse than one level less of detail.
        let header = header_band(rect, header_h, min_tile);
        tiles.push(Tile {
            community: pos,
            node: None,
            rect: header.unwrap_or(rect),
            header: header.is_some(),
        });
        let Some(band) = header else { continue };
        let body = Rect::new(rect.x, band.bottom(), rect.w, rect.bottom() - band.bottom());
        if body.w <= 0.0 || body.h <= 0.0 {
            continue;
        }
        let member_values: Vec<f64> = community
            .weights
            .iter()
            .map(|&w| if w > 0.0 { w } else { 1e-6 })
            .collect();
        for (k, cell) in squarify(&member_values, body).iter().enumerate() {
            let cell = cell.inset(1.0);
            if cell.w < min_tile || cell.h < min_tile {
                continue;
            }
            tiles.push(Tile {
                community: pos,
                node: Some(community.members[k]),
                rect: cell,
                header: false,
            });
        }
    }
    tiles
}

/// The band a community keeps for its name, or `None` when the tile is too
/// small to leave its members a usable body below it.
fn header_band(rect: Rect, height: f32, min_tile: f32) -> Option<Rect> {
    if rect.w < 44.0 || rect.h - height < min_tile * 3.0 {
        return None;
    }
    Some(Rect::new(rect.x, rect.y, rect.w, height))
}

/// Split `area` into one rectangle per value, proportional to it.
///
/// Returned in the order of `values`, not in layout order. After Bruls,
/// Huizing and van Wijk ("Squarified Treemaps"), following disktree's port.
pub fn squarify(values: &[f64], area: Rect) -> Vec<Rect> {
    let mut rects = vec![Rect::default(); values.len()];
    let total: f64 = values.iter().filter(|v| **v > 0.0).sum();
    if total <= 0.0 || area.w <= 0.0 || area.h <= 0.0 {
        return rects;
    }

    let mut order: Vec<usize> = (0..values.len()).filter(|i| values[*i] > 0.0).collect();
    order.sort_by(|a, b| values[*b].total_cmp(&values[*a]));

    let scale = f64::from(area.w) * f64::from(area.h) / total;
    let areas: Vec<f64> = order.iter().map(|i| values[*i] * scale).collect();

    let mut free = area;
    let mut start = 0;
    while start < areas.len() {
        let side = f64::from(free.w.min(free.h));
        let mut end = start + 1;
        let mut row_sum = areas[start];
        let mut row_worst = worst_ratio(&areas[start..end], row_sum, side);

        while end < areas.len() {
            let candidate_sum = row_sum + areas[end];
            let candidate_worst = worst_ratio(&areas[start..=end], candidate_sum, side);
            if candidate_worst > row_worst {
                break;
            }
            row_sum = candidate_sum;
            row_worst = candidate_worst;
            end += 1;
        }

        if free.w >= free.h {
            // A vertical strip on the left; tiles stack top to bottom.
            let strip_w = ((row_sum / f64::from(free.h)) as f32).min(free.w);
            let mut y = free.y;
            for index in start..end {
                let height = if strip_w > 0.0 {
                    (areas[index] / f64::from(strip_w)) as f32
                } else {
                    0.0
                };
                let height = height.min(free.bottom() - y).max(0.0);
                rects[order[index]] = Rect::new(free.x, y, strip_w, height);
                y += height;
            }
            free.x += strip_w;
            free.w -= strip_w;
        } else {
            // A horizontal strip along the top; tiles run left to right.
            let strip_h = ((row_sum / f64::from(free.w)) as f32).min(free.h);
            let mut x = free.x;
            for index in start..end {
                let width = if strip_h > 0.0 {
                    (areas[index] / f64::from(strip_h)) as f32
                } else {
                    0.0
                };
                let width = width.min(free.right() - x).max(0.0);
                rects[order[index]] = Rect::new(x, free.y, width, strip_h);
                x += width;
            }
            free.y += strip_h;
            free.h -= strip_h;
        }
        start = end;
    }
    rects
}

/// Worst (largest) aspect ratio in a row of `areas` laid along `side`.
fn worst_ratio(areas: &[f64], row_sum: f64, side: f64) -> f64 {
    if row_sum <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let thickness = row_sum / side;
    areas.iter().fold(0.0_f64, |worst, area| {
        if *area <= 0.0 || thickness <= 0.0 {
            return worst;
        }
        let other = area / thickness;
        worst.max((thickness / other).max(other / thickness))
    })
}

/// The deepest tile containing a point.
///
/// Member cells are emitted after their community header and are inset inside
/// the community rect, so the last match in reverse order is the most
/// specific one.
pub fn hit(tiles: &[Tile], x: f32, y: f32) -> Option<&Tile> {
    tiles.iter().rev().find(|tile| tile.rect.contains(x, y))
}

/// Sunburst (radial partition) over Leiden communities: pure geometry.
///
/// The inner ring holds one arc per community, sized by community weight;
/// the outer ring subdivides each community's span into member arcs sized
/// by member weight. Angles start at the top (−π/2) and run clockwise in
/// screen space (y down). Painting + canvas wiring live in `ui.rs`.
#[derive(Clone, Debug)]
pub struct SunburstWedge {
    /// Position of the community in the layout's community list.
    pub community: usize,
    /// `Some(node_idx)` for a member arc, `None` for a community arc.
    pub node: Option<usize>,
    /// Angular span in radians, measured clockwise from the top.
    pub start: f32,
    pub end: f32,
    /// Absolute radii in canvas pixels.
    pub inner: f32,
    pub outer: f32,
}

impl SunburstWedge {
    pub fn span(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }

    pub fn mid_angle(&self) -> f32 {
        (self.start + self.end) / 2.0
    }

    pub fn mid_radius(&self) -> f32 {
        (self.inner + self.outer) / 2.0
    }

    /// Arc length at the mid radius — the readability/hit metric,
    /// mirroring `min_tile` in the treemap layout.
    pub fn arc_len(&self) -> f32 {
        self.span() * self.mid_radius()
    }
}

/// Center + radius for the sunburst in a viewport, shared by paint +
/// hit-testing so both always agree.
pub fn sunburst_frame(vw: f32, vh: f32) -> (f32, f32, f32) {
    let w = vw.max(50.0);
    let h = vh.max(50.0);
    (w / 2.0, h / 2.0, (w.min(h) / 2.0 - 8.0).max(20.0))
}

/// Fraction of the radius kept as the center circle (the "up" affordance).
pub const SUNBURST_CENTER_FRAC: f32 = 0.30;

/// Lay out `communities` as sunburst wedges for a `radius`-px sunburst.
/// Member arcs narrower than ~2px at their mid radius are dropped — they
/// can neither be read nor hit, like sub-`min_tile` treemap cells.
pub fn layout_sunburst(communities: &[Community], radius: f32) -> Vec<SunburstWedge> {
    let mut wedges = Vec::new();
    if communities.is_empty() || radius <= 0.0 {
        return wedges;
    }
    let total: f64 = communities.iter().map(|c| c.weight.max(0.0)).sum();
    let n = communities.len();
    let two_pi = std::f32::consts::TAU;
    let base = -std::f32::consts::FRAC_PI_2;
    let (c_inner, c_outer) = (radius * SUNBURST_CENTER_FRAC, radius * 0.62);
    let (m_inner, m_outer) = (radius * 0.64, radius * 0.97);
    let mut a = base;
    for (pos, comm) in communities.iter().enumerate() {
        let share = if total > 0.0 {
            (comm.weight.max(0.0) / total) as f32
        } else {
            1.0 / n.max(1) as f32
        };
        let (a0, a1) = (a, a + share * two_pi);
        a = a1;
        if a1 <= a0 {
            continue;
        }
        wedges.push(SunburstWedge {
            community: pos,
            node: None,
            start: a0,
            end: a1,
            inner: c_inner,
            outer: c_outer,
        });
        let mtotal: f64 = comm.weights.iter().map(|w| (*w).max(0.0)).sum();
        let mcount = comm.members.len().max(1) as f64;
        let mut ma = a0;
        for (k, &m) in comm.members.iter().enumerate() {
            let w = comm.weights.get(k).copied().unwrap_or(1.0).max(0.0);
            let mspan = if mtotal > 0.0 {
                (w / mtotal) as f32 * (a1 - a0)
            } else {
                (a1 - a0) / mcount as f32
            };
            let (m0, m1) = (ma, ma + mspan);
            ma = m1;
            if m1 <= m0 {
                continue;
            }
            let wedge = SunburstWedge {
                community: pos,
                node: Some(m),
                start: m0,
                end: m1,
                inner: m_inner,
                outer: m_outer,
            };
            if wedge.arc_len() < 2.0 {
                continue;
            }
            wedges.push(wedge);
        }
    }
    wedges
}

/// Polar pick result for a canvas-local point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SunburstHit {
    /// Inside the center circle — the "up" affordance.
    Center,
    /// A wedge: community position + optional member node index.
    Wedge {
        community: usize,
        node: Option<usize>,
    },
}

/// Pick the sunburst at a canvas-local point. `radius` is the outer radius
/// from [`sunburst_frame`]; points outside it miss (`None`).
pub fn pick_sunburst(
    wedges: &[SunburstWedge],
    cx: f32,
    cy: f32,
    radius: f32,
    x: f32,
    y: f32,
) -> Option<SunburstHit> {
    let dx = x - cx;
    let dy = y - cy;
    let r = dx.hypot(dy);
    if r > radius {
        return None;
    }
    if r < radius * SUNBURST_CENTER_FRAC {
        return Some(SunburstHit::Center);
    }
    let base = -std::f32::consts::FRAC_PI_2;
    let mut a = dy.atan2(dx) - base;
    while a < 0.0 {
        a += std::f32::consts::TAU;
    }
    while a >= std::f32::consts::TAU {
        a -= std::f32::consts::TAU;
    }
    let a = a + base;
    // Outer (member) wedges are emitted after their community arc, so
    // reverse order prefers the most specific ring — like `hit` above.
    wedges
        .iter()
        .rev()
        .find(|w| a >= w.start && a < w.end && r >= w.inner && r <= w.outer)
        .map(|w| SunburstHit::Wedge {
            community: w.community,
            node: w.node,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 800.0, 500.0)
    }

    fn two_cliques() -> Vec<Community> {
        // Two 4-cliques, as in the GDS_LEIDEN two-cliques test.
        build_communities(8, &[0, 0, 0, 0, 1, 1, 1, 1], None)
    }

    #[test]
    fn squarify_fills_the_area() {
        let values = [40.0, 30.0, 20.0, 5.0, 3.0, 2.0];
        let rects = squarify(&values, area());
        let covered: f32 = rects.iter().map(Rect::area).sum();
        assert!(
            (covered - area().area()).abs() < 1.0,
            "covered {covered} of {}",
            area().area()
        );
        for rect in &rects {
            assert!(rect.x >= -0.01 && rect.y >= -0.01);
            assert!(rect.right() <= area().right() + 0.01);
            assert!(rect.bottom() <= area().bottom() + 0.01);
        }
    }

    #[test]
    fn squarify_keeps_aspect_ratios_reasonable() {
        let values = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let rects = squarify(&values, Rect::new(0.0, 0.0, 600.0, 400.0));
        for rect in &rects {
            let ratio = (rect.w / rect.h).max(rect.h / rect.w);
            assert!(ratio <= 4.0, "aspect ratio {ratio} in {rect:?}");
        }
    }

    #[test]
    fn squarify_survives_degenerate_input() {
        assert_eq!(squarify(&[], area()).len(), 0);
        assert!(squarify(&[0.0, 0.0], area())
            .iter()
            .all(|r| r.area().abs() < f32::EPSILON));
        assert!(
            squarify(&[1.0], Rect::new(0.0, 0.0, 0.0, 10.0))[0]
                .area()
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn communities_sort_largest_first_with_stable_slots() {
        let cs = build_communities(5, &[7, 7, 7, 3, 3], None);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].members.len(), 3);
        assert_eq!(cs[1].members.len(), 2);
        assert_eq!((cs[0].slot, cs[1].slot), (0, 1));
    }

    #[test]
    fn layout_nests_cells_inside_their_community() {
        let cs = two_cliques();
        let tiles = layout_treemap(&cs, area(), 5.0, 20.0);
        // 2 headers + 8 member cells.
        assert_eq!(tiles.iter().filter(|t| t.header).count(), 2);
        assert_eq!(tiles.iter().filter(|t| !t.header).count(), 8);
        for cell in tiles.iter().filter(|t| !t.header) {
            let header = tiles
                .iter()
                .find(|t| t.header && t.community == cell.community)
                .expect("community header");
            // Cells start below their community's header band.
            assert!(
                cell.rect.y >= header.rect.bottom() - f32::EPSILON,
                "{cell:?} overlaps its header"
            );
        }
    }

    #[test]
    fn hit_prefers_the_member_cell_over_the_header() {
        let cs = two_cliques();
        let tiles = layout_treemap(&cs, area(), 5.0, 20.0);
        let cell = tiles.iter().find(|t| !t.header).expect("a cell");
        let cx = cell.rect.x + cell.rect.w / 2.0;
        let cy = cell.rect.y + cell.rect.h / 2.0;
        let found = hit(&tiles, cx, cy).expect("a hit");
        assert_eq!(found.node, cell.node);
        assert!(hit(&tiles, -50.0, -50.0).is_none());
    }

    #[test]
    fn sunburst_spans_partition_the_circle() {
        let cs = two_cliques();
        let wedges = layout_sunburst(&cs, 250.0);
        let comm_spans: f32 = wedges
            .iter()
            .filter(|w| w.node.is_none())
            .map(|w| w.span())
            .sum();
        assert!(
            (comm_spans - std::f32::consts::TAU).abs() < 1e-3,
            "community spans cover the circle: {comm_spans}"
        );
        // 2 community arcs + 8 member arcs, all members kept at this size.
        assert_eq!(wedges.iter().filter(|w| w.node.is_none()).count(), 2);
        assert_eq!(wedges.iter().filter(|w| w.node.is_some()).count(), 8);
        // Every member span sits inside its parent community span.
        for m in wedges.iter().filter(|w| w.node.is_some()) {
            let parent = wedges
                .iter()
                .find(|w| w.node.is_none() && w.community == m.community)
                .expect("parent arc");
            assert!(m.start >= parent.start - 1e-4 && m.end <= parent.end + 1e-4);
        }
    }

    #[test]
    fn sunburst_pick_resolves_rings() {
        let cs = two_cliques();
        let radius = 250.0;
        let wedges = layout_sunburst(&cs, radius);
        let (cx, cy, _) = (400.0, 250.0, radius);
        // Center circle → up affordance; outside → miss.
        assert_eq!(
            pick_sunburst(&wedges, cx, cy, radius, cx, cy),
            Some(SunburstHit::Center)
        );
        assert_eq!(
            pick_sunburst(&wedges, cx, cy, radius, cx + radius + 50.0, cy),
            None
        );
        // A point safely inside the first community arc → that arc. (The
        // exact midpoint can sit on a shared span boundary in f32.)
        let first = wedges.iter().find(|w| w.node.is_none()).unwrap();
        let probe_a = first.start + first.span() * 0.25;
        let mid_r = first.mid_radius();
        let x = cx + mid_r * probe_a.cos();
        let y = cy + mid_r * probe_a.sin();
        assert_eq!(
            pick_sunburst(&wedges, cx, cy, radius, x, y),
            Some(SunburstHit::Wedge {
                community: first.community,
                node: None,
            })
        );
        // A member arc midpoint resolves to the member.
        let member = wedges.iter().find(|w| w.node.is_some()).unwrap();
        let (ma, mr) = (member.mid_angle(), member.mid_radius());
        let x = cx + mr * ma.cos();
        let y = cy + mr * ma.sin();
        assert_eq!(
            pick_sunburst(&wedges, cx, cy, radius, x, y),
            Some(SunburstHit::Wedge {
                community: member.community,
                node: member.node,
            })
        );
    }

    #[test]
    fn sunburst_rings_nest_root_community_member() {
        let cs = two_cliques();
        let radius = 250.0;
        let wedges = layout_sunburst(&cs, radius);
        // Community ring starts where the root disc ends; members sit
        // outside their parent ring — root → community → member.
        for w in wedges.iter().filter(|w| w.node.is_none()) {
            assert!((w.inner - radius * SUNBURST_CENTER_FRAC).abs() < 1e-3);
        }
        for m in wedges.iter().filter(|w| w.node.is_some()) {
            let parent = wedges
                .iter()
                .find(|w| w.node.is_none() && w.community == m.community)
                .expect("parent arc");
            assert!(m.inner >= parent.outer - 1e-4);
            assert!(m.outer <= radius + 1e-3);
        }
    }

    #[test]
    fn sunburst_drops_unreadable_members() {
        // One heavy node + 200 dust nodes: dust arcs are sub-pixel.
        let mut assignment = vec![0u64; 201];
        let mut weights = vec![0.0001f64; 201];
        assignment[0] = 0;
        weights[0] = 1000.0;
        let cs = build_communities(201, &assignment, Some(&weights));
        assert_eq!(cs.len(), 1);
        let wedges = layout_sunburst(&cs, 250.0);
        let members = wedges.iter().filter(|w| w.node.is_some()).count();
        assert!(members < 201, "dust filtered: {members} kept");
        assert!(
            wedges.iter().any(|w| w.node == Some(0)),
            "heavy member kept"
        );
        assert!(layout_sunburst(&[], 250.0).is_empty());
        assert!(layout_sunburst(&cs, 0.0).is_empty());
    }

    #[test]
    fn pagerank_weights_scale_community_share() {
        let weights = [10.0, 10.0, 10.0, 10.0, 1.0, 1.0, 1.0, 1.0];
        let cs = build_communities(8, &[0, 0, 0, 0, 1, 1, 1, 1], Some(&weights));
        assert_eq!(cs.len(), 2);
        // Heavy community sorts first and owns 10x the weight.
        assert!((cs[0].weight - 40.0).abs() < 1e-9);
        assert!((cs[1].weight - 4.0).abs() < 1e-9);
        let tiles = layout_treemap(&cs, area(), 5.0, 20.0);
        let area_of = |community: usize| {
            tiles
                .iter()
                .filter(|t| t.community == community)
                .map(|t| t.rect.area())
                .sum::<f32>()
        };
        // Header bands are shared per community, so compare loosely.
        assert!(area_of(0) > area_of(1) * 2.0);
    }
}
