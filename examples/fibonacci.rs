//! A golden spiral, as nested splits: resizable, and resizes stick.

use duckwm::arrange::{Arranger, Snapshot, reply, track};
use duckwm::tree::{Tree, col, row};
use duckwm::{Client, Value};

fn spiral(ws: &[u32], i: usize) -> Option<Tree> {
    let (&first, rest) = ws.split_first()?;
    if rest.is_empty() {
        return Some(Tree::Window(first));
    }
    let rest = spiral(rest, i + 1);
    let split = if i.is_multiple_of(2) { row } else { col };
    let b = if i % 4 < 2 {
        split([Some(Tree::Window(first)), rest]).ratios([0.618, 0.382]) // window on the left / top
    } else {
        split([rest, Some(Tree::Window(first))]).ratios([0.382, 0.618]) // on the right / bottom
    };
    b.into()
}

pub fn fibonacci() -> Arranger {
    Arranger::new("fibonacci", |s: Snapshot| {
        let old = s.state.as_ref().and_then(Value::as_ids).unwrap_or_default();
        let order = track(&s, &old);
        reply(&s, spiral(&order, 0), Some(order.into()))
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> duckwm::Result<()> {
    let wm = Client::connect().await?;
    wm.set_arranger(&fibonacci()).await?;
    wm.closed().await;
    Ok(())
}
