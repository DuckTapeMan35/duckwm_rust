use std::collections::HashSet;

use crate::arrange::{Snapshot, fixed, key, node, split};
use crate::geometry::Axis;
use crate::proto;

/// A window, or a branch of them.
#[derive(Clone, Debug, PartialEq)]
pub enum Tree {
    Window(u32),
    Branch(Branch),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    pub axis: Axis,
    pub children: Vec<Tree>,
    pub ratios: Option<Vec<f64>>,
    pub key: Option<u32>,
}

impl From<u32> for Tree {
    fn from(id: u32) -> Self {
        Tree::Window(id)
    }
}

/// A row or column being put together; it becomes a tree once used.
#[derive(Clone, Debug)]
pub struct Builder(Branch);

impl Builder {
    /// Shares of the container; ignored unless there's one per child.
    pub fn ratios(mut self, r: impl Into<Vec<f64>>) -> Self {
        self.0.ratios = Some(r.into());
        self
    }

    /// A key of its own, instead of the depth-first one.
    pub fn key(mut self, k: u32) -> Self {
        self.0.key = Some(k);
        self
    }
}

impl From<Builder> for Option<Tree> {
    /// No children is nothing; one child (without a key) is just that child.
    fn from(Builder(mut b): Builder) -> Self {
        if b.children.is_empty() {
            return None;
        }
        if b.children.len() == 1 && b.key.is_none() {
            return b.children.pop();
        }
        if b.ratios
            .as_ref()
            .is_some_and(|r| r.len() != b.children.len())
        {
            b.ratios = None;
        }
        Some(Tree::Branch(b))
    }
}

fn branch<T: Into<Option<Tree>>>(axis: Axis, children: impl IntoIterator<Item = T>) -> Builder {
    let children = children.into_iter().filter_map(Into::into).collect();
    Builder(Branch {
        axis,
        children,
        ratios: None,
        key: None,
    })
}

/// Children side by side. None children are skipped.
pub fn row<T: Into<Option<Tree>>>(children: impl IntoIterator<Item = T>) -> Builder {
    branch(Axis::H, children)
}

/// Children stacked top to bottom.
pub fn col<T: Into<Option<Tree>>>(children: impl IntoIterator<Item = T>) -> Builder {
    branch(Axis::V, children)
}

/// Windows as tree leaves, for row/col: `col(windows(&s.windows))`.
pub fn windows(ids: &[u32]) -> impl Iterator<Item = Tree> + '_ {
    ids.iter().map(|&id| Tree::Window(id))
}

fn branches<'a>(t: &'a Tree, out: &mut Vec<&'a Branch>) {
    if let Tree::Branch(b) = t {
        out.push(b);
        for c in &b.children {
            branches(c, out);
        }
    }
}

/// The tree as nodes: keyed containers holding splits; the root fills the work area.
pub fn compile(s: &Snapshot, t: Option<Tree>) -> Vec<proto::NodeSpec> {
    let Some(t) = t else { return vec![] };
    let area = s.area();
    let Tree::Branch(_) = &t else {
        let Tree::Window(id) = t else { unreachable!() };
        return vec![node(id, fixed(area))];
    };

    // Keys: explicit ones as given, the rest depth-first from 0, skipping those.
    let mut all = Vec::new();
    branches(&t, &mut all);
    let taken: HashSet<u32> = all.iter().filter_map(|b| b.key).collect();
    let mut auto = (0..).filter(|k| !taken.contains(k));
    let keys: Vec<u32> = all
        .iter()
        .map(|b| b.key.unwrap_or_else(|| auto.next().unwrap()))
        .collect();
    let key_of = |b: &Branch| keys[all.iter().position(|x| std::ptr::eq(*x, b)).unwrap()];

    let mut nodes = Vec::new();
    for (i, b) in all.iter().enumerate() {
        let k = keys[i];
        let stored = s
            .ratios_of(k)
            .filter(|r| r.len() == b.children.len() && r.iter().all(|x| *x > 0.0));
        let ratios = stored.or_else(|| b.ratios.clone());
        let kids = b.children.iter().map(|c| match c {
            Tree::Branch(cb) => key(key_of(cb)),
            Tree::Window(id) => (*id).into(),
        });
        let mut cons = if i == 0 { fixed(area) } else { vec![] };
        cons.push(split(key(k), b.axis, kids, ratios.as_deref()));
        nodes.push(node(key(k), cons));
        for c in &b.children {
            if let Tree::Window(id) = c {
                nodes.push(node(*id, []));
            }
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::key;
    use crate::proto::constraint_spec::Kind as C;

    fn snapshot(windows: &[u32]) -> Snapshot {
        Snapshot::new(proto::Snapshot {
            work_area: Some(proto::Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            }),
            windows: windows
                .iter()
                .map(|&id| proto::WindowInfo { id, geometry: None })
                .collect(),
            ..Default::default()
        })
    }

    fn ratios(nodes: &[proto::NodeSpec]) -> Vec<f64> {
        nodes[0]
            .constraints
            .iter()
            .find_map(|c| match &c.kind {
                Some(C::Split(sp)) => Some(sp.ratios.clone()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn one_child_is_just_the_child() {
        assert_eq!(
            Option::<Tree>::from(row([Tree::Window(3)])),
            Some(Tree::Window(3))
        );
        assert_eq!(Option::<Tree>::from(col(Vec::<Tree>::new())), None);
        // ...unless it has a key of its own.
        let keyed = Option::<Tree>::from(row([Tree::Window(3)]).key(5));
        assert!(matches!(keyed, Some(Tree::Branch(_))));
    }

    #[test]
    fn keys_depth_first_skipping_explicit_ones() {
        let s = snapshot(&[1, 2, 3]);
        let t = row([Some(Tree::Window(1)), col(windows(&[2, 3])).into()]).key(1);
        let nodes = compile(&s, t.into());
        let keys: Vec<_> = nodes
            .iter()
            .filter_map(|n| match n.id.as_ref()?.kind {
                Some(proto::r#ref::Kind::Key(k)) => Some(k),
                _ => None,
            })
            .collect();
        assert_eq!(keys, [1, 0]); // the root keeps 1; the column gets the first free one
        let fills =
            |c: &proto::ConstraintSpec| matches!(&c.kind, Some(C::FixedWidth(w)) if w.value == 800);
        assert!(
            nodes[0].constraints.iter().any(fills),
            "the root fills the work area"
        );
    }

    #[test]
    fn stored_ratios_win_while_the_shape_holds() {
        let mut s = snapshot(&[1, 2]);
        let stored = proto::NodeSpec {
            id: Some(key(0)),
            constraints: vec![split(
                key(0),
                Axis::H,
                [1.into(), 2.into()],
                Some(&[0.7, 0.3]),
            )],
        };
        s.raw.layout = Some(proto::LayoutSpec {
            nodes: vec![stored],
        });
        assert_eq!(
            ratios(&compile(&s, row(windows(&[1, 2])).into())),
            [0.7, 0.3]
        );
        // A third window: the shape changed, back to equal shares.
        assert_eq!(
            ratios(&compile(&s, row(windows(&[1, 2, 3])).into())).len(),
            3
        );
    }
}
