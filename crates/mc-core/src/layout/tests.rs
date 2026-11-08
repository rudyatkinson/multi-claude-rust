//! `test/layout.test.ts`, one for one (the `reorder` and `dropTarget` parts are in `tab_order`),
//! then what the Rust version adds: panel tabs, the strip on the left, the limits of `parse_layout`.

use std::borrow::Cow;
use std::collections::HashSet;

use serde_json::{json, Value};

use super::*;

fn sref(id: &str) -> SessionRef {
    SessionRef { id: id.to_string(), replaces: None, place: None }
}

fn refs(ids: &[&str]) -> Vec<SessionRef> {
    ids.iter().map(|id| sref(id)).collect()
}

fn replacing(id: &str, old: &str) -> SessionRef {
    SessionRef { id: id.to_string(), replaces: Some(old.to_string()), place: None }
}

fn placed(id: &str, place: Place) -> SessionRef {
    SessionRef { id: id.to_string(), replaces: None, place: Some(place) }
}

/// A layout with one area holding these tabs, the first one in front.
fn one(ids: &[&str]) -> Layout {
    reconcile(&empty_layout(), &refs(ids)).into_owned()
}

/// "a b | c" style picture of a layout: groups in reading order, the shown tab in brackets, * for the keyboard.
fn picture(l: &Layout) -> String {
    groups_of(&l.root)
        .iter()
        .map(|g| {
            let tabs: Vec<String> =
                g.tabs.iter().map(|t| if g.active.as_ref() == Some(t) { format!("[{t}]") } else { t.clone() }).collect();
            format!("{}{}", if g.id == l.focused { "*" } else { "" }, tabs.join(" "))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn shape(node: &Node) -> String {
    match node {
        Node::Group(g) => g.tabs.concat(),
        Node::Split(s) => {
            format!("{}({},{})", if s.dir == Dir::Row { "H" } else { "V" }, shape(&s.first), shape(&s.second))
        }
    }
}

fn split_id(node: &Node) -> &str {
    match node {
        Node::Split(s) => &s.id,
        Node::Group(_) => panic!("a split at the root"),
    }
}

fn ratio_of(node: &Node) -> f64 {
    match node {
        Node::Split(s) => s.ratio,
        Node::Group(_) => panic!("a split at the root"),
    }
}

fn group_id(l: &Layout, session_id: &str) -> String {
    group_of_tab(l, session_id).expect("the tab is in a group").id.clone()
}

/// `toBe(l)`: the very layout that went in.
#[allow(clippy::ptr_arg)] // the variant is what is asked: a `&Layout` cannot tell
fn same(result: &Cow<'_, Layout>, l: &Layout) -> bool {
    matches!(result, Cow::Borrowed(x) if std::ptr::eq(*x, l))
}

/// `toBeCloseTo` with its default precision.
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.005
}

/// What must hold for every layout the transitions produce.
fn check(l: &Layout) {
    fn visit<'a>(node: &'a Node, ids: &mut Vec<&'a str>) {
        match node {
            Node::Group(g) => ids.push(&g.id),
            Node::Split(s) => {
                ids.push(&s.id);
                assert!(s.ratio >= 0.05 && s.ratio <= 0.95, "ratio {} out of bounds", s.ratio);
                visit(&s.first, ids);
                visit(&s.second, ids);
            }
        }
    }
    let all = groups_of(&l.root);
    let mut node_ids = Vec::new();
    visit(&l.root, &mut node_ids);
    assert_eq!(node_ids.iter().collect::<HashSet<_>>().len(), node_ids.len(), "node ids are unique");
    assert!(all.iter().any(|g| g.id == l.focused), "the keyboard is in an existing group");
    let tabs = flatten(l);
    assert_eq!(tabs.iter().collect::<HashSet<_>>().len(), tabs.len(), "a session has one tab");
    for g in &all {
        if g.tabs.is_empty() {
            assert_eq!(all.len(), 1, "only a lone group may be empty");
            assert_eq!(g.active, None);
        } else {
            let active = g.active.as_ref().expect("a group shows one of its tabs");
            assert!(g.tabs.contains(active), "a group shows one of its tabs");
        }
    }
}

mod one_area {
    use super::*;

    #[test]
    fn starts_empty_and_takes_sessions_as_tabs() {
        assert_eq!(picture(&empty_layout()), "*");
        let l = one(&["a", "b", "c"]);
        assert_eq!(picture(&l), "*[a] b c");
        assert_eq!(active_id_of(&l), Some("a"));
        check(&l);
    }

    #[test]
    fn shows_the_chosen_tab() {
        let l = activate(&one(&["a", "b", "c"]), "c").into_owned();
        assert_eq!(picture(&l), "*a b [c]");
        assert!(same(&activate(&l, "c"), &l));
        assert!(same(&activate(&l, "nope"), &l));
    }

    #[test]
    fn goes_to_a_tab_by_position_and_cycles_through_them() {
        let l = one(&["a", "b", "c"]);
        assert_eq!(active_id_of(&tab_at(&l, TabPos::Index(2))), Some("c"));
        assert_eq!(active_id_of(&tab_at(&l, TabPos::Last)), Some("c"));
        assert!(same(&tab_at(&l, TabPos::Index(7)), &l));
        assert_eq!(active_id_of(&cycle_tab(&l, 1)), Some("b"));
        assert_eq!(active_id_of(&cycle_tab(&l, -1)), Some("c"));
        assert_eq!(*cycle_tab(&one(&["a"]), 1), one(&["a"]));
    }

    #[test]
    fn reorders_its_tabs_without_changing_which_one_is_shown() {
        let l = one(&["a", "b", "c"]);
        assert_eq!(picture(&move_tab(&l, "c", "g1", Some("a"))), "*c [a] b");
        assert!(same(&move_tab(&l, "a", "g1", Some("b")), &l));
    }
}

mod splitting {
    use super::*;

    fn abc() -> Layout {
        one(&["a", "b", "c"])
    }

    #[test]
    fn gives_a_tab_an_area_of_its_own_on_the_side_it_was_dropped_on() {
        let abc = abc();
        let right = split_with(&abc, "c", "g1", Edge::Right).into_owned();
        assert_eq!(picture(&right), "[a] b | *[c]");
        assert_eq!(shape(&right.root), "H(ab,c)");
        assert_eq!(shape(&split_with(&abc, "c", "g1", Edge::Left).root), "H(c,ab)");
        assert_eq!(shape(&split_with(&abc, "c", "g1", Edge::Top).root), "V(c,ab)");
        assert_eq!(shape(&split_with(&abc, "c", "g1", Edge::Bottom).root), "V(ab,c)");
        check(&right);
    }

    #[test]
    fn shows_the_neighbour_in_the_area_a_shown_tab_left() {
        let abc = abc();
        assert_eq!(picture(&split_with(&abc, "a", "g1", Edge::Right)), "[b] c | *[a]");
        assert_eq!(picture(&split_with(&activate(&abc, "c"), "c", "g1", Edge::Right)), "a [b] | *[c]");
    }

    #[test]
    fn cannot_split_off_the_only_tab_of_an_area_next_to_itself() {
        let lone = one(&["a"]);
        assert!(!can_split(&lone, "a", "g1"));
        assert!(same(&split_with(&lone, "a", "g1", Edge::Right), &lone));
        assert!(same(&split_root(&lone, "a", Edge::Left), &lone));
    }

    #[test]
    fn nests_areas_can_be_split_again_in_either_direction() {
        let l1 = split_with(&one(&["a", "b", "c", "d"]), "b", "g1", Edge::Right).into_owned();
        let right = group_id(&l1, "b");
        let l2 = split_with(&l1, "c", &right, Edge::Bottom).into_owned();
        assert_eq!(shape(&l2.root), "H(ad,V(b,c))");
        assert_eq!(picture(&l2), "[a] d | [b] | *[c]");
        check(&l2);
    }

    #[test]
    fn moves_the_only_tab_of_one_area_next_to_another_its_old_area_folds_away() {
        let l1 = split_with(&split_with(&abc(), "b", "g1", Edge::Right), "c", "g1", Edge::Bottom).into_owned();
        assert_eq!(shape(&l1.root), "H(V(a,c),b)");
        let l2 = split_with(&l1, "c", &group_id(&l1, "b"), Edge::Right).into_owned();
        assert_eq!(shape(&l2.root), "H(a,H(b,c))");
        check(&l2);
    }

    #[test]
    fn splits_the_whole_window_along_one_of_its_sides() {
        let l1 = split_with(&abc(), "b", "g1", Edge::Right).into_owned();
        let l2 = split_root(&l1, "c", Edge::Bottom).into_owned();
        assert_eq!(shape(&l2.root), "V(H(a,b),c)");
        assert_eq!(active_id_of(&l2), Some("c"));
        // taking the only tab of an area: that area folds, the tab spans the window
        assert_eq!(shape(&split_root(&l2, "b", Edge::Left).root), "H(b,V(a,c))");
        check(&l2);
    }

    #[test]
    fn moves_a_tab_into_another_area_at_a_position_or_at_the_end() {
        let l1 = split_with(&abc(), "c", "g1", Edge::Right).into_owned();
        let right = group_id(&l1, "c");
        assert_eq!(picture(&move_tab(&l1, "a", &right, Some("c"))), "[b] | *[a] c");
        assert_eq!(picture(&move_tab(&l1, "a", &right, None)), "[b] | *c [a]");
        // the last tab leaving folds the area
        let merged = move_tab(&l1, "c", "g1", None).into_owned();
        assert_eq!(picture(&merged), "*a b [c]");
        assert!(matches!(merged.root, Node::Group(_)));
    }

    #[test]
    fn merges_every_area_back_into_one_keeping_the_tab_that_had_the_keyboard_in_front() {
        let l = split_with(&split_with(&abc(), "b", "g1", Edge::Right), "c", "g1", Edge::Bottom).into_owned();
        assert_eq!(picture(&merge_all(&l)), "*a [c] b");
        assert_eq!(*merge_all(&one(&["a"])), one(&["a"]));
    }

    #[test]
    fn moves_the_keyboard_from_area_to_area_and_through_all_tabs_in_reading_order() {
        let l = split_with(&abc(), "c", "g1", Edge::Right).into_owned();
        assert_eq!(picture(&focus_step(&l, 1)), "*[a] b | [c]");
        assert_eq!(picture(&focus_step(&focus_step(&l, 1), -1)), "[a] b | *[c]");
        // c (right) -> a (left) -> b (left) -> c
        assert_eq!(picture(&cycle_tab(&l, 1)), "*[a] b | [c]");
        assert_eq!(picture(&cycle_tab(&cycle_tab(&l, 1), 1)), "*a [b] | [c]");
        assert_eq!(*focus_step(&one(&["a", "b"]), 1), one(&["a", "b"]));
    }

    #[test]
    fn goes_by_position_within_the_area_that_has_the_keyboard_only() {
        let l = split_with(&abc(), "c", "g1", Edge::Right).into_owned();
        assert!(same(&tab_at(&l, TabPos::Index(1)), &l));
        assert_eq!(active_id_of(&tab_at(&focus_step(&l, 1), TabPos::Index(1))), Some("b"));
    }

    #[test]
    fn changes_the_share_of_a_split_within_limits() {
        let l = split_with(&one(&["a", "b"]), "b", "g1", Edge::Right).into_owned();
        let id = split_id(&l.root).to_string();
        assert_eq!(ratio_of(&set_ratio(&l, &id, 0.3).root), 0.3);
        assert_eq!(ratio_of(&set_ratio(&l, &id, 2.0).root), 0.95);
        assert!(same(&set_ratio(&l, &id, 0.5), &l));
        assert!(same(&set_ratio(&l, "nope", 0.3), &l));
    }

    #[test]
    fn knows_what_is_on_screen() {
        let l = split_with(&abc(), "c", "g1", Edge::Right).into_owned();
        assert_eq!(visible_ids(&l), ["a", "c"]);
    }
}

mod reconciling {
    use super::*;

    fn split() -> Layout {
        split_with(&one(&["a", "b", "c", "d"]), "d", "g1", Edge::Right).into_owned()
    }

    #[test]
    fn returns_the_same_layout_when_the_sessions_are_the_same() {
        let split = split();
        assert!(same(&reconcile(&split, &refs(&["a", "b", "c", "d"])), &split));
        // the list arrives again whenever a session's model or context use changes
        assert!(same(&reconcile(&split, &refs(&["d", "c", "b", "a"])), &split));
        let empty = empty_layout();
        assert!(same(&reconcile(&empty, &[]), &empty));
    }

    #[test]
    fn shows_the_tab_that_took_a_closed_tabs_position_else_the_last_one() {
        let split = split();
        assert_eq!(picture(&reconcile(&activate(&split, "b"), &refs(&["a", "c", "d"]))), "*a [c] | [d]");
        assert_eq!(picture(&reconcile(&activate(&split, "c"), &refs(&["a", "b", "d"]))), "*a [b] | [d]");
        // a closed tab in the background changes nothing on screen
        assert_eq!(picture(&reconcile(&activate(&split, "b"), &refs(&["a", "b", "d"]))), "*a [b] | [d]");
    }

    #[test]
    fn folds_an_area_whose_last_tab_closed_and_hands_the_keyboard_on() {
        let next = reconcile(&split(), &refs(&["a", "b", "c"])).into_owned();
        assert_eq!(picture(&next), "*[a] b c");
        assert!(matches!(next.root, Node::Group(_)));
    }

    #[test]
    fn survives_closing_tabs_across_areas_at_once() {
        let three = split_with(&split(), "c", "g1", Edge::Bottom).into_owned();
        let next = reconcile(&three, &refs(&["b"])).into_owned();
        assert_eq!(picture(&next), "*[b]");
        check(&next);
        assert_eq!(picture(&reconcile(&three, &[])), "*");
    }

    #[test]
    fn a_restarted_session_keeps_its_tab_under_its_new_id_shown_or_not() {
        let split = split();
        let mut list = refs(&["a", "b", "c"]);
        list.push(replacing("d2", "d"));
        assert_eq!(picture(&reconcile(&split, &list)), "[a] b c | *[d2]");
        let mut list = refs(&["a", "c", "d"]);
        list.push(replacing("b2", "b"));
        assert_eq!(picture(&reconcile(&split, &list)), "[a] b2 c | *[d]");
        assert_eq!(picture(&reconcile(&activate(&split, "b"), &list)), "*a [b2] c | [d]");
    }

    #[test]
    fn puts_a_new_session_into_the_area_that_has_the_keyboard_without_bringing_it_to_the_front() {
        assert_eq!(picture(&reconcile(&split(), &refs(&["a", "b", "c", "d", "e"]))), "[a] b c | *[d] e");
        assert_eq!(picture(&reconcile(&empty_layout(), &refs(&["x"]))), "*[x]");
    }

    #[test]
    fn follows_where_a_new_session_asks_to_be_placed() {
        let split = split();
        let mut after = refs(&["a", "b", "c", "d"]);
        after.push(placed("e", Place { group_id: None, after_id: Some("a".into()) }));
        assert_eq!(picture(&reconcile(&split, &after)), "[a] e b c | *[d]");
        let mut in_group = refs(&["a", "b", "c", "d"]);
        in_group.push(placed("e", Place { group_id: Some("g1".into()), after_id: None }));
        assert_eq!(picture(&reconcile(&split, &in_group)), "[a] b c e | *[d]");
    }

    #[test]
    fn falls_back_when_the_wished_place_is_gone() {
        let mut gone = refs(&["a", "b", "c", "d"]);
        gone.push(placed("e", Place { group_id: Some("closed".into()), after_id: Some("closed-tab".into()) }));
        assert_eq!(picture(&reconcile(&split(), &gone)), "[a] b c | *[d] e");
    }
}

mod layout_invariants {
    use super::*;

    /// The generator of the TypeScript test, with its float arithmetic, so that the runs are the same.
    struct Random(i64);

    impl Random {
        fn next(&mut self, n: usize) -> usize {
            // `(seed * 1103515245 + 12345) & 0x7fffffff` on JavaScript numbers: the product is
            // rounded to a double, `&` takes the low bits of that (whole) number
            let p = (self.0 as f64) * 1_103_515_245.0 + 12_345.0;
            let low = p.rem_euclid(4_294_967_296.0) as u64;
            self.0 = (low & 0x7fff_ffff) as i64;
            self.0 as usize % n
        }

        fn pick<T: Clone>(&mut self, items: &[T]) -> T {
            items[self.next(items.len())].clone()
        }

        fn step(&mut self) -> i32 {
            if self.next(2) != 0 {
                1
            } else {
                -1
            }
        }
    }

    #[test]
    fn the_generator_is_the_one_of_the_typescript_test() {
        // printed by node with the TypeScript generator
        let mut random = Random(20_261_007);
        let first: Vec<usize> = (0..12).map(|_| random.next(1000)).collect();
        assert_eq!(first, [676, 832, 368, 0, 456, 432, 792, 32, 248, 920, 728, 608]);
        assert_eq!(random.0, 1_222_788_608);
    }

    #[test]
    fn hold_through_long_random_sequences_of_everything_a_user_can_do() {
        // a small deterministic generator, so a failure can be reproduced
        let mut random = Random(20_261_007);
        let edges = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];

        for run in 0..40 {
            let mut live: Vec<String> = vec!["s1".into(), "s2".into(), "s3".into()];
            let mut made = 3;
            let mut l = reconcile(&empty_layout(), &refs(&["s1", "s2", "s3"])).into_owned();
            for step in 0..120 {
                let groups: Vec<String> = groups_of(&l.root).iter().map(|g| g.id.clone()).collect();
                let op = random.next(10);
                l = if live.is_empty() || op == 0 {
                    made += 1;
                    live.push(format!("s{made}"));
                    let mut list = Vec::new();
                    for (i, id) in live.iter().enumerate() {
                        if i == live.len() - 1 && random.next(2) != 0 {
                            let after = random.pick(&live);
                            list.push(placed(id, Place { group_id: None, after_id: Some(after) }));
                        } else {
                            list.push(sref(id));
                        }
                    }
                    reconcile(&l, &list).into_owned()
                } else if op == 1 {
                    let closing = random.pick(&live);
                    live.retain(|id| *id != closing);
                    let list: Vec<SessionRef> = live.iter().map(|id| sref(id)).collect();
                    reconcile(&l, &list).into_owned()
                } else if op == 2 {
                    let old = random.pick(&live);
                    made += 1;
                    let fresh = format!("s{made}");
                    for id in live.iter_mut() {
                        if *id == old {
                            *id = fresh.clone();
                        }
                    }
                    let list: Vec<SessionRef> =
                        live.iter().map(|id| if *id == fresh { replacing(id, &old) } else { sref(id) }).collect();
                    reconcile(&l, &list).into_owned()
                } else if op == 3 {
                    let id = random.pick(&live);
                    activate(&l, &id).into_owned()
                } else if op == 4 {
                    let id = random.pick(&live);
                    let group = random.pick(&groups);
                    let before = if random.next(2) != 0 { Some(random.pick(&live)) } else { None };
                    move_tab(&l, &id, &group, before.as_deref()).into_owned()
                } else if op == 5 {
                    let id = random.pick(&live);
                    let group = random.pick(&groups);
                    let edge = random.pick(&edges);
                    split_with(&l, &id, &group, edge).into_owned()
                } else if op == 6 {
                    let id = random.pick(&live);
                    let edge = random.pick(&edges);
                    split_root(&l, &id, edge).into_owned()
                } else if op == 7 {
                    if random.next(4) == 0 {
                        merge_all(&l).into_owned()
                    } else {
                        let step = random.step();
                        focus_step(&l, step).into_owned()
                    }
                } else if op == 8 {
                    let step = random.step();
                    cycle_tab(&l, step).into_owned()
                } else {
                    let index = random.next(4);
                    tab_at(&l, TabPos::Index(index)).into_owned()
                };

                check(&l);
                let mut tabs: Vec<&str> = flatten(&l);
                tabs.sort_unstable();
                let mut expected: Vec<&str> = live.iter().map(String::as_str).collect();
                expected.sort_unstable();
                assert_eq!(tabs, expected, "run {run} step {step}");
                // what was saved comes back the same
                let order = flatten(&l);
                if let Some(saved) = serialize(&l, &order) {
                    let text = serde_json::to_string(&saved).unwrap();
                    let parsed = parse_saved_layout(&serde_json::from_str::<Value>(&text).unwrap()).unwrap();
                    let ids: Vec<Option<&str>> = order.iter().map(|id| Some(*id)).collect();
                    let back = deserialize(&parsed, &ids, &order);
                    assert_eq!(picture(&back), picture(&l), "run {run} step {step}");
                    assert_eq!(shape(&back.root), shape(&l.root), "run {run} step {step}");
                }
            }
        }
    }
}

