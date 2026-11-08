//! The saved form of a layout: tabs are positions in the saved tab list instead of session ids.

use std::borrow::Cow;

use crate::types::{Group, Layout, Node, SavedLayout, SavedNode, SessionRef, Split};

use super::clamp_ratio;
use super::tree::{empty_layout, groups_of, reconcile};

/// Rewrites the tab numbers of a saved tree; tabs mapped to None go, and with them groups left
/// empty. None when no tab is left.
pub fn remap_saved(saved: &SavedLayout, map: impl Fn(i64) -> Option<i64>) -> Option<SavedLayout> {
    struct Remap<'f, F> {
        map: &'f F,
        target: u32,
        groups_seen: u32,
        focused: Option<u32>,
        kept: u32,
    }
    impl<F: Fn(i64) -> Option<i64>> Remap<'_, F> {
        fn visit(&mut self, node: &SavedNode) -> Option<SavedNode> {
            match node {
                SavedNode::Group { tabs, active } => {
                    let was_focused = self.groups_seen == self.target;
                    self.groups_seen += 1;
                    let pairs: Vec<(i64, i64)> = tabs.iter().filter_map(|&i| (self.map)(i).map(|j| (i, j))).collect();
                    let &(_, last) = pairs.last()?;
                    if was_focused {
                        self.focused = Some(self.kept);
                    }
                    self.kept += 1;
                    let active = pairs.iter().find(|(i, _)| i == active).map_or(last, |&(_, j)| j);
                    Some(SavedNode::Group { tabs: pairs.into_iter().map(|(_, j)| j).collect(), active })
                }
                SavedNode::Split { dir, ratio, first, second } => {
                    let first = self.visit(first);
                    let second = self.visit(second);
                    match (first, second) {
                        (None, second) => second,
                        (first, None) => first,
                        (Some(first), Some(second)) => Some(SavedNode::Split {
                            dir: *dir,
                            ratio: *ratio,
                            first: Box::new(first),
                            second: Box::new(second),
                        }),
                    }
                }
            }
        }
    }
    let mut remap = Remap { map: &map, target: saved.focused, groups_seen: 0, focused: None, kept: 0 };
    let root = remap.visit(&saved.root)?;
    Some(SavedLayout { root, focused: remap.focused.unwrap_or(0) })
}

/// The layout with session ids replaced by their positions in `ids` (the saved tab list);
/// sessions that are not in `ids` are left out, and so are the groups that only held those.
/// None when no tab is left (`null` in the TypeScript).
pub fn serialize<S: AsRef<str>>(l: &Layout, ids: &[S]) -> Option<SavedLayout> {
    let index_of = |id: &str| ids.iter().position(|x| x.as_ref() == id).map_or(-1, |i| i as i64);
    fn visit(node: &Node, index_of: &dyn Fn(&str) -> i64) -> SavedNode {
        match node {
            Node::Group(g) => SavedNode::Group {
                tabs: g.tabs.iter().map(|id| index_of(id)).collect(),
                active: g.active.as_deref().map_or(-1, index_of),
            },
            Node::Split(s) => SavedNode::Split {
                dir: s.dir,
                ratio: s.ratio,
                first: Box::new(visit(&s.first, index_of)),
                second: Box::new(visit(&s.second, index_of)),
            },
        }
    }
    let focused = groups_of(&l.root).iter().position(|g| g.id == l.focused).unwrap_or(0);
    let raw = SavedLayout { root: visit(&l.root, &index_of), focused: focused as u32 };
    remap_saved(&raw, |index| (index >= 0).then_some(index))
}

/// A layout from a saved one. `ids[i]` is the session that now stands for saved tab `i`, if it
/// was reopened. Sessions in `live_ids` that the saved layout does not mention join the focused
/// group (the result is reconciled with `live_ids`).
pub fn deserialize<A: AsRef<str>, B: AsRef<str>>(saved: &SavedLayout, ids: &[Option<A>], live_ids: &[B]) -> Layout {
    struct Build<'s, A> {
        ids: &'s [Option<A>],
        target: u32,
        seq: u64,
        groups_seen: u32,
        focused: Option<String>,
    }
    impl<A: AsRef<str>> Build<'_, A> {
        fn id_at(&self, index: i64) -> Option<String> {
            let index = usize::try_from(index).ok()?;
            self.ids.get(index)?.as_ref().map(|id| id.as_ref().to_string())
        }
        fn visit(&mut self, node: &SavedNode) -> Option<Node> {
            match node {
                SavedNode::Group { tabs, active } => {
                    let position = self.groups_seen;
                    self.groups_seen += 1;
                    let tabs: Vec<String> = tabs.iter().filter_map(|&i| self.id_at(i)).collect();
                    let last = tabs.last()?.clone();
                    self.seq += 1;
                    let id = format!("g{}", self.seq);
                    let active = self.id_at(*active).unwrap_or(last);
                    if position == self.target {
                        self.focused = Some(id.clone());
                    }
                    Some(Node::Group(Group { id, tabs, active: Some(active) }))
                }
                SavedNode::Split { dir, ratio, first, second } => {
                    let first = self.visit(first);
                    let second = self.visit(second);
                    match (first, second) {
                        (None, second) => second,
                        (first, None) => first,
                        (Some(first), Some(second)) => {
                            self.seq += 1;
                            Some(Node::Split(Split {
                                id: format!("s{}", self.seq),
                                dir: *dir,
                                ratio: clamp_ratio(*ratio),
                                first: Box::new(first),
                                second: Box::new(second),
                            }))
                        }
                    }
                }
            }
        }
    }
    let mut build = Build { ids, target: saved.focused, seq: 0, groups_seen: 0, focused: None };
    let root = build.visit(&saved.root).unwrap_or_else(|| empty_layout().root);
    let focused = build.focused.filter(|f| !f.is_empty()).unwrap_or_else(|| groups_of(&root)[0].id.clone());
    let base = Layout { root, focused, seq: build.seq.max(1) };
    let refs: Vec<SessionRef> =
        live_ids.iter().map(|id| SessionRef { id: id.as_ref().to_string(), replaces: None, place: None }).collect();
    let reconciled = match reconcile(&base, &refs) {
        Cow::Owned(next) => Some(next),
        Cow::Borrowed(_) => None,
    };
    reconciled.unwrap_or(base)
}
