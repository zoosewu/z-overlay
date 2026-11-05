use crate::settings::Align;

/// A screen rectangle in physical pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Top-left corner of an overlay of `size` whose `pivot` point sits `offset`
/// away from the `align` point of `area`. The pivot stays put when the
/// overlay changes size.
pub fn place(
    area: Rect,
    align: Align,
    pivot: Align,
    offset: [i32; 2],
    size: (i32, i32),
) -> (i32, i32) {
    let (area_column, area_row) = align.cell();
    let (pivot_column, pivot_row) = pivot.cell();
    let x = area.x + area.width * area_column / 2 - size.0 * pivot_column / 2 + offset[0];
    let y = area.y + area.height * area_row / 2 - size.1 * pivot_row / 2 + offset[1];
    (x, y)
}

/// The offset that puts an overlay of `size` with its top-left corner at `position`.
pub fn offset_for(
    area: Rect,
    align: Align,
    pivot: Align,
    position: (i32, i32),
    size: (i32, i32),
) -> [i32; 2] {
    let (x, y) = place(area, align, pivot, [0, 0], size);
    [position.0 - x, position.1 - y]
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 100,
        y: 50,
        width: 800,
        height: 600,
    };

    #[test]
    fn matching_points_line_up_the_overlay_with_the_area() {
        let size = (200, 100);
        let same = |align| place(AREA, align, align, [0, 0], size);
        assert_eq!(same(Align::TopLeft), (100, 50));
        assert_eq!(same(Align::BottomRight), (700, 550));
        assert_eq!(same(Align::Center), (400, 300));
        assert_eq!(same(Align::Top), (400, 50));
        assert_eq!(same(Align::Left), (100, 300));
    }

    #[test]
    fn the_pivot_can_differ_from_the_area_point() {
        let size = (200, 100);
        // The overlay's centre on the area's top-left corner.
        assert_eq!(
            place(AREA, Align::TopLeft, Align::Center, [0, 0], size),
            (0, 0)
        );
        // The overlay's top-left corner on the area's centre.
        assert_eq!(
            place(AREA, Align::Center, Align::TopLeft, [0, 0], size),
            (500, 350)
        );
    }

    #[test]
    fn the_pivot_stays_put_when_the_size_changes() {
        for pivot in Align::ALL {
            let (column, row) = pivot.cell();
            let point = |size: (i32, i32)| {
                let (x, y) = place(AREA, Align::Center, pivot, [7, -3], size);
                (x + size.0 * column / 2, y + size.1 * row / 2)
            };
            assert_eq!(point((40, 40)), point((300, 300)), "{pivot:?}");
        }
    }

    #[test]
    fn offset_moves_away_from_the_aligned_point() {
        let size = (200, 100);
        let same = |align, offset| place(AREA, align, align, offset, size);
        assert_eq!(same(Align::TopLeft, [10, 20]), (110, 70));
        assert_eq!(same(Align::BottomRight, [-10, -20]), (690, 530));
    }

    #[test]
    fn offset_for_inverts_place() {
        let size = (37, 91);
        for align in Align::ALL {
            for pivot in Align::ALL {
                let offset = offset_for(AREA, align, pivot, (321, 123), size);
                assert_eq!(
                    place(AREA, align, pivot, offset, size),
                    (321, 123),
                    "{align:?} {pivot:?}"
                );
            }
        }
    }
}
