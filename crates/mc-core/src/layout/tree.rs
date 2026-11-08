//! Reading the tree, rewriting it, and the transitions the user makes on it.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::tab_order::reorder;
use crate::types::{Edge, Group, Layout, Node, SessionRef, Split};

use super::{clamp_ratio, dir_of, leads};

/// Tabs whose id starts with this are panels of the app, not sessions.
pub const PANEL_PREFIX: &str = "panel:";

/// Whether a tab is a panel rather than a session (see [`PANEL_PREFIX`]).
pub fn is_panel(id: &str) -> bool {
    id.starts_with(PANEL_PREFIX)
}

/// One empty group.
pub fn empty_layout() -> Layout {
    Layout::default()
}

// ---------- reading ----------

/// The groups in reading order: left before right, top before bottom.
pub fn groups_of(node: &Node) -> Vec<&Group> {
    fn collect<'a>(node: &'a Node, out: &mut Vec<&'a Group>) {
        match node {
            Node::Group(g) => out.push(g),
            Node::Split(s) => {
                collect(&s.first, out);
                collect(&s.second, out);
            }
        }
    }
    let mut out = Vec::new();
    collect(node, &mut out);
    out
}

pub fn find_group<'a>(l: &'a Layout, group_id: &str) -> Option<&'a Group> {
    groups_of(&l.root).into_iter().find(|g| g.id == group_id)
}

/// The group that has this tab (`groupOf`).
pub fn group_of_tab<'a>(l: &'a Layout, session_id: &str) -> Option<&'a Group> {
    groups_of(&l.root).into_iter().find(|g| g.tabs.iter().any(|t| t == session_id))
}

/// Every tab, group after group.
pub fn flatten(l: &Layout) -> Vec<&str> {
    groups_of(&l.root).into_iter().flat_map(|g| g.tabs.iter().map(String::as_str)).collect()
}

/// The sessions on screen: the active tab of every group.
pub fn visible_ids(l: &Layout) -> Vec<&str> {
    groups_of(&l.root).into_iter().filter_map(|g| g.active.as_deref()).collect()
}

pub fn is_visible(l: &Layout, session_id: &str) -> bool {
    visible_ids(l).contains(&session_id)
}

/// The session that has the keyboard.
pub fn active_id_of(l: &Layout) -> Option<&str> {
    find_group(l, &l.focused)?.active.as_deref()
}

// ---------- rewriting the tree ----------
//
// A rewritten node is `Cow::Borrowed` when it is the very node that went in (`same`), so untouched
// parts keep their identity as they do in the TypeScript.

/// Whether a rewrite gave back the node it got, untouched.
#[allow(clippy::ptr_arg)] // the variant is what is asked: a `&Node` cannot tell
fn same(node: &Cow<'_, Node>, original: &Node) -> bool {
    matches!(node, Cow::Borrowed(n) if std::ptr::eq(*n, original))
}

/// `first === node.first && second === node.second ? node : { ...node, first, second }`.
fn rebuild<'a>(node: &'a Node, split: &'a Split, first: Cow<'a, Node>, second: Cow<'a, Node>) -> Cow<'a, Node> {
    if same(&first, &split.first) && same(&second, &split.second) {
        Cow::Borrowed(node)
    } else {
        Cow::Owned(Node::Split(Split {
            id: split.id.clone(),
            dir: split.dir,
            ratio: split.ratio,
            first: Box::new(first.into_owned()),
            second: Box::new(second.into_owned()),
        }))
    }
}

/// Rebuilds the tree with every group passed through `f`; None from `f` keeps the group as it is.
pub(super) fn map_groups<'a>(node: &'a Node, f: &mut impl FnMut(&Group) -> Option<Group>) -> Cow<'a, Node> {
    match node {
        Node::Group(g) => match f(g) {
            None => Cow::Borrowed(node),
            Some(g) => Cow::Owned(Node::Group(g)),
        },
        Node::Split(s) => {
            let first = map_groups(&s.first, f);
            let second = map_groups(&s.second, f);
            rebuild(node, s, first, second)
        }
    }
}