mod geometry {
    use super::*;

    fn metrics() -> Metrics {
        Metrics::new(1.0, 200.0, 120.0)
    }

    fn row() -> Layout {
        split_with(&one(&["a", "b"]), "b", "g1", Edge::Right).into_owned()
    }

    fn three() -> Layout {
        let l = split_with(&one(&["a", "b", "c"]), "b", "g1", Edge::Right).into_owned();
        split_with(&l, "c", &group_id(&l, "b"), Edge::Bottom).into_owned()
    }

    #[test]
    fn fills_the_window_exactly_with_the_gap_between_areas() {
        let row = row();
        let Geometry { groups, dividers } = layout_rects(&row.root, 1001.0, 600.0, &metrics());
        assert_eq!(
            groups,
            [
                GroupRect { id: "g1".into(), x: 0.0, y: 0.0, w: 500.0, h: 600.0 },
                GroupRect { id: groups[1].id.clone(), x: 501.0, y: 0.0, w: 500.0, h: 600.0 },
            ]
        );
        let d = &dividers[0];
        assert_eq!((d.x, d.y, d.w, d.h, d.dir), (500.0, 0.0, 1.0, 600.0, Dir::Row));
        // odd sizes: the second area takes what is left, nothing is lost to rounding
        let odd = layout_rects(&row.root, 1000.0, 600.0, &metrics()).groups;
        assert_eq!(odd[0].w + 1.0 + odd[1].w, 1000.0);
    }

