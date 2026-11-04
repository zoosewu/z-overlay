use crate::settings::Align;

/// A screen rectangle in physical pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Top-left corner of an overlay of `size` whose `align` point sits `offset`
/// away from the same point of `area`.
pub fn place(area: Rect, align: Align, offset: [i32; 2], size: (i32, i32)) -> (i32, i32) {
    let (column, row) = align.cell();
    let x = area.x + (area.width - size.0) * column / 2 + offset[0];
    let y = area.y + (area.height - size.1) * row / 2 + offset[1];
    (x, y)
}

/// The offset that puts an overlay of `size` with its top-left corner at `position`.
pub fn offset_for(area: Rect, align: Align, position: (i32, i32), size: (i32, i32)) -> [i32; 2] {
    let (x, y) = place(area, align, [0, 0], size);
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
    fn corners_and_centre_line_up_with_the_area() {
        let size = (200, 100);
        assert_eq!(place(AREA, Align::TopLeft, [0, 0], size), (100, 50));
        assert_eq!(place(AREA, Align::BottomRight, [0, 0], size), (700, 550));
        assert_eq!(place(AREA, Align::Center, [0, 0], size), (400, 300));
        assert_eq!(place(AREA, Align::Top, [0, 0], size), (400, 50));
        assert_eq!(place(AREA, Align::Left, [0, 0], size), (100, 300));
    }

    #[test]
    fn offset_moves_away_from_the_aligned_point() {
        let size = (200, 100);
        assert_eq!(place(AREA, Align::TopLeft, [10, 20], size), (110, 70));
        assert_eq!(
            place(AREA, Align::BottomRight, [-10, -20], size),
            (690, 530)
        );
    }

    #[test]
    fn offset_for_inverts_place() {
        let size = (37, 91);
        for align in Align::ALL {
            let offset = offset_for(AREA, align, (321, 123), size);
            assert_eq!(place(AREA, align, offset, size), (321, 123), "{align:?}");
        }
    }
}
