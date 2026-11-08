//! The area tree (`src/shared/layout.ts`): how the tabs are arranged in areas of the window, the
//! transitions the user makes on it, its geometry and drop zones, and its saved form.
//!
//! The renderer keeps its own TypeScript copy of this logic for drag previews; this one validates
//! (`parse_layout`) and persists (`tabs.json`) what the renderer sends, so both must give the same
//! results. The tests are the ones of `test/layout.test.ts`, one for one.
//!
//! **"Returns the same object when nothing changes."** The TypeScript compares layouts by identity.
//! Here every transition takes `&Layout` and returns [`Cow<Layout>`](std::borrow::Cow):
//! `Cow::Borrowed` is the layout that went in (`=== l`), `Cow::Owned` a new one. Inside the tree
//! rewrites the same holds per node: a `Cow::Borrowed` node that points at the node that went in is
//! that node untouched, so `settle` hands back the very layout it got exactly where the TypeScript
//! did.
//!
//! **Panels.** Tabs whose id starts with [`PANEL_PREFIX`] (`panel:`) are not sessions: no session
//! list mentions them, and `reconcile` keeps them alive anyway (see [`reconcile`]).

mod geometry;
mod parse;
mod saved;
mod tree;

#[cfg(test)]
mod tests;

pub use geometry::{
    can_drop, hit_test, layout_rects, min_size, preview_rect, ratio_at, DividerRect, Geometry, GroupRect, Hit, HitOptions,
    Metrics, Rect, Size, StripSide,
};
pub use parse::{parse_layout, parse_saved_layout};
pub use saved::{deserialize, remap_saved, serialize};
pub use tree::{
    activate, active_id_of, can_split, cycle_tab, empty_layout, find_group, flatten, focus_group, focus_step, group_of_tab,
    groups_of, is_panel, is_visible, merge_all, move_tab, reconcile, remove_tab, set_ratio, split_root, split_with, tab_at,
    visible_ids, TabPos, PANEL_PREFIX,
};

pub use crate::types::{
    Dir, Edge, Group, Layout, Node, Place, SavedLayout, SavedNode, SessionRef, Split, RATIO_MAX, RATIO_MIN,
};

/// Deeper trees than this are refused when they come from outside; far more than anyone nests by hand.
pub const MAX_DEPTH: usize = crate::types::LAYOUT_MAX_DEPTH;

/// A ratio within its limits; anything that is not a finite number is an even split.
fn clamp_ratio(ratio: f64) -> f64 {
    if ratio.is_finite() {
        ratio.clamp(RATIO_MIN, RATIO_MAX)
    } else {
        0.5
    }
}

fn dir_of(edge: Edge) -> Dir {
    match edge {
        Edge::Left | Edge::Right => Dir::Row,
        Edge::Top | Edge::Bottom => Dir::Column,
    }
}

/// Whether a new area on this side comes first in its split.
fn leads(edge: Edge) -> bool {
    matches!(edge, Edge::Left | Edge::Top)
}

/// `Math.round`: the nearest whole number, halves towards +infinity (also below zero).
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}
