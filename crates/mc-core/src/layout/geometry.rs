//! Where the areas and dividers go on screen, and what a tab dropped at a point would do.
//!
//! Each area has a tab strip. With [`StripSide::Top`] (the default, and the only placement of the
//! Electron version) it is a band of height `strip` along the top of the area; with
//! [`StripSide::Left`] it is a column of width `strip` along its left side. The area's body is the
//! rest. `Top` reproduces `layout.ts` exactly; what `Left` changes is said at each function.

use crate::types::{Dir, Edge, Layout, Node, RATIO_MAX, RATIO_MIN};

use super::tree::{can_split, flatten, group_of_tab, rest_without};
use super::{dir_of, js_round};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupRect {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl GroupRect {
    pub fn rect(&self) -> Rect {
        Rect { x: self.x, y: self.y, w: self.w, h: self.h }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DividerRect {
    pub split_id: String,
    pub dir: Dir,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// The box of the split it belongs to (`box` in the TypeScript).
    pub bounds: Rect,
    /// The ratios that keep both sides at their minimum size.
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Geometry {
    pub groups: Vec<GroupRect>,
    pub dividers: Vec<DividerRect>,
}

/// Where an area's tab strip is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StripSide {
    /// A band of height `strip` along the top of the area.
    #[default]
    Top,
    /// A column of width `strip` along the left of the area.
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Pixels between two areas.
    pub gap: f64,
    /// The smallest an area may get. With the strip on top it is included in `min_h`; a strip on
    /// the left comes on top of `min_w`.
    pub min_w: f64,
    pub min_h: f64,
    pub strip_side: StripSide,
    /// Width of a left strip; not used with the strip on top.
    pub strip: f64,
}

impl Metrics {
    /// With the strip on top, as in the Electron version.
    pub fn new(gap: f64, min_w: f64, min_h: f64) -> Self {
        Metrics { gap, min_w, min_h, strip_side: StripSide::Top, strip: 0.0 }
    }

    pub fn with_strip(self, strip_side: StripSide, strip: f64) -> Self {
        Metrics { strip_side, strip, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// The smallest a single area may get: `Left` adds the strip to the width.
fn group_min(m: &Metrics) -> Size {
    match m.strip_side {
        StripSide::Top => Size { w: m.min_w, h: m.min_h },
        StripSide::Left => Size { w: m.min_w + m.strip, h: m.min_h },
    }
}

/// The smallest size a part of the tree needs so that each of its areas keeps its minimum.
pub fn min_size(node: &Node, m: &Metrics) -> Size {
    match node {
        Node::Group(_) => group_min(m),
        Node::Split(s) => {
            let a = min_size(&s.first, m);
            let b = min_size(&s.second, m);
            match s.dir {
                Dir::Row => Size { w: a.w + m.gap + b.w, h: a.h.max(b.h) },
                Dir::Column => Size { w: a.w.max(b.w), h: a.h + m.gap + b.h },
            }
        }
    }
}

/// Where each area and each divider goes, in whole pixels. The second child of a split gets what
/// the first leaves, so the parts always add up to the whole. The strip side only matters through
/// [`min_size`].
pub fn layout_rects(root: &Node, w: f64, h: f64, m: &Metrics) -> Geometry {
    fn place(node: &Node, bx: Rect, m: &Metrics, geometry: &mut Geometry) {
        let s = match node {
            Node::Group(g) => {
                geometry.groups.push(GroupRect { id: g.id.clone(), x: bx.x, y: bx.y, w: bx.w, h: bx.h });
                return;
            }
            Node::Split(s) => s,
        };
        let row = s.dir == Dir::Row;
        let total = 0f64.max((if row { bx.w } else { bx.h }) - m.gap);
        let a = min_size(&s.first, m);
        let b = min_size(&s.second, m);
        let least = if row { a.w } else { a.h };
        let most = total - if row { b.w } else { b.h };
        let fits = most >= least;
        // too small for both minimums: share what there is
        let size = if fits {
            most.min(least.max(js_round(total * s.ratio)))
        } else {
            js_round(total * s.ratio)
        };
        let (first_box, second_box) = if row {
            (Rect { w: size, ..bx }, Rect { x: bx.x + size + m.gap, w: total - size, ..bx })
        } else {
            (Rect { h: size, ..bx }, Rect { y: bx.y + size + m.gap, h: total - size, ..bx })
        };
        place(&s.first, first_box, m, geometry);
        let line = if row {
            Rect { x: bx.x + size, y: bx.y, w: m.gap, h: bx.h }
        } else {
            Rect { x: bx.x, y: bx.y + size, w: bx.w, h: m.gap }
        };
        geometry.dividers.push(DividerRect {
            split_id: s.id.clone(),
            dir: s.dir,
            x: line.x,
            y: line.y,
            w: line.w,
            h: line.h,
            bounds: bx,
            min: if total > 0.0 && fits { least / total } else { RATIO_MIN },
            max: if total > 0.0 && fits { most / total } else { RATIO_MAX },
        });
        place(&s.second, second_box, m, geometry);
    }
    let mut geometry = Geometry::default();
    place(root, Rect { x: 0.0, y: 0.0, w: 0f64.max(w), h: 0f64.max(h) }, m, &mut geometry);
    geometry
}

/// The ratio a divider would get if dragged to a pointer position, kept within its limits.
pub fn ratio_at(divider: &DividerRect, x: f64, y: f64) -> f64 {
    let row = divider.dir == Dir::Row;
    let total = if row { divider.bounds.w - divider.w } else { divider.bounds.h - divider.h };
    if total <= 0.0 {
        return 0.5;
    }
    let offset = if row { x - divider.bounds.x - divider.w / 2.0 } else { y - divider.bounds.y - divider.h / 2.0 };
    divider.max.min(divider.min.max(offset / total))
}

/// What a tab dropped at a point would do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Hit {
    /// Onto a tab strip: the strip itself says where among the tabs.
    Strip { group_id: String },
    /// Into the middle of an area: join it.
    Move { group_id: String },
    /// Near a side of an area: a new area there.
    Split { group_id: String, edge: Edge },
    /// Along a side of the window: a new area along the whole window.
    Root { edge: Edge },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitOptions {
    /// Height (strip on top) or width (strip on the left) of a group's tab strip.
    pub strip: f64,
    pub strip_side: StripSide,
    /// Width of the band along the window's sides that stands for the whole window.
    pub root_band: f64,
    /// Share of an area, from each side, that means "a new area on this side".
    pub edge_share: f64,
}

impl HitOptions {
    /// With the strip on top, as in the Electron version.
    pub fn new(strip: f64, root_band: f64, edge_share: f64) -> Self {
        HitOptions { strip, strip_side: StripSide::Top, root_band, edge_share }
    }

    pub fn with_strip_side(self, strip_side: StripSide) -> Self {
        HitOptions { strip_side, ..self }
    }
}

/// The part of an area below (`Top`) or right of (`Left`) its tab strip.
fn body_of(group: &GroupRect, strip: f64, side: StripSide) -> Rect {
    match side {
        StripSide::Top => Rect { x: group.x, y: group.y + strip, w: group.w, h: group.h - strip },
        StripSide::Left => Rect { x: group.x + strip, y: group.y, w: group.w - strip, h: group.h },
    }
}

/// What a tab dropped at a point would do. The tab strips come first; then, with more than one
/// area, a band along the sides of the window that the strips do not take splits the whole
/// window; in an area, near a side means a new area there, the middle means joining that area.
///
/// `Top`: the band runs along the left, right and bottom (the top belongs to the tab strips),
/// checked in that order. `Left`: a point with `x < group.x + strip` is on the strip; the band
/// runs along the right, top and bottom (the left belongs to the strips), checked in that order;
/// the sides of an area are measured in its body right of the strip. In both, a point as near to
/// two sides goes to the first of left, right, top, bottom.
pub fn hit_test(geometry: &Geometry, w: f64, h: f64, x: f64, y: f64, o: &HitOptions) -> Option<Hit> {
    let group = geometry.groups.iter().find(|g| x >= g.x && x < g.x + g.w && y >= g.y && y < g.y + g.h)?;
    let group_id = || group.id.clone();
    let on_strip = match o.strip_side {
        StripSide::Top => y < group.y + o.strip,
        StripSide::Left => x < group.x + o.strip,
    };
    if on_strip {
        return Some(Hit::Strip { group_id: group_id() });
    }
    if geometry.groups.len() > 1 {
        let left = x < o.root_band;
        let right = x >= w - o.root_band;
        let top = y < o.root_band;
        let bottom = y >= h - o.root_band;
        let band = match o.strip_side {
            StripSide::Top => [(left, Edge::Left), (right, Edge::Right), (bottom, Edge::Bottom)],
            StripSide::Left => [(right, Edge::Right), (top, Edge::Top), (bottom, Edge::Bottom)],
        };
        if let Some((_, edge)) = band.into_iter().find(|(inside, _)| *inside) {
            return Some(Hit::Root { edge });
        }
    }
    // the fractions are computed as in the TypeScript, so that `Top` gives the same results to the last bit
    let (fx, fy) = match o.strip_side {
        StripSide::Top => {
            let body_h = group.h - o.strip;
            if group.w <= 0.0 || body_h <= 0.0 {
                return Some(Hit::Move { group_id: group_id() });
            }
            ((x - group.x) / group.w, (y - group.y - o.strip) / body_h)
        }
        StripSide::Left => {
            let body_w = group.w - o.strip;
            if body_w <= 0.0 || group.h <= 0.0 {
                return Some(Hit::Move { group_id: group_id() });
            }
            ((x - group.x - o.strip) / body_w, (y - group.y) / group.h)
        }
    };
    let sides = [(Edge::Left, fx), (Edge::Right, 1.0 - fx), (Edge::Top, fy), (Edge::Bottom, 1.0 - fy)];
    let (edge, distance) = sides[1..].iter().fold(sides[0], |best, side| if side.1 < best.1 { *side } else { best });
    Some(if distance < o.edge_share {
        Hit::Split { group_id: group_id(), edge }
    } else {
        Hit::Move { group_id: group_id() }
    })
}

fn half(bx: Rect, edge: Edge) -> Rect {
    match edge {
        Edge::Left => Rect { w: bx.w / 2.0, ..bx },
        Edge::Right => Rect { x: bx.x + bx.w / 2.0, w: bx.w / 2.0, ..bx },
        Edge::Top => Rect { h: bx.h / 2.0, ..bx },
        Edge::Bottom => Rect { y: bx.y + bx.h / 2.0, h: bx.h / 2.0, ..bx },
    }
}

/// The part of the window to highlight for a drop; a drop on a tab strip is shown in the strip
/// itself. The TypeScript takes the strip height alone; here the options also say where it is.
pub fn preview_rect(geometry: &Geometry, w: f64, h: f64, hit: &Hit, o: &HitOptions) -> Option<Rect> {
    let (group_id, edge) = match hit {
        Hit::Root { edge } => return Some(half(Rect { x: 0.0, y: 0.0, w, h }, *edge)),
        Hit::Strip { .. } => return None,
        Hit::Move { group_id } => (group_id, None),
        Hit::Split { group_id, edge } => (group_id, Some(*edge)),
    };
    let group = geometry.groups.iter().find(|g| g.id == *group_id)?;
    let body = body_of(group, o.strip, o.strip_side);
    Some(edge.map_or(body, |edge| half(body, edge)))
}

/// Whether a drop would change anything and leave every area at least its minimum size.
pub fn can_drop(l: &Layout, session_id: &str, hit: &Hit, geometry: &Geometry, w: f64, h: f64, m: &Metrics) -> bool {
    let Some(from) = group_of_tab(l, session_id) else { return false };
    let one = group_min(m);
    match hit {
        Hit::Strip { .. } => true,
        Hit::Move { group_id } => from.id != *group_id,
        Hit::Root { edge } => {
            if flatten(l).len() < 2 {
                return false;
            }
            let Some(rest) = rest_without(l, session_id) else { return false };
            let need = min_size(&rest, m);
            match dir_of(*edge) {
                Dir::Row => w >= need.w + m.gap + one.w,
                Dir::Column => h >= need.h + m.gap + one.h,
            }
        }
        Hit::Split { group_id, edge } => {
            if !can_split(l, session_id, group_id) {
                return false;
            }
            let Some(bx) = geometry.groups.iter().find(|g| g.id == *group_id) else { return false };
            match dir_of(*edge) {
                Dir::Row => bx.w >= 2.0 * one.w + m.gap,
                Dir::Column => bx.h >= 2.0 * one.h + m.gap,
            }
        }
    }
}
