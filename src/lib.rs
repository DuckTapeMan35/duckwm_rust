/// The messages generated from duckwm.proto by scripts/gen. Don't edit.
#[allow(clippy::all)]
pub mod proto {
    include!("proto/duckwm.v1.rs");
}

mod api;
pub mod arrange;
mod client;
pub mod cmd;
mod conn;
pub mod forever;
pub mod geometry;
pub mod layouts;
mod mods;
mod reply;
pub mod tree;
mod value;
mod wire;

pub use arrange::{Arranger, Snapshot};
pub use client::{Client, Error, Platform, Result, Workspace, WorkspaceState};
pub use forever::{forever, forever_at};
pub use geometry::Rect;
pub use mods::{KeyCombo, Mods, ParseComboError};
pub use proto::command::Kind as Cmd;
pub use reply::Reply;
pub use value::Value;
pub use wire::socket_path;

impl From<Cmd> for proto::Command {
    fn from(kind: Cmd) -> Self {
        proto::Command { kind: Some(kind) }
    }
}
