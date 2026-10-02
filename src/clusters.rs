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
