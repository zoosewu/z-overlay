//! The overlay editor's hierarchy: overlays in no group first, then each group
//! followed by its overlays.

use crate::settings::ManagedApplication;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Row {
    /// The heading of the overlays that are in no group.
    NoGroup,
    /// A group, by index.
    Group(usize),
    /// An overlay, by index.
    Overlay(usize),
}

/// The group an overlay is shown under; a missing group counts as none.
fn group_of(app: &ManagedApplication, overlay: usize) -> Option<u32> {
    app.overlays[overlay]
        .group
        .filter(|id| app.group(*id).is_some())
}

pub fn rows(app: &ManagedApplication) -> Vec<Row> {
    let overlays_in = |group: Option<u32>| {
        (0..app.overlays.len())
            .filter(move |index| group_of(app, *index) == group)
            .map(Row::Overlay)
    };
    let mut rows = vec![Row::NoGroup];
    rows.extend(overlays_in(None));
    for (index, group) in app.groups.iter().enumerate() {
        rows.push(Row::Group(index));
        rows.extend(overlays_in(Some(group.id)));
    }
    rows
}

/// Where an overlay dropped on the line above row `target` (or below the last
/// row) goes: the overlay to sit before, if any, and its group. It joins the
/// section of the row above the line, so dropping just under a group's
/// heading puts it at the top of that group.
pub fn drop_place(
    app: &ManagedApplication,
    rows: &[Row],
    target: usize,
) -> (Option<usize>, Option<u32>) {
    // Nothing goes above the "No group" heading.
    let target = target.clamp(1, rows.len());
    let group = rows[..target]
        .iter()
        .rev()
        .find_map(|row| match row {
            Row::NoGroup => Some(None),
            Row::Group(index) => Some(Some(app.groups[*index].id)),
            Row::Overlay(_) => None,
        })
        .flatten();
    let before = match rows.get(target) {
        Some(Row::Overlay(index)) => Some(*index),
        // At the end of a section: after its last overlay.
        _ => (0..app.overlays.len())
            .rev()
            .find(|index| group_of(app, *index) == group)
            .map(|last| last + 1)
            .filter(|next| *next < app.overlays.len()),
    };
    (before, group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Overlay;

    /// Overlays a (none), b (group 1), c (none), d (group 2); groups 1 and 2.
    fn app() -> ManagedApplication {
        let mut app = ManagedApplication::new("a.exe".into(), None);
        app.add_group("one".into());
        app.add_group("two".into());
        for (name, group) in [("a", None), ("b", Some(1)), ("c", None), ("d", Some(2))] {
            let mut overlay = Overlay::text(name.into(), name.into());
            overlay.group = group;
            app.overlays.push(overlay);
        }
        app
    }

    #[test]
    fn rows_list_ungrouped_overlays_then_each_group() {
        use Row::*;
        let mut app = app();
        assert_eq!(
            rows(&app),
            [
                NoGroup,
                Overlay(0),
                Overlay(2),
                Group(0),
                Overlay(1),
                Group(1),
                Overlay(3)
            ]
        );
        // An overlay whose group was removed shows with the ungrouped ones.
        app.overlays[3].group = Some(9);
        assert_eq!(
            rows(&app),
            [
                NoGroup,
                Overlay(0),
                Overlay(2),
                Overlay(3),
                Group(0),
                Overlay(1),
                Group(1)
            ]
        );
    }

    #[test]
    fn a_drop_joins_the_section_above_the_line() {
        let app = app();
        let rows = rows(&app);
        // rows: 0 NoGroup, 1 a, 2 c, 3 Group one, 4 b, 5 Group two, 6 d
        assert_eq!(drop_place(&app, &rows, 1), (Some(0), None));
        assert_eq!(drop_place(&app, &rows, 2), (Some(2), None));
        // Under the last ungrouped overlay: after c, still in no group.
        assert_eq!(drop_place(&app, &rows, 3), (Some(3), None));
        // Just under a heading: top of that group.
        assert_eq!(drop_place(&app, &rows, 4), (Some(1), Some(1)));
        // End of group one: after b.
        assert_eq!(drop_place(&app, &rows, 5), (Some(2), Some(1)));
        // Below everything: end of group two, which is the end of the list.
        assert_eq!(drop_place(&app, &rows, 7), (None, Some(2)));
        assert_eq!(drop_place(&app, &rows, 99), (None, Some(2)));
        // Above the first heading counts as no group.
        assert_eq!(drop_place(&app, &rows, 0), (Some(0), None));
    }

    #[test]
    fn a_drop_into_an_empty_group_goes_last() {
        let mut app = app();
        app.add_group("empty".into());
        let rows = rows(&app);
        assert_eq!(rows.last(), Some(&Row::Group(2)));
        assert_eq!(drop_place(&app, &rows, rows.len()), (None, Some(3)));
    }

    #[test]
    fn dropping_moves_the_overlay_where_the_line_was() {
        let mut app = app();
        // Drag a (index 0) to the top of group two (rows index 6 is d).
        let rows = rows(&app);
        let (before, group) = drop_place(&app, &rows, 6);
        let index = app.move_overlay(0, before, group);
        let names: Vec<_> = app.overlays.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["b", "c", "a", "d"]);
        assert_eq!((index, app.overlays[index].group), (2, Some(2)));
    }
}