fn replace_group<'a>(node: &'a Node, group_id: &str, f: &mut impl FnMut(&Group) -> Node) -> Cow<'a, Node> {
    match node {
        Node::Group(g) if g.id == group_id => Cow::Owned(f(g)),
        Node::Group(_) => Cow::Borrowed(node),
        Node::Split(s) => {
            let first = replace_group(&s.first, group_id, f);
            let second = replace_group(&s.second, group_id, f);
            rebuild(node, s, first, second)
        }
    }
}

/// Removes empty groups: a split with one gives its place to its other child. None when no tab is
/// left; the caller decides what the lone empty root is.
pub(super) fn collapse(node: &Node) -> Option<Cow<'_, Node>> {
    match node {
        Node::Group(g) => (!g.tabs.is_empty()).then_some(Cow::Borrowed(node)),
        Node::Split(s) => {
            let first = collapse(&s.first);
            let second = collapse(&s.second);
            match (first, second) {
                (None, second) => second,
                (first, None) => first,
                (Some(first), Some(second)) => Some(rebuild(node, s, first, second)),
            }
        }
    }
}

/// The tab that stands in for a tab that left: the one now at its position, else the last one.
fn fallback(tabs: &[String], index: usize) -> Option<String> {
    let last = tabs.len().checked_sub(1)?;
    Some(tabs[index.min(last)].clone())
}

/// The group without this tab; None when it does not have it.
pub(super) fn without_tab(group: &Group, session_id: &str) -> Option<Group> {
    let index = group.tabs.iter().position(|t| t == session_id)?;
    let tabs: Vec<String> = group.tabs.iter().filter(|t| *t != session_id).cloned().collect();
    let active = if group.active.as_deref() == Some(session_id) { fallback(&tabs, index) } else { group.active.clone() };
    Some(Group { id: group.id.clone(), tabs, active })
}

/// The tree without this tab, its empty groups folded away; None when no tab is left.
pub(super) fn rest_without(l: &Layout, session_id: &str) -> Option<Node> {
    let without = map_groups(&l.root, &mut |g| without_tab(g, session_id));
    collapse(&without).map(Cow::into_owned)
}

/// Brings a tree back into shape after a change: no empty groups, every group shows one of its
/// tabs, and the keyboard is in a group that exists (`prefer` if it does). Gives back `l` itself
/// when `root` is `l.root` untouched and neither the focus nor `seq` changes.
pub(super) fn settle<'a>(l: &'a Layout, root: Cow<'_, Node>, prefer: &str, seq: u64) -> Cow<'a, Layout> {
    // with no tab left anywhere, one empty group remains
    let lone = || match &*root {
        Node::Group(_) => Cow::Borrowed(&*root),
        Node::Split(_) => {
            Cow::Owned(Node::Group(Group { id: groups_of(&root)[0].id.clone(), tabs: Vec::new(), active: None }))
        }
    };
    let collapsed = collapse(&root).unwrap_or_else(lone);
    let next = map_groups(&collapsed, &mut |g| {
        let shown = if g.tabs.is_empty() {
            g.active.is_none()
        } else {
            g.active.as_ref().is_some_and(|a| g.tabs.contains(a))
        };
        if shown {
            None
        } else {
            Some(Group { id: g.id.clone(), tabs: g.tabs.clone(), active: fallback(&g.tabs, g.tabs.len().saturating_sub(1)) })
        }
    });
    let all = groups_of(&next);
    let focused = if all.iter().any(|g| g.id == prefer) {
        prefer
    } else if all.iter().any(|g| g.id == l.focused) {
        l.focused.as_str()
    } else {
        all[0].id.as_str()
    }
    .to_string();
    if same(&next, &l.root) && focused == l.focused && seq == l.seq {
        Cow::Borrowed(l)
    } else {
        Cow::Owned(Layout { root: next.into_owned(), focused, seq })
    }
}

// ---------- transitions ----------

