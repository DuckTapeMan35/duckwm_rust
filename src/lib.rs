/// The messages generated from duckwm.proto by scripts/gen. Don't edit.
#[allow(clippy::all)]
pub mod proto {
    include!("proto/duckwm.v1.rs");
}

mod api;
mod client;
mod mods;
mod value;
mod wire;

pub use client::{Client, Error, Result, Workspace, WorkspaceState};
pub use mods::{KeyCombo, Mods, ParseComboError};
pub use proto::command::Kind as Cmd;
pub use value::Value;
pub use wire::socket_path;

impl From<Cmd> for proto::Command {
    fn from(kind: Cmd) -> Self {
        proto::Command { kind: Some(kind) }
    }
}
