use std::fmt;
use std::io;
use std::path::Path;

use crate::Cmd;
use crate::mods::{KeyCombo, Mods};
use crate::proto::{self, Empty, client_message, server_message};
use crate::value::Value;
use crate::wire::{Connection, socket_path};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// duckwm closed the connection (it quit, restarted, or dropped us).
    Closed,
    /// duckwm refused the batch; nothing in it took effect.
    Rejected(String),
    /// A result wasn't the shape a typed method expected.
    Unexpected(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "duckwm socket: {e}"),
            Error::Closed => f.write_str("duckwm closed the connection"),
            Error::Rejected(why) => write!(f, "duckwm refused: {why}"),
            Error::Unexpected(what) => write!(f, "unexpected reply from duckwm: {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset => Error::Closed,
            _ => Error::Io(e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A workspace's state, from get_workspace_states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkspaceState {
    /// The one you're on.
    Focused,
    /// Has something in it.
    Active,
    /// Empty.
    Inactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Workspace {
    /// 1-based, at the current level.
    pub position: u32,
    pub state: WorkspaceState,
}

/// A connection to duckwm. Blocking: from async code, call it in spawn_blocking.
pub struct Client {
    conn: Connection,
}

impl Client {
    /// Connect at [`socket_path`].
    pub fn connect() -> Result<Self> {
        Self::connect_to(socket_path())
    }

    /// Connect at a given path, e.g. another user's `/run/user/<uid>/duckwm.sock`.
    pub fn connect_to(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            conn: Connection::open(path.as_ref())?,
        })
    }

    /// Run the commands as one transaction; one result per command.
    pub fn run(&mut self, cmds: impl IntoIterator<Item = Cmd>) -> Result<Vec<Value>> {
        let request = proto::Request {
            commands: cmds.into_iter().map(Into::into).collect(),
        };
        self.conn.send(&proto::ClientMessage {
            kind: Some(client_message::Kind::Request(request)),
        })?;
        loop {
            match self.conn.recv()?.kind {
                Some(server_message::Kind::Response(r)) => {
                    if !r.error.is_empty() {
                        return Err(Error::Rejected(r.error));
                    }
                    return Ok(r.results.into_iter().map(Value::from).collect());
                }
                // This client registers no callbacks, so any call is a mistake.
                Some(server_message::Kind::Call(_)) => self.conn.send(&proto::ClientMessage {
                    kind: Some(client_message::Kind::Ret(proto::Return {
                        err: Some("this client has no callbacks".into()),
                        ..Default::default()
                    })),
                })?,
                None => return Err(Error::Unexpected("empty message")),
            }
        }
    }

    /// Run one command and return its result.
    pub fn run_one(&mut self, cmd: Cmd) -> Result<Value> {
        self.run([cmd])?.pop().ok_or(Error::Unexpected("no result"))
    }

    /// Every bound key combination, once each.
    pub fn get_keybinds(&mut self) -> Result<Vec<KeyCombo>> {
        let v = self.run_one(Cmd::GetKeybinds(Empty {}))?;
        let items = v
            .as_list()
            .ok_or(Error::Unexpected("get_keybinds: expected a list"))?;
        items
            .iter()
            .map(|b| {
                let mods = b.get("mods").and_then(Value::as_int);
                let key = b.get("key").and_then(Value::as_str);
                match (mods, key) {
                    (Some(m), Some(k)) => Ok(KeyCombo {
                        mods: Mods(m as u32),
                        key: k.to_string(),
                    }),
                    _ => Err(Error::Unexpected("get_keybinds: expected { mods, key }[]")),
                }
            })
            .collect()
    }

    /// The workspaces at the current level, in order.
    pub fn get_workspace_states(&mut self) -> Result<Vec<Workspace>> {
        let v = self.run_one(Cmd::GetWorkspaceStates(Empty {}))?;
        let items = v
            .as_list()
            .ok_or(Error::Unexpected("get_workspace_states: expected a list"))?;
        items
            .iter()
            .map(|w| {
                let position = w.get("position").and_then(Value::as_int);
                let state = match w.get("state").and_then(Value::as_str) {
                    Some("focused") => Some(WorkspaceState::Focused),
                    Some("active") => Some(WorkspaceState::Active),
                    Some("inactive") => Some(WorkspaceState::Inactive),
                    _ => None,
                };
                match (position, state) {
                    (Some(p), Some(s)) => Ok(Workspace {
                        position: p as u32,
                        state: s,
                    }),
                    _ => Err(Error::Unexpected(
                        "get_workspace_states: expected { position, state }[]",
                    )),
                }
            })
            .collect()
    }
}