    #[test]
    fn lays_out_nested_splits() {
        let Geometry { groups, dividers } = layout_rects(&three().root, 1001.0, 601.0, &metrics());
        let boxes: Vec<String> = groups.iter().map(|g| format!("{},{} {}x{}", g.x, g.y, g.w, g.h)).collect();
        assert_eq!(boxes, ["0,0 500x601", "501,0 500x300", "501,301 500x300"]);
        assert_eq!(dividers.iter().map(|d| d.dir).collect::<Vec<_>>(), [Dir::Row, Dir::Column]);
    }

    #[test]
    fn keeps_every_area_at_its_minimum_size_while_there_is_room() {
        let row = row();
        let squeezed = set_ratio(&row, split_id(&row.root), 0.05).into_owned();
        let Geometry { groups, dividers } = layout_rects(&squeezed.root, 1001.0, 600.0, &metrics());
        assert_eq!(groups[0].w, 200.0);
        assert!(close(dividers[0].min, 0.2));
        assert!(close(dividers[0].max, 0.8));
        assert_eq!(min_size(&three().root, &metrics()), Size { w: 401.0, h: 241.0 });
        // no room for both minimums: share what there is instead of overflowing
        let tiny = layout_rects(&row.root, 301.0, 600.0, &metrics()).groups;
        assert_eq!(tiny[0].w + 1.0 + tiny[1].w, 301.0);
    }