/// A tab was chosen: its group shows it and gets the keyboard.
pub fn activate<'a>(l: &'a Layout, session_id: &str) -> Cow<'a, Layout> {
    let Some(group) = group_of_tab(l, session_id) else { return Cow::Borrowed(l) };
    let shown = group.active.as_deref() == Some(session_id);
    if shown && l.focused == group.id {
        return Cow::Borrowed(l);
    }
    let root = if shown {
        l.root.clone()
    } else {
        replace_group(&l.root, &group.id, &mut |g| {
            Node::Group(Group { id: g.id.clone(), tabs: g.tabs.clone(), active: Some(session_id.to_string()) })
        })
        .into_owned()
    };
    Cow::Owned(Layout { root, focused: group.id.clone(), seq: l.seq })
}

pub fn focus_group<'a>(l: &'a Layout, group_id: &str) -> Cow<'a, Layout> {
    if l.focused == group_id || find_group(l, group_id).is_none() {
        Cow::Borrowed(l)
    } else {
        Cow::Owned(Layout { focused: group_id.to_string(), ..l.clone() })
    }
}

/// `(index + step + n) % n` of the TypeScript, for an index that may be -1 (not found).
fn step_from(index: Option<usize>, step: i32, n: usize) -> usize {
    let index = index.map_or(-1, |i| i as i64);
    (index + i64::from(step)).rem_euclid(n as i64) as usize
}

/// Gives the keyboard to the next (`step` 1) or the previous (-1) group.
pub fn focus_step(l: &Layout, step: i32) -> Cow<'_, Layout> {
    let all = groups_of(&l.root);
    if all.len() < 2 {
        return Cow::Borrowed(l);
    }
    let index = all.iter().position(|g| g.id == l.focused);
    let focused = all[step_from(index, step, all.len())].id.clone();
    Cow::Owned(Layout { focused, ..l.clone() })
}

/// Goes to the next (`step` 1) or the previous (-1) tab, through all groups in reading order.
pub fn cycle_tab(l: &Layout, step: i32) -> Cow<'_, Layout> {
    let all = flatten(l);
    if all.len() < 2 {
        return Cow::Borrowed(l);
    }
    let index = active_id_of(l).and_then(|a| all.iter().position(|t| *t == a));
    activate(l, all[step_from(index, step, all.len())])
}

/// A tab position in [`tab_at`]: by index, or the last tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabPos {
    Index(usize),
    Last,
}

/// Goes to a tab of the group that has the keyboard, by position.
pub fn tab_at(l: &Layout, pos: TabPos) -> Cow<'_, Layout> {
    let tabs = find_group(l, &l.focused).map_or(&[][..], |g| g.tabs.as_slice());
    let target = match pos {
        TabPos::Index(i) => tabs.get(i),
        TabPos::Last => tabs.last(),
    };
    match target {
        Some(id) if !id.is_empty() => activate(l, id),
        _ => Cow::Borrowed(l),
    }
}

/// Moves a tab in front of `before_id` in a group (to its end with None). Within its own group
/// this only reorders; into another group the tab is shown there and takes the keyboard along.
pub fn move_tab<'a>(l: &'a Layout, session_id: &str, to_group_id: &str, before_id: Option<&str>) -> Cow<'a, Layout> {
    let (Some(from), Some(to)) = (group_of_tab(l, session_id), find_group(l, to_group_id)) else {
        return Cow::Borrowed(l);
    };
    if from.id == to.id {
        let Cow::Owned(tabs) = reorder(&from.tabs, session_id, before_id) else { return Cow::Borrowed(l) };
        let root = replace_group(&l.root, &from.id, &mut |g| {
            Node::Group(Group { id: g.id.clone(), tabs: tabs.clone(), active: g.active.clone() })
        });
        return Cow::Owned(Layout { root: root.into_owned(), focused: l.focused.clone(), seq: l.seq });
    }
    let root = map_groups(&l.root, &mut |g| {
        if g.id == from.id {
            return without_tab(g, session_id);
        }
        if g.id != to.id {
            return None;
        }
        let mut tabs = g.tabs.clone();
        match before_id.and_then(|b| g.tabs.iter().position(|t| t == b)) {
            Some(at) => tabs.insert(at, session_id.to_string()),
            None => tabs.push(session_id.to_string()),
        }
        Some(Group { id: g.id.clone(), tabs, active: Some(session_id.to_string()) })
    });
    settle(l, root, &to.id, l.seq)
}

