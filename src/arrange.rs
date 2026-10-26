use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::geometry::{Axis, Rect};
use crate::proto::{self, constraint_spec::Kind as C};
use crate::reply::Reply;
use crate::tree::{self, Tree};
use crate::value::Value;
use crate::{Cmd, cmd};

pub use proto::ArrangerEvent as Event;

/// What an arranger sees: the event, the workspace's tiled windows, and its
/// layout and state as duckwm last stored them.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub raw: proto::Snapshot,
    pub event: Event,
    /// The window the event is about.
    pub id: u32,
    /// What was focused before the event.
    pub prev_id: Option<u32>,
    pub focused: Option<u32>,
    pub work_area: Rect,
    /// The cursor relative to the focused window.
    pub cursor: (i32, i32),
    pub state: Option<Value>,
    /// The tiled windows, in the order they appeared; not the one being unmapped.
    pub windows: Vec<u32>,
    geometry: HashMap<u32, Rect>,
}

impl Snapshot {
    pub fn new(raw: proto::Snapshot) -> Self {
        let event = Event::try_from(raw.event).unwrap_or(Event::Map);
        let gone = (event == Event::Unmap).then_some(raw.id);
        Self {
            event,
            id: raw.id,
            prev_id: raw.prev_id,
            focused: raw.focused,
            work_area: raw.work_area.map(Rect::from).unwrap_or_default(),
            cursor: (raw.cursor_dx, raw.cursor_dy),
            state: raw.state.clone().map(Value::from),
            windows: raw
                .windows
                .iter()
                .map(|w| w.id)
                .filter(|id| Some(*id) != gone)
                .collect(),
            geometry: raw
                .windows
                .iter()
                .map(|w| (w.id, w.geometry.map(Rect::from).unwrap_or_default()))
                .collect(),
            raw,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.work_area.width, self.work_area.height)
    }