    #[test]
    fn turns_a_pointer_position_on_a_divider_into_a_ratio_within_its_limits() {
        let dividers = layout_rects(&row().root, 1001.0, 600.0, &metrics()).dividers;
        assert!(close(ratio_at(&dividers[0], 300.5, 10.0), 0.3));
        assert!(close(ratio_at(&dividers[0], 5.0, 10.0), 0.2));
        assert!(close(ratio_at(&dividers[0], 995.0, 10.0), 0.8));
    }
}

mod drop_zones {
    use super::*;

    const W: f64 = 1001.0;
    const H: f64 = 644.0;

    fn metrics() -> Metrics {
        Metrics::new(1.0, 200.0, 120.0)
    }

    fn options() -> HitOptions {
        HitOptions::new(44.0, 24.0, 0.25)
    }

    fn row() -> Layout {
        split_with(&one(&["a", "b", "c"]), "c", "g1", Edge::Right).into_owned()
    }

    fn strip(id: &str) -> Option<Hit> {
        Some(Hit::Strip { group_id: id.into() })
    }

    fn mv(id: &str) -> Option<Hit> {
        Some(Hit::Move { group_id: id.into() })
    }

    fn split(id: &str, edge: Edge) -> Option<Hit> {
        Some(Hit::Split { group_id: id.into(), edge })
    }