/// Whether a tab can get an area of its own next to `target_group_id`: not when it is that
/// group's only tab.
pub fn can_split(l: &Layout, session_id: &str, target_group_id: &str) -> bool {
    match (group_of_tab(l, session_id), find_group(l, target_group_id)) {
        (Some(from), Some(target)) => !(from.id == target.id && from.tabs.len() == 1),
        _ => false,
    }
}

fn fresh_group(l: &Layout, session_id: &str) -> Group {
    Group {
        id: format!("g{}", l.seq.saturating_add(1)),
        tabs: vec![session_id.to_string()],
        active: Some(session_id.to_string()),
    }
}

/// A split of `fresh` and `other` on `edge`, numbered after `l.seq`.
fn split_of(l: &Layout, edge: Edge, fresh: Node, other: Node) -> Node {
    let (first, second) = if leads(edge) { (fresh, other) } else { (other, fresh) };
    Node::Split(Split {
        id: format!("s{}", l.seq.saturating_add(2)),
        dir: dir_of(edge),
        ratio: 0.5,
        first: Box::new(first),
        second: Box::new(second),
    })
}

/// Gives a tab a new area on one side of a group; the keyboard goes with it.
pub fn split_with<'a>(l: &'a Layout, session_id: &str, target_group_id: &str, edge: Edge) -> Cow<'a, Layout> {
    if !can_split(l, session_id, target_group_id) {
        return Cow::Borrowed(l);
    }
    let fresh = fresh_group(l, session_id);
    let fresh_id = fresh.id.clone();
    let root = map_groups(&l.root, &mut |g| without_tab(g, session_id));
    let mut fresh = Some(Node::Group(fresh));
    let root = replace_group(&root, target_group_id, &mut |target| {
        let fresh = fresh.take().unwrap_or_else(|| Node::Group(fresh_group(l, session_id)));
        split_of(l, edge, fresh, Node::Group(target.clone()))
    });
    settle(l, root, &fresh_id, l.seq.saturating_add(2))
}

/// Gives a tab a new area along one side of the whole window.
pub fn split_root<'a>(l: &'a Layout, session_id: &str, edge: Edge) -> Cow<'a, Layout> {
    if group_of_tab(l, session_id).is_none() || flatten(l).len() < 2 {
        return Cow::Borrowed(l);
    }
    let Some(rest) = rest_without(l, session_id) else { return Cow::Borrowed(l) };
    let fresh = fresh_group(l, session_id);
    let fresh_id = fresh.id.clone();
    let root = split_of(l, edge, Node::Group(fresh), rest);
    settle(l, Cow::Owned(root), &fresh_id, l.seq.saturating_add(2))
}

pub fn set_ratio<'a>(l: &'a Layout, split_id: &str, ratio: f64) -> Cow<'a, Layout> {
    fn visit<'a>(node: &'a Node, split_id: &str, next: f64) -> Cow<'a, Node> {
        match node {
            Node::Group(_) => Cow::Borrowed(node),
            Node::Split(s) if s.id == split_id => {
                if s.ratio == next {
                    Cow::Borrowed(node)
                } else {
                    Cow::Owned(Node::Split(Split { ratio: next, ..s.clone() }))
                }
            }
            Node::Split(s) => {
                let first = visit(&s.first, split_id, next);
                let second = visit(&s.second, split_id, next);
                rebuild(node, s, first, second)
            }
        }
    }
    let root = visit(&l.root, split_id, clamp_ratio(ratio));
    if same(&root, &l.root) {
        Cow::Borrowed(l)
    } else {
        Cow::Owned(Layout { root: root.into_owned(), focused: l.focused.clone(), seq: l.seq })
    }
}

/// Takes a tab out of the layout: its area shows the tab that took its position, an area left
/// empty folds away. For panel tabs; the tabs of sessions follow the session list ([`reconcile`]).
pub fn remove_tab<'a>(l: &'a Layout, tab_id: &str) -> Cow<'a, Layout> {
    if group_of_tab(l, tab_id).is_none() {
        return Cow::Borrowed(l);
    }
    let root = map_groups(&l.root, &mut |g| without_tab(g, tab_id));
    settle(l, root, &l.focused, l.seq)
}