    /// The work area, relative to itself: what window constraints are measured in.
    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.work_area.width, self.work_area.height)
    }

    pub fn geometry(&self, id: u32) -> Option<Rect> {
        self.geometry.get(&id).copied()
    }

    /// The split ratios keyed container k has now: resizes end up here.
    pub fn ratios_of(&self, k: u32) -> Option<Vec<f64>> {
        let layout = self.raw.layout.as_ref()?;
        layout
            .nodes
            .iter()
            .filter(|n| {
                n.id.as_ref()
                    .is_some_and(|r| r.kind == Some(proto::r#ref::Kind::Key(k)))
            })
            .flat_map(|n| &n.constraints)
            .find_map(|c| match &c.kind {
                Some(C::Split(s)) => Some(s.ratios.clone()),
                _ => None,
            })
    }
}

// ---- refs and nodes ----
impl From<u32> for proto::Ref {
    /// A window (or any node) by id.
    fn from(id: u32) -> Self {
        proto::Ref {
            kind: Some(proto::r#ref::Kind::Id(id)),
        }
    }
}

/// A container, by key.
pub fn key(k: u32) -> proto::Ref {
    proto::Ref {
        kind: Some(proto::r#ref::Kind::Key(k)),
    }
}

pub fn node(
    r: impl Into<proto::Ref>,
    constraints: impl IntoIterator<Item = proto::ConstraintSpec>,
) -> proto::NodeSpec {
    proto::NodeSpec {
        id: Some(r.into()),
        constraints: constraints.into_iter().collect(),
    }
}

fn spec(kind: C) -> proto::ConstraintSpec {
    proto::ConstraintSpec { kind: Some(kind) }
}

// ---- constraints ----
/// Position and size, relative to the work area.
pub fn fixed(r: Rect) -> Vec<proto::ConstraintSpec> {
    vec![
        spec(C::FixedX(proto::ConstraintSpecFixedX { value: r.x })),
        spec(C::FixedY(proto::ConstraintSpecFixedY { value: r.y })),
        spec(C::FixedWidth(proto::ConstraintSpecFixedWidth {
            value: r.width,
        })),
        spec(C::FixedHeight(proto::ConstraintSpecFixedHeight {
            value: r.height,
        })),
    ]
}

/// A node placed at a rectangle of the work area.
pub fn at(r: impl Into<proto::Ref>, rect: Rect) -> proto::NodeSpec {
    node(r, fixed(rect))
}

pub fn grid_cell(
    container: impl Into<proto::Ref>,
    col: u32,
    row: u32,
    cols: u32,
    rows: u32,
) -> proto::ConstraintSpec {
    spec(C::GridCell(proto::ConstraintSpecGridCell {
        col,
        row,
        cols,
        rows,
        container: Some(container.into()),
    }))
}

/// An exact rectangle inside container, in its own coordinates.
pub fn grid_cell_abs(container: impl Into<proto::Ref>, r: Rect) -> proto::ConstraintSpec {
    spec(C::GridCellAbs(proto::ConstraintSpecGridCellAbs {
        x: r.x,
        y: r.y,
        w: r.width,
        h: r.height,
        container: Some(container.into()),
    }))
}

/// The whole of container.
pub fn fill(container: impl Into<proto::Ref>) -> proto::ConstraintSpec {
    grid_cell(container, 0, 0, 1, 1)
}

/// Divide container among children, side by side (h) or stacked (v); equal shares without ratios.
pub fn split(
    container: impl Into<proto::Ref>,
    axis: Axis,
    children: impl IntoIterator<Item = proto::Ref>,
    ratios: Option<&[f64]>,
) -> proto::ConstraintSpec {
    let children: Vec<proto::Ref> = children.into_iter().collect();
    let ratios = match ratios {
        Some(r) => r.to_vec(),
        None => vec![1.0 / children.len().max(1) as f64; children.len()],
    };
    spec(C::Split(proto::ConstraintSpecSplit {
        container: Some(container.into()),
        axis: axis as i32,
        ratios,
        children,
    }))
}

// ---- order and replies ----
/// Last time's window order, minus closed windows, plus new ones: a mapped
/// window goes after the one that was focused, any others at the end.
///
/// duckwm calls an arranger with pre_map before map, and the new window is
/// already in that snapshot: it's placed then, and map finds it in place.
pub fn track(s: &Snapshot, old: &[u32]) -> Vec<u32> {
    let present: HashSet<u32> = s.windows.iter().copied().collect();
    let mut out: Vec<u32> = old
        .iter()
        .copied()
        .filter(|w| present.contains(w))
        .collect();
    for &w in &s.windows {
        if out.contains(&w) {
            continue;
        }
        let mapping = matches!(s.event, Event::PreMap | Event::Map);
        let after_focused = (mapping && w == s.id)
            .then(|| s.prev_id.and_then(|p| out.iter().position(|x| *x == p)))
            .flatten();
        out.insert(after_focused.map_or(out.len(), |i| i + 1), w);
    }
    out
}

/// A layout: nodes, or a tree of rows and columns (see [`crate::tree`]).
pub enum Layout {
    Nodes(Vec<proto::NodeSpec>),
    Tree(Option<Tree>),
}

impl From<Vec<proto::NodeSpec>> for Layout {
    fn from(n: Vec<proto::NodeSpec>) -> Self {
        Layout::Nodes(n)
    }
}

impl From<Option<Tree>> for Layout {
    fn from(t: Option<Tree>) -> Self {
        Layout::Tree(t)
    }
}

impl From<Tree> for Layout {
    fn from(t: Tree) -> Self {
        Layout::Tree(Some(t))
    }
}

impl From<tree::Builder> for Layout {
    fn from(b: tree::Builder) -> Self {
        Layout::Tree(b.into())
    }
}

/// The whole layout and the state to keep (None leaves it alone); a newly
/// mapped window gets focus.
pub fn reply(s: &Snapshot, layout: impl Into<Layout>, state: Option<Value>) -> Reply {
    let nodes = match layout.into() {
        Layout::Nodes(n) => n,
        Layout::Tree(t) => tree::compile(s, t),
    };
    let commands: Vec<Cmd> = if s.event == Event::Map {
        vec![cmd::focus(s.id)]
    } else {
        vec![]
    };
    Reply {
        commands,
        layout: Some(proto::LayoutSpec { nodes }),
        state,
        made: None,
    }
}

// ---- arrangers ----
/// A layout function and its name. Bind it to a key, or make it the default.
#[derive(Clone)]
pub struct Arranger {
    name: String,
    f: Arc<dyn Fn(Snapshot) -> Reply + Send + Sync>,
}

impl Arranger {
    pub fn new(
        name: impl Into<String>,
        f: impl Fn(Snapshot) -> Reply + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            f: Arc::new(f),
        }
    }

    /// What duckwm shows for it (get_arranger_name).
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn call(&self, s: Snapshot) -> Reply {
        (self.f)(s)
    }
}

impl std::fmt::Debug for Arranger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<arranger {}>", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(event: Event, id: u32, prev: Option<u32>, windows: &[u32]) -> Snapshot {
        Snapshot::new(proto::Snapshot {
            event: event as i32,
            id,
            prev_id: prev,
            windows: windows
                .iter()
                .map(|&id| proto::WindowInfo { id, geometry: None })
                .collect(),
            ..Default::default()
        })
    }

    #[test]
    fn track_puts_a_new_window_after_the_focused_one() {
        let s = snap(Event::Map, 9, Some(1), &[1, 2, 9]);
        assert_eq!(track(&s, &[1, 2]), [1, 9, 2]);
    }

    #[test]
    fn track_drops_closed_windows() {
        // An unmapping window isn't in s.windows any more.
        let s = snap(Event::Unmap, 2, Some(2), &[1, 2, 3]);
        assert_eq!(s.windows, [1, 3]);
        assert_eq!(track(&s, &[1, 2, 3]), [1, 3]);
    }

    #[test]
    fn track_places_a_new_window_at_pre_map() {
        // duckwm calls pre_map, then map, with the new window in both.
        let placed = track(&snap(Event::PreMap, 9, Some(1), &[1, 2, 9]), &[1, 2]);
        assert_eq!(placed, [1, 9, 2]);
        assert_eq!(
            track(&snap(Event::Map, 9, Some(1), &[1, 2, 9]), &placed),
            [1, 9, 2]
        );
    }

    #[test]
    fn reply_focuses_a_mapped_window() {
        let s = snap(Event::Map, 9, None, &[9]);
        let r = reply(&s, vec![at(9, s.area())], None);
        assert_eq!(r.commands, [cmd::focus(9)]);
        assert!(
            reply(&snap(Event::Focus, 9, None, &[9]), Vec::new(), None)
                .commands
                .is_empty()
        );
    }
}