    fn root(edge: Edge) -> Option<Hit> {
        Some(Hit::Root { edge })
    }

    #[test]
    fn reads_a_single_area_its_tab_strip_its_four_sides_and_its_middle() {
        let lone = one(&["a", "b"]);
        let geo = layout_rects(&lone.root, W, H, &metrics());
        let o = options();
        assert_eq!(hit_test(&geo, W, H, 500.0, 10.0, &o), strip("g1"));
        assert_eq!(hit_test(&geo, W, H, 500.0, 344.0, &o), mv("g1"));
        assert_eq!(hit_test(&geo, W, H, 30.0, 344.0, &o), split("g1", Edge::Left));
        assert_eq!(hit_test(&geo, W, H, 990.0, 344.0, &o), split("g1", Edge::Right));
        assert_eq!(hit_test(&geo, W, H, 500.0, 60.0, &o), split("g1", Edge::Top));
        assert_eq!(hit_test(&geo, W, H, 500.0, 630.0, &o), split("g1", Edge::Bottom));
        // a corner goes to the nearer side
        assert_eq!(hit_test(&geo, W, H, 20.0, 400.0, &o), split("g1", Edge::Left));
        assert_eq!(hit_test(&geo, W, H, 5000.0, 10.0, &o), None);
    }

    #[test]
    fn with_several_areas_the_outer_band_of_the_window_means_the_whole_window() {
        let row = row();
        let right = group_id(&row, "c");
        let geo = layout_rects(&row.root, W, H, &metrics());
        let o = options();
        assert_eq!(hit_test(&geo, W, H, 10.0, 344.0, &o), root(Edge::Left));
        assert_eq!(hit_test(&geo, W, H, 995.0, 344.0, &o), root(Edge::Right));
        assert_eq!(hit_test(&geo, W, H, 300.0, 640.0, &o), root(Edge::Bottom));
        // just inside the band it is the area's own side again
        assert_eq!(hit_test(&geo, W, H, 40.0, 344.0, &o), split("g1", Edge::Left));
        assert_eq!(hit_test(&geo, W, H, 520.0, 344.0, &o), split(&right, Edge::Left));
        // the top of the window belongs to the tab strips
        assert_eq!(hit_test(&geo, W, H, 700.0, 5.0, &o), strip(&right));
    }

    #[test]
    fn previews_the_half_that_the_new_area_would_take() {
        let row = row();
        let right = group_id(&row, "c");
        let geo = layout_rects(&row.root, W, H, &metrics());
        let o = options();
        let rect = |x, y, w, h| Some(Rect { x, y, w, h });
        let hit = Hit::Split { group_id: "g1".into(), edge: Edge::Right };
        assert_eq!(preview_rect(&geo, W, H, &hit, &o), rect(250.0, 44.0, 250.0, 600.0));
        assert_eq!(preview_rect(&geo, W, H, &Hit::Move { group_id: right }, &o), rect(501.0, 44.0, 500.0, 600.0));
        assert_eq!(preview_rect(&geo, W, H, &Hit::Root { edge: Edge::Bottom }, &o), rect(0.0, 322.0, W, 322.0));
        assert_eq!(preview_rect(&geo, W, H, &Hit::Strip { group_id: "g1".into() }, &o), None);
    }

