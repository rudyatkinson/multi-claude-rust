//! The order of the tabs in a strip (`src/shared/tabOrder.ts`): `reorder` and `drop_target`.

use std::borrow::Cow;

/// Moves `id` in front of `before_id`, or to the end when `before_id` is None. The anchor is an id
/// rather than an index because the order can change between a drag starting and its drop.
/// Returns the same slice (`Cow::Borrowed`) when nothing moves.
pub fn reorder<'a>(order: &'a [String], id: &str, before_id: Option<&str>) -> Cow<'a, [String]> {
    if !order.iter().any(|x| x == id) || before_id == Some(id) {
        return Cow::Borrowed(order);
    }
    let mut rest: Vec<String> = order.iter().filter(|x| *x != id).cloned().collect();
    let at = match before_id {
        None => Some(rest.len()),
        Some(before) => rest.iter().position(|x| x == before),
    };
    // an anchor that is gone by now: leave the order alone
    let Some(at) = at else { return Cow::Borrowed(order) };
    rest.insert(at, id.to_string());
    if rest.iter().zip(order).all(|(x, y)| x == y) {
        Cow::Borrowed(order)
    } else {
        Cow::Owned(rest)
    }
}

/// A tab of a strip, as laid out on screen. For a vertical strip pass its top and bottom (and the
/// pointer's `y` to [`drop_target`]).
#[derive(Debug, Clone, PartialEq)]
pub struct TabRect {
    pub id: String,
    pub left: f64,
    pub right: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropTarget {
    /// The tab the indicator is drawn on.
    pub id: String,
    pub side: Side,
    /// What [`reorder`] takes: the tab to land in front of, or None for the end.
    pub before_id: Option<String>,
}

/// Where a tab dragged to horizontal position `x` would land. `rects` are the tabs in order.
/// Left of a tab's middle means in front of it; past the last tab means the end.
pub fn drop_target(rects: &[TabRect], x: f64) -> Option<DropTarget> {
    let last = rects.last()?;
    for rect in rects {
        if x < (rect.left + rect.right) / 2.0 {
            return Some(DropTarget { id: rect.id.clone(), side: Side::Before, before_id: Some(rect.id.clone()) });
        }
    }
    Some(DropTarget { id: last.id.clone(), side: Side::After, before_id: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[allow(clippy::ptr_arg)] // the variant is what is asked: a slice cannot tell
    fn same(result: &Cow<'_, [String]>, order: &[String]) -> bool {
        matches!(result, Cow::Borrowed(x) if std::ptr::eq(*x, order))
    }

    mod reorder {
        use super::*;

        #[test]
        fn moves_a_tab_in_front_of_another_one_or_to_the_end() {
            let order = ids(&["a", "b", "c", "d"]);
            assert_eq!(&*reorder(&order, "d", Some("a")), ids(&["d", "a", "b", "c"]).as_slice());
            assert_eq!(&*reorder(&order, "a", Some("d")), ids(&["b", "c", "a", "d"]).as_slice());
            assert_eq!(&*reorder(&order, "a", None), ids(&["b", "c", "d", "a"]).as_slice());
        }

        #[test]
        fn returns_the_same_array_when_nothing_moves() {
            let order = ids(&["a", "b", "c", "d"]);
            assert!(same(&reorder(&order, "b", Some("c")), &order));
            assert!(same(&reorder(&order, "d", None), &order));
            assert!(same(&reorder(&order, "b", Some("b")), &order));
        }

        #[test]
        fn ignores_a_tab_or_an_anchor_that_no_longer_exists() {
            let order = ids(&["a", "b", "c", "d"]);
            assert!(same(&reorder(&order, "x", Some("a")), &order));
            assert!(same(&reorder(&order, "a", Some("x")), &order));
        }
    }

    mod drop_target {
        use super::*;

        fn rects() -> Vec<TabRect> {
            vec![
                TabRect { id: "a".into(), left: 0.0, right: 100.0 },
                TabRect { id: "b".into(), left: 100.0, right: 300.0 },
                TabRect { id: "c".into(), left: 300.0, right: 400.0 },
            ]
        }

        fn before(id: &str) -> Option<DropTarget> {
            Some(DropTarget { id: id.into(), side: Side::Before, before_id: Some(id.into()) })
        }

        #[test]
        fn lands_in_front_of_the_tab_whose_left_half_is_under_the_pointer() {
            assert_eq!(drop_target(&rects(), 10.0), before("a"));
            assert_eq!(drop_target(&rects(), 60.0), before("b"));
            assert_eq!(drop_target(&rects(), 201.0), before("c"));
        }

        #[test]
        fn lands_at_the_end_past_the_middle_of_the_last_tab_also_in_the_empty_space_behind_it() {
            let end = Some(DropTarget { id: "c".into(), side: Side::After, before_id: None });
            assert_eq!(drop_target(&rects(), 351.0), end);
            assert_eq!(drop_target(&rects(), 900.0), end);
            assert_eq!(drop_target(&[], 5.0), None);
        }
    }
}
