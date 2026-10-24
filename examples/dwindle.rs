//! Dwindle: each new window splits the focused one, alternating direction, on
//! the side of the cursor. Splits keep keys of their own, stored in state, so a
//! resized split keeps its ratios while the tree changes around it.

use std::collections::HashSet;

use duckwm::arrange::{Arranger, Snapshot, reply};
use duckwm::geometry::Axis;
use duckwm::tree::{self, Tree};
use duckwm::{Client, Value};

/// A window, or a split with its key: the arranger's own tree, kept in state.
#[derive(Clone, Debug)]
enum T {
    Win(u32),
    Split(u32, Axis, Box<T>, Box<T>),
}

#[derive(Debug)]
struct State {
    tree: Option<T>,
    h: bool,
    key: u32,
}

// ---- state <-> Value: [key, "h"|"v", a, b] or a window id ----
fn to_value(t: &T) -> Value {
    match t {
        T::Win(w) => (*w).into(),
        T::Split(k, axis, a, b) => {
            let axis = if *axis == Axis::H { "h" } else { "v" };
            Value::List(vec![(*k).into(), axis.into(), to_value(a), to_value(b)])
        }
    }
}

fn from_value(v: &Value) -> Option<T> {
    if let Some(w) = v.as_int() {
        return Some(T::Win(w as u32));
    }
    let [k, axis, a, b] = v.as_list()? else {
        return None;
    };
    let axis = if axis.as_str()? == "h" {
        Axis::H
    } else {
        Axis::V
    };
    Some(T::Split(
        k.as_int()? as u32,
        axis,
        Box::new(from_value(a)?),
        Box::new(from_value(b)?),
    ))
}

impl State {
    fn load(v: Option<&Value>) -> Self {
        let get = |k| v.and_then(|v| v.get(k));
        State {
            tree: get("tree").and_then(from_value),
            h: get("h").and_then(Value::as_bool).unwrap_or(true),
            key: get("key").and_then(Value::as_int).map_or(1, |k| k as u32),
        }
    }

    fn save(&self) -> Value {
        Value::Map(vec![
            (
                "tree".into(),
                self.tree.as_ref().map_or(Value::Nil, to_value),
            ),
            ("h".into(), self.h.into()),
            ("key".into(), self.key.into()),
        ])
    }
}

// ---- the tree ----
fn leaves(t: &T, out: &mut Vec<u32>) {
    match t {
        T::Win(w) => out.push(*w),
        T::Split(_, _, a, b) => {
            leaves(a, out);
            leaves(b, out);
        }
    }
}

/// t without window w: its split collapses into the sibling.
fn remove(t: T, w: u32) -> Option<T> {
    match t {
        T::Win(x) => (x != w).then_some(T::Win(x)),
        T::Split(k, axis, a, b) => match (remove(*a, w), remove(*b, w)) {
            (None, b) => b,
            (a, None) => a,
            (Some(a), Some(b)) => Some(T::Split(k, axis, Box::new(a), Box::new(b))),
        },
    }
}

fn replace(t: T, target: u32, new: &T) -> T {
    match t {
        T::Win(x) if x == target => new.clone(),
        T::Win(x) => T::Win(x),
        T::Split(k, axis, a, b) => T::Split(
            k,
            axis,
            Box::new(replace(*a, target, new)),
            Box::new(replace(*b, target, new)),
        ),
    }
}

fn area(s: &Snapshot, w: u32) -> u64 {
    s.geometry(w)
        .map_or(0, |g| g.width as u64 * g.height as u64)
}

/// Split the focused window (or the largest, if it's tiny or gone) for w.
fn insert(s: &Snapshot, st: &mut State, w: u32) {
    let Some(t) = st.tree.take() else {
        st.tree = Some(T::Win(w));
        return;
    };
    let mut ls = Vec::new();
    leaves(&t, &mut ls);
    let mut target = *ls.iter().max_by_key(|x| area(s, **x)).unwrap();
    if let Some(p) = s.prev_id.filter(|p| ls.contains(p))
        && s.geometry(p)
            .is_none_or(|g| g.width >= 20 && g.height >= 20)
    {
        target = p;
    }
    let (dx, dy) = s.cursor; // relative to the focused window
    let first = (if st.h { dx } else { dy }) < 0;
    let (a, b) = if first {
        (T::Win(w), T::Win(target))
    } else {
        (T::Win(target), T::Win(w))
    };
    let axis = if st.h { Axis::H } else { Axis::V };
    let split = T::Split(st.key, axis, Box::new(a), Box::new(b));
    st.tree = Some(replace(t, target, &split));
    st.h = !st.h;
    st.key += 1;
}

fn build(t: &T) -> Tree {
    match t {
        T::Win(w) => Tree::Window(*w),
        T::Split(k, axis, a, b) => {
            let split = if *axis == Axis::H {
                tree::row
            } else {
                tree::col
            };
            Option::<Tree>::from(split([build(a), build(b)]).key(*k)).unwrap()
        }
    }
}

pub fn dwindle() -> Arranger {
    Arranger::new("dwindle", |s: Snapshot| {
        let mut st = State::load(s.state.as_ref());
        let present: HashSet<u32> = s.windows.iter().copied().collect();
        let mut gone = Vec::new();
        if let Some(t) = &st.tree {
            leaves(t, &mut gone);
        }
        for w in gone.into_iter().filter(|w| !present.contains(w)) {
            st.tree = st.tree.take().and_then(|t| remove(t, w));
        }
        let mut have = Vec::new();
        if let Some(t) = &st.tree {
            leaves(t, &mut have);
        }
        for &w in &s.windows {
            if !have.contains(&w) {
                insert(&s, &mut st, w);
                have.push(w);
            }
        }
        let layout = st.tree.as_ref().map(build);
        reply(&s, layout, Some(st.save()))
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> duckwm::Result<()> {
    let wm = Client::connect().await?;
    wm.set_arranger(&dwindle()).await?;
    wm.closed().await;
    Ok(())
}