    #[test]
    fn refuses_drops_that_change_nothing_or_leave_an_area_too_small() {
        let row = row();
        let right = group_id(&row, "c");
        let m = metrics();
        let geo = layout_rects(&row.root, W, H, &m);
        let mv = |id: &str| Hit::Move { group_id: id.into() };
        let split = |id: &str, edge| Hit::Split { group_id: id.into(), edge };
        // onto its own area
        assert!(!can_drop(&row, "a", &mv("g1"), &geo, W, H, &m));
        assert!(can_drop(&row, "a", &mv(&right), &geo, W, H, &m));
        // the only tab of an area beside itself
        assert!(!can_drop(&row, "c", &split(&right, Edge::Left), &geo, W, H, &m));
        assert!(can_drop(&row, "a", &split("g1", Edge::Left), &geo, W, H, &m));
        assert!(can_drop(&row, "a", &Hit::Strip { group_id: "g1".into() }, &geo, W, H, &m));
        // a narrow window: no room for a third column, still room to stack
        let narrow = layout_rects(&row.root, 601.0, H, &m);
        assert!(!can_drop(&row, "a", &split("g1", Edge::Left), &narrow, 601.0, H, &m));
        assert!(can_drop(&row, "a", &split("g1", Edge::Bottom), &narrow, 601.0, H, &m));
        assert!(!can_drop(&row, "a", &Hit::Root { edge: Edge::Left }, &narrow, 601.0, H, &m));
        assert!(can_drop(&row, "a", &Hit::Root { edge: Edge::Bottom }, &narrow, 601.0, H, &m));
    }
}

mod saving_a_layout {
    use super::*;

    fn layout() -> Layout {
        let l = split_with(&one(&["a", "b", "c", "d"]), "c", "g1", Edge::Right).into_owned();
        split_with(&l, "d", "g1", Edge::Bottom).into_owned()
    }

    fn saved_json(saved: &Option<SavedLayout>) -> Value {
        serde_json::to_value(saved).unwrap()
    }

    #[test]
    fn stores_tab_positions_instead_of_session_ids_and_comes_back_the_same() {
        let l = layout();
        let saved = serialize(&l, &flatten(&l));
        assert_eq!(
            saved_json(&saved),
            json!({
                "root": { "dir": "row", "ratio": 0.5, "first": { "dir": "column", "ratio": 0.5, "first": { "tabs": [0, 1], "active": 0 }, "second": { "tabs": [2], "active": 2 } }, "second": { "tabs": [3], "active": 3 } },
                "focused": 1
            })
        );
        let ids = ["A", "B", "D", "C"];
        let back = deserialize(&saved.unwrap(), &ids.map(Some), &ids);
        assert_eq!(picture(&back), "[A] B | *[D] | [C]");
        assert_eq!(shape(&back.root), "H(V(AB,D),C)");
        check(&back);
    }

    #[test]
    fn leaves_out_sessions_that_are_not_saved_and_the_areas_that_only_held_those() {
        let l = layout();
        // b is a login tab, d has ended: neither is saved
        let saved = serialize(&l, &["a", "c"]);
        assert_eq!(
            saved_json(&saved),
            json!({ "root": { "dir": "row", "ratio": 0.5, "first": { "tabs": [0], "active": 0 }, "second": { "tabs": [1], "active": 1 } }, "focused": 0 })
        );
        assert_eq!(serialize::<&str>(&l, &[]), None);
    }

    #[test]
    fn restores_what_could_be_reopened_tabs_that_could_not_are_dropped_with_their_areas() {
        let l = layout();
        let saved = serialize(&l, &flatten(&l)).unwrap();
        // the tab in the bottom-left area was not reopened (its account is gone)
        let back = deserialize(&saved, &[Some("A"), Some("B"), None, Some("C")], &["A", "B", "C"]);
        assert_eq!(shape(&back.root), "H(AB,C)");
        check(&back);
        // nothing reopened at all
        assert_eq!(picture(&deserialize::<&str, &str>(&saved, &[], &[])), "*");
    }

    #[test]
    fn puts_reopened_tabs_the_saved_layout_does_not_mention_into_the_area_with_the_keyboard() {
        let l = layout();
        let saved = serialize(&l, &flatten(&l)).unwrap();
        let back = deserialize(&saved, &["A", "B", "D", "C"].map(Some), &["A", "B", "D", "C", "E"]);
        assert_eq!(picture(&back), "[A] B | *[D] E | [C]");
    }

    #[test]
    fn renumbers_a_saved_layout_when_entries_of_the_tab_list_were_skipped() {
        let l = layout();
        let saved = serialize(&l, &flatten(&l)).unwrap();
        // entry 1 was unreadable: the ones after it move up
        let moved = remap_saved(&saved, |i| match i {
            1 => None,
            i if i > 1 => Some(i - 1),
            i => Some(i),
        });
        assert_eq!(
            saved_json(&moved)["root"],
            json!({ "dir": "row", "ratio": 0.5, "first": { "dir": "column", "ratio": 0.5, "first": { "tabs": [0], "active": 0 }, "second": { "tabs": [1], "active": 1 } }, "second": { "tabs": [2], "active": 2 } })
        );
        assert_eq!(remap_saved(&saved, |_| None), None);
    }