/// Puts every tab back into one area, in reading order.
pub fn merge_all(l: &Layout) -> Cow<'_, Layout> {
    let all = groups_of(&l.root);
    if all.len() < 2 {
        return Cow::Borrowed(l);
    }
    let id = all[0].id.clone();
    let active = active_id_of(l).map(str::to_string).or_else(|| all[0].active.clone());
    let tabs = flatten(l).into_iter().map(str::to_string).collect();
    Cow::Owned(Layout { root: Node::Group(Group { id: id.clone(), tabs, active }), focused: id, seq: l.seq })
}

/// Brings the layout in line with a new session list. A session that replaces another takes its
/// tab; tabs of sessions that are gone disappear, and the tab that took an active tab's position
/// is shown instead; new sessions get a tab where their `place` says, else in the group that has
/// the keyboard. Returns the same layout (`Cow::Borrowed`) when nothing changes.
///
/// Panel tabs (ids starting with [`PANEL_PREFIX`]) are never in the session list and count as
/// alive all the same: they stay where they are until the renderer removes them itself
/// ([`remove_tab`]). `layout.ts` does the same; session ids are UUIDs, so no session tab can be
/// mistaken for a panel.
pub fn reconcile<'a>(l: &'a Layout, sessions: &[SessionRef]) -> Cow<'a, Layout> {
    let ids: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    let live = |id: &str| ids.contains(id) || is_panel(id);
    let successor: HashMap<&str, &str> = sessions
        .iter()
        .filter_map(|s| s.replaces.as_deref().filter(|r| !r.is_empty()).map(|r| (r, s.id.as_str())))
        .collect();

    let mut root = map_groups(&l.root, &mut |g| {
        if g.tabs.iter().all(|id| live(id) && !successor.contains_key(id.as_str())) {
            return None;
        }
        let now: Vec<&str> = g.tabs.iter().map(|id| successor.get(id.as_str()).copied().unwrap_or(id)).collect();
        let tabs: Vec<String> = now.iter().filter(|id| live(id)).map(|id| id.to_string()).collect();
        let index = g.active.as_ref().and_then(|a| g.tabs.iter().position(|t| t == a));
        let active = match index {
            Some(i) if live(now[i]) => Some(now[i].to_string()),
            _ => fallback(&tabs, now[..index.unwrap_or(0)].iter().filter(|id| live(id)).count()),
        };
        Some(Group { id: g.id.clone(), tabs, active })
    });

    let mut placed: HashSet<String> =
        groups_of(&root).into_iter().flat_map(|g| g.tabs.iter().cloned()).collect();
    for session in sessions {
        if !placed.insert(session.id.clone()) {
            continue;
        }
        let place = session.place.as_ref();
        let anchor = place.and_then(|p| p.after_id.as_deref()).filter(|a| !a.is_empty());
        let (target, beside) = {
            let all = groups_of(&root);
            let beside = anchor.and_then(|a| all.iter().find(|g| g.tabs.iter().any(|t| t == a)));
            let target = beside
                .or_else(|| all.iter().find(|g| place.and_then(|p| p.group_id.as_deref()) == Some(g.id.as_str())))
                .or_else(|| all.iter().find(|g| g.id == l.focused))
                .unwrap_or(&all[0]);
            (target.id.clone(), beside.is_some())
        };
        let next = replace_group(&root, &target, &mut |g| {
            let at = match anchor {
                Some(a) if beside => g.tabs.iter().position(|t| t == a).map_or(0, |i| i + 1),
                _ => g.tabs.len(),
            };
            let mut tabs = g.tabs.clone();
            tabs.insert(at, session.id.clone());
            // a group that showed nothing shows its first tab; otherwise the caller decides what comes to the front
            let active = g.active.clone().or_else(|| Some(session.id.clone()));
            Node::Group(Group { id: g.id.clone(), tabs, active })
        })
        .into_owned();
        root = Cow::Owned(next);
    }
    settle(l, root, &l.focused, l.seq)
}
