use crate::arrange::{Arranger, Snapshot, reply, track};
use crate::tree::{Tree, col, row, windows};
use crate::value::Value;

/// Columns of near-equal windows: two side by side, then as square as possible.
pub fn fair() -> Arranger {
    Arranger::new("fair", |s: Snapshot| {
        let n = s.windows.len();
        let rows = if n == 2 {
            1
        } else {
            ((n as f64).sqrt().ceil() as usize).max(1)
        };
        let columns = s
            .windows
            .chunks(rows)
            .map(|c| Option::<Tree>::from(col(windows(c))));
        reply(&s, row(columns), None)
    })
}

/// Master and stack: the first window on the left, the rest stacked on the right.
pub fn tile() -> Arranger {
    Arranger::new("tile", |s: Snapshot| {
        let old = s.state.as_ref().and_then(Value::as_ids).unwrap_or_default();
        let order = track(&s, &old);
        let (master, stack) = match order.split_first() {
            Some((m, rest)) => (Some(Tree::Window(*m)), rest),
            None => (None, &[][..]),
        };
        let layout = row([master, col(windows(stack)).into()]).ratios([0.55, 0.45]);
        reply(&s, layout, Some(order.into()))
    })
}