    #[test]
    fn rejects_a_saved_layout_that_is_not_one() {
        assert_eq!(parse_saved_layout(&json!(null)), None);
        assert_eq!(parse_saved_layout(&json!({ "root": { "tabs": [] } })), None);
        assert_eq!(parse_saved_layout(&json!({ "root": { "tabs": [0, 0], "active": 0 } })), None);
        assert_eq!(parse_saved_layout(&json!({ "root": { "tabs": [-1], "active": 0 } })), None);
        assert_eq!(
            parse_saved_layout(&json!({ "root": { "dir": "diagonal", "first": { "tabs": [0] }, "second": { "tabs": [1] } } })),
            None
        );
        // the same tab in two areas
        assert_eq!(
            parse_saved_layout(&json!({ "root": { "dir": "row", "ratio": 0.5, "first": { "tabs": [0] }, "second": { "tabs": [0] } } })),
            None
        );
        // a bad ratio or focus is repaired rather than fatal
        let repaired = parse_saved_layout(&json!({
            "root": { "dir": "row", "ratio": 9, "first": { "tabs": [0], "active": 0 }, "second": { "tabs": [1], "active": 1 } },
            "focused": 7
        }));
        assert_eq!(
            saved_json(&repaired),
            json!({ "root": { "dir": "row", "ratio": 0.95, "first": { "tabs": [0], "active": 0 }, "second": { "tabs": [1], "active": 1 } }, "focused": 0 })
        );
        let mut deep = json!({ "tabs": [0], "active": 0 });
        for i in 1..40 {
            deep = json!({ "dir": "row", "ratio": 0.5, "first": deep, "second": { "tabs": [i], "active": i } });
        }
        assert_eq!(parse_saved_layout(&json!({ "root": deep })), None);
    }
}

mod parsing_a_layout {
    use super::*;

    fn layout() -> Layout {
        split_with(&one(&["a", "b", "c"]), "c", "g1", Edge::Right).into_owned()
    }

    #[test]
    fn accepts_a_layout_that_went_through_json() {
        let l = layout();
        let back = parse_layout(&serde_json::to_value(&l).unwrap()).unwrap();
        assert_eq!(back, l);
        // new areas made afterwards do not reuse an id
        let next = split_with(&back, "b", "g1", Edge::Bottom).into_owned();
        check(&next);
    }

    #[test]
    fn rejects_malformed_trees() {
        assert_eq!(parse_layout(&json!(null)), None);
        assert_eq!(parse_layout(&json!({ "root": { "kind": "group", "id": "g1", "tabs": ["a", "a"], "active": "a" }, "focused": "g1" })), None);
        assert_eq!(
            parse_layout(&json!({ "root": { "kind": "split", "id": "s1", "dir": "row", "ratio": 0.5, "first": { "kind": "group", "id": "x", "tabs": ["a"] }, "second": { "kind": "group", "id": "x", "tabs": ["b"] } } })),
            None
        );
        assert_eq!(parse_layout(&json!({ "root": { "kind": "group", "id": "g1", "tabs": [1, 2] } })), None);
    }

    #[test]
    fn repairs_what_can_be_repaired_a_missing_focus_a_tab_shown_that_is_not_there_empty_areas() {
        let repaired = parse_layout(&json!({
            "root": {
                "kind": "split",
                "id": "s9",
                "dir": "row",
                "ratio": 0.5,
                "first": { "kind": "group", "id": "g4", "tabs": ["a", "b"], "active": "zzz" },
                "second": { "kind": "group", "id": "g7", "tabs": [], "active": null }
            },
            "focused": "nope"
        }))
        .unwrap();
        assert_eq!(picture(&repaired), "*a [b]");
        assert!(repaired.seq >= 9);
        check(&repaired);
    }

    // ---------- not in the TypeScript tests ----------

    #[test]
    fn counts_id_lengths_in_utf16_units_and_limits_the_depth() {
        let group = |id: String| json!({ "root": { "kind": "group", "id": "g1", "tabs": [id] }, "focused": "g1" });
        assert!(parse_layout(&group("x".repeat(64))).is_some());
        assert_eq!(parse_layout(&group("x".repeat(65))), None);
        // 32 emoji are 64 units, one more is too long
        assert!(parse_layout(&group("😀".repeat(32))).is_some());
        assert_eq!(parse_layout(&group("😀".repeat(32) + "x")), None);
        assert_eq!(parse_layout(&group(String::new())), None);

        let nested = |levels: usize| {
            let mut node = json!({ "kind": "group", "id": "g0", "tabs": ["t0"] });
            for i in 1..=levels {
                node = json!({ "kind": "split", "id": format!("s{i}"), "dir": "column", "ratio": "0.4", "first": node, "second": { "kind": "group", "id": format!("g{i}"), "tabs": [format!("t{i}")] } });
            }
            json!({ "root": node, "focused": "g0", "seq": 3 })
        };
        let deepest = parse_layout(&nested(MAX_DEPTH)).unwrap();
        assert_eq!(ratio_of(&deepest.root), 0.4);
        assert_eq!(deepest.seq, MAX_DEPTH as u64);
        assert_eq!(parse_layout(&nested(MAX_DEPTH + 1)), None);
    }
}

mod panels {
    use super::*;

    #[test]
    fn reconcile_keeps_panel_tabs_that_no_session_has() {
        let l = one(&["a", "panel:usage", "b"]);
        assert!(is_panel("panel:usage") && !is_panel("a"));
        // the session list never names a panel
        assert!(same(&reconcile(&l, &refs(&["a", "b"])), &l));
        assert_eq!(picture(&reconcile(&l, &refs(&["a"]))), "*[a] panel:usage");
        // with every session gone, a shown panel stays, and so does an area that holds one
        let shown = activate(&l, "panel:usage").into_owned();
        assert_eq!(picture(&reconcile(&shown, &[])), "*[panel:usage]");
        let apart = split_with(&l, "panel:usage", "g1", Edge::Right).into_owned();
        assert_eq!(picture(&reconcile(&apart, &[])), "*[panel:usage]");
        assert_eq!(shape(&reconcile(&apart, &refs(&["b"])).root), "H(b,panel:usage)");
        // a tab that merely looks like one is still a session when it lacks the prefix
        assert_eq!(picture(&reconcile(&one(&["a", "panel-usage"]), &refs(&["a"]))), "*[a]");
    }
}

