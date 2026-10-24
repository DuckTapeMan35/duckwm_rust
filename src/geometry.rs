use crate::proto;

pub use proto::Axis;

/// A rectangle: position, then size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// How much to cut: a fraction of the length, or pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Amount {
    Frac(f64),
    Px(u32),
}

impl From<f64> for Amount {
    fn from(f: f64) -> Self {
        Amount::Frac(f)
    }
}

impl From<u32> for Amount {
    fn from(px: u32) -> Self {
        Amount::Px(px)
    }
}

impl Amount {
    fn of(self, total: u32) -> u32 {
        let n = match self {
            Amount::Frac(f) => (total as f64 * f).round().max(0.0) as u32,
            Amount::Px(px) => px,
        };
        n.min(total)
    }
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Take a piece off one side: (piece, what's left).
    pub fn cut(self, side: Side, amount: impl Into<Amount>) -> (Rect, Rect) {
        let Rect {
            x,
            y,
            width: w,
            height: h,
        } = self;
        let amount = amount.into();
        match side {
            Side::Left => {
                let p = amount.of(w);
                (Rect::new(x, y, p, h), Rect::new(x + p as i32, y, w - p, h))
            }
            Side::Right => {
                let p = amount.of(w);
                (
                    Rect::new(x + (w - p) as i32, y, p, h),
                    Rect::new(x, y, w - p, h),
                )
            }
            Side::Top => {
                let p = amount.of(h);
                (Rect::new(x, y, w, p), Rect::new(x, y + p as i32, w, h - p))
            }
            Side::Bottom => {
                let p = amount.of(h);
                (
                    Rect::new(x, y + (h - p) as i32, w, p),
                    Rect::new(x, y, w, h - p),
                )
            }
        }
    }

    /// Side by side (h) or stacked (v), by ratios. The pieces cover the
    /// rectangle exactly: rounding goes to the edges between them.
    pub fn split(self, axis: Axis, ratios: &[f64]) -> Vec<Rect> {
        let total: f64 = ratios.iter().sum();
        let length = if axis == Axis::H {
            self.width
        } else {
            self.height
        };
        let (mut acc, mut prev, mut out) = (0.0, 0u32, Vec::with_capacity(ratios.len()));
        for r in ratios {
            acc += r;
            let end = (length as f64 * acc / total).round() as u32;
            out.push(match axis {
                Axis::H => Rect::new(self.x + prev as i32, self.y, end - prev, self.height),
                Axis::V => Rect::new(self.x, self.y + prev as i32, self.width, end - prev),
            });
            prev = end;
        }
        out
    }

    /// n equal parts.
    pub fn split_n(self, axis: Axis, n: usize) -> Vec<Rect> {
        self.split(axis, &vec![1.0; n])
    }

    /// rows x cols cells, row by row.
    pub fn grid(self, rows: usize, cols: usize) -> Vec<Rect> {
        self.split_n(Axis::V, rows)
            .into_iter()
            .flat_map(|r| r.split_n(Axis::H, cols))
            .collect()
    }
}

impl From<proto::Rect> for Rect {
    fn from(r: proto::Rect) -> Self {
        Rect::new(r.x, r.y, r.width, r.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_covers_exactly() {
        let parts = Rect::new(0, 0, 100, 50).split_n(Axis::H, 3);
        assert_eq!(parts.iter().map(|r| r.width).sum::<u32>(), 100);
        assert_eq!(parts[0], Rect::new(0, 0, 33, 50));
        assert_eq!(parts[2].x + parts[2].width as i32, 100);
    }

    #[test]
    fn cut_sides() {
        let r = Rect::new(10, 10, 100, 100);
        assert_eq!(
            r.cut(Side::Left, 0.25),
            (Rect::new(10, 10, 25, 100), Rect::new(35, 10, 75, 100))
        );
        assert_eq!(r.cut(Side::Bottom, 30u32).0, Rect::new(10, 80, 100, 30));
        assert_eq!(r.cut(Side::Top, 500u32).1.height, 0);
    }

    #[test]
    fn grid_row_by_row() {
        let g = Rect::new(0, 0, 200, 100).grid(2, 2);
        assert_eq!(g[1], Rect::new(100, 0, 100, 50));
        assert_eq!(g[2], Rect::new(0, 50, 100, 50));
    }
}