mod strip_on_the_left {
    use super::*;

    const W: f64 = 1001.0;
    const H: f64 = 600.0;

    fn metrics() -> Metrics {
        Metrics::new(1.0, 200.0, 120.0).with_strip(StripSide::Left, 160.0)
    }

    fn options() -> HitOptions {
        HitOptions::new(160.0, 24.0, 0.25).with_strip_side(StripSide::Left)
    }

    fn row() -> Layout {
        split_with(&one(&["a", "b", "c"]), "c", "g1", Edge::Right).into_owned()
    }

    #[test]
    fn an_area_needs_room_for_its_strip_beside_it() {
        assert_eq!(min_size(&one(&["a"]).root, &metrics()), Size { w: 360.0, h: 120.0 });
        // with the strip on top it is the area's own minimum, as in the TypeScript
        assert_eq!(min_size(&one(&["a"]).root, &Metrics::new(1.0, 200.0, 120.0)), Size { w: 200.0, h: 120.0 });
        let two = split_with(&one(&["a", "b"]), "b", "g1", Edge::Right).into_owned();
        assert_eq!(min_size(&two.root, &metrics()), Size { w: 721.0, h: 120.0 });
        let squeezed = set_ratio(&two, split_id(&two.root), 0.05).into_owned();
        let Geometry { groups, dividers } = layout_rects(&squeezed.root, W, H, &metrics());
        assert_eq!(groups[0].w, 360.0);
        assert!(close(dividers[0].min, 0.36));
        assert!(close(dividers[0].max, 0.64));
    }

    #[test]
    fn reads_the_strip_column_and_measures_the_sides_in_the_body() {
        let lone = one(&["a", "b"]);
        let geo = layout_rects(&lone.root, W, H, &metrics());
        let o = options();
        let hit = |x, y| hit_test(&geo, W, H, x, y, &o);
        let split = |edge| Some(Hit::Split { group_id: "g1".into(), edge });
        assert_eq!(hit(100.0, 300.0), Some(Hit::Strip { group_id: "g1".into() }));
        // the top left corner is the strip, not the top side
        assert_eq!(hit(100.0, 10.0), Some(Hit::Strip { group_id: "g1".into() }));
        assert_eq!(hit(600.0, 300.0), Some(Hit::Move { group_id: "g1".into() }));
        assert_eq!(hit(170.0, 300.0), split(Edge::Left));
        assert_eq!(hit(990.0, 300.0), split(Edge::Right));
        assert_eq!(hit(600.0, 10.0), split(Edge::Top));
        assert_eq!(hit(600.0, 590.0), split(Edge::Bottom));
    }

    #[test]
    fn with_several_areas_the_band_runs_along_the_sides_the_strips_do_not_take() {
        let row = row();
        let right = group_id(&row, "c");
        let geo = layout_rects(&row.root, W, H, &metrics());
        let o = options();
        let hit = |x, y| hit_test(&geo, W, H, x, y, &o);
        let root = |edge| Some(Hit::Root { edge });
        // the left of the window belongs to the strips
        assert_eq!(hit(10.0, 300.0), Some(Hit::Strip { group_id: "g1".into() }));
        assert_eq!(hit(995.0, 300.0), root(Edge::Right));
        assert_eq!(hit(800.0, 5.0), root(Edge::Top));
        assert_eq!(hit(800.0, 590.0), root(Edge::Bottom));
        // a corner goes to the right before the top
        assert_eq!(hit(995.0, 5.0), root(Edge::Right));
        // the right area's strip, then its body
        assert_eq!(hit(520.0, 300.0), Some(Hit::Strip { group_id: right.clone() }));
        assert_eq!(hit(700.0, 300.0), Some(Hit::Split { group_id: right, edge: Edge::Left }));
    }

    #[test]
    fn previews_and_checks_drops_in_the_body() {
        let row = row();
        let right = group_id(&row, "c");
        let m = metrics();
        let geo = layout_rects(&row.root, W, H, &m);
        let o = options();
        let rect = |x, y, w, h| Some(Rect { x, y, w, h });
        assert_eq!(preview_rect(&geo, W, H, &Hit::Move { group_id: right }, &o), rect(661.0, 0.0, 340.0, 600.0));
        let hit = Hit::Split { group_id: "g1".into(), edge: Edge::Right };
        assert_eq!(preview_rect(&geo, W, H, &hit, &o), rect(330.0, 0.0, 170.0, 600.0));
        assert_eq!(preview_rect(&geo, W, H, &Hit::Root { edge: Edge::Left }, &o), rect(0.0, 0.0, 500.5, 600.0));
        // two columns with their strips do not fit in an area 500 wide; with the strip on top they would
        let left_of_g1 = Hit::Split { group_id: "g1".into(), edge: Edge::Left };
        assert!(!can_drop(&row, "a", &left_of_g1, &geo, W, H, &m));
        assert!(can_drop(&row, "a", &left_of_g1, &geo, W, H, &Metrics::new(1.0, 200.0, 120.0)));
        assert!(can_drop(&row, "a", &Hit::Split { group_id: "g1".into(), edge: Edge::Bottom }, &geo, W, H, &m));
        assert!(!can_drop(&row, "a", &Hit::Root { edge: Edge::Left }, &geo, W, H, &m));
        assert!(can_drop(&row, "a", &Hit::Root { edge: Edge::Bottom }, &geo, W, H, &m));
    }
}
