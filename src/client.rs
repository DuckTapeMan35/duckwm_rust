use std::fmt;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;

use tokio::net::UnixStream;
use tokio::sync::oneshot;

use crate::arrange::{Arranger, Snapshot};
use crate::conn::{self, Handler, Inner, Op};
use crate::mods::{KeyCombo, Mods};
use crate::proto::{self, Empty, call_args};
use crate::reply::Reply;
use crate::value::Value;
use crate::wire::socket_path;
use crate::{Cmd, cmd};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// duckwm closed the connection (it quit, restarted, or dropped us).
    Closed,
    /// duckwm refused the batch; nothing in it took effect.
    Rejected(String),
    /// A result wasn't the shape a typed method expected.
    Unexpected(&'static str),
    /// An argument we couldn't make sense of, e.g. a bad key combination.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "duckwm socket: {e}"),
            Error::Closed => f.write_str("duckwm closed the connection"),
            Error::Rejected(why) => write!(f, "duckwm refused: {why}"),
            Error::Unexpected(what) => write!(f, "unexpected reply from duckwm: {what}"),
            Error::Invalid(what) => f.write_str(what),
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
    /// Its node id, for set_name; None from a duckwm too old to say.
    pub id: Option<u32>,
}

/// Which duckwm this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    X11,
    River,
}

/// A connection to duckwm. Every command is a method (wm.focus(3).await),
/// run on its own; [`crate::cmd`] has the same commands as values, for
/// batches, binds and callback replies.
///
/// Inside a callback, the Client it's given makes requests that run in the
/// transaction duckwm is waiting on; requests from elsewhere wait their turn.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
    /// 0 at the top level; inside a callback, the call it belongs to.
    scope: u64,
}

impl Client {
    pub(crate) fn new(inner: Arc<Inner>, scope: u64) -> Self {
        Self { inner, scope }
    }

    /// Connect at [`socket_path`].
    pub async fn connect() -> Result<Self> {
        Self::connect_to(socket_path()).await
    }

    /// Connect at a given path, e.g. another user's `/run/user/<uid>/duckwm.sock`.
    pub async fn connect_to(path: impl AsRef<Path>) -> Result<Self> {
        Ok(conn::start(UnixStream::connect(path).await?))
    }

    /// Returns once the connection is gone: serving binds and arrangers
    /// until duckwm goes away is `wm.closed().await`.
    pub async fn closed(&self) {
        let mut rx = self.inner.closed.clone();
        let _ = rx.wait_for(|closed| *closed).await;
    }

    // ---- requests ----
    /// Run the commands as one transaction; one result per command.
    pub async fn run(&self, cmds: impl IntoIterator<Item = Cmd>) -> Result<Vec<Value>> {
        let (done, rx) = oneshot::channel();
        let commands = cmds.into_iter().map(Into::into).collect();
        let op = Op::Request {
            scope: self.scope,
            commands,
            done,
        };
        self.inner.ops.send(op).map_err(|_| Error::Closed)?;
        rx.await.map_err(|_| Error::Closed)?
    }

    /// Run one command and return its result.
    pub async fn run_one(&self, cmd: Cmd) -> Result<Value> {
        self.run([cmd])
            .await?
            .pop()
            .ok_or(Error::Unexpected("no result"))
    }

    // ---- callbacks ----
    /// A raw callback: f gets the call's arguments as duckwm sent them.
    pub fn register<F, Fut, R>(&self, f: F) -> proto::Callback
    where
        F: Fn(Client, proto::CallArgs) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R>> + Send + 'static,
        R: Into<Reply>,
    {
        let handler: Handler = Arc::new(move |wm, args| {
            let fut = f(wm, args);
            Box::pin(async move { fut.await.map(Into::into) })
        });
        let id = self.inner.next_id();
        self.inner.handlers.lock().unwrap().insert(id, handler);
        proto::Callback { id }
    }

    /// Bind a key ("super+shift+a") to an async function, run here on each press.
    pub async fn bind<F, Fut, R>(&self, combo: &str, f: F) -> Result<()>
    where
        F: Fn(Client) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R>> + Send + 'static,
        R: Into<Reply>,
    {
        let combo = parse_combo(combo)?;
        let callback = self.register(move |wm, _| f(wm));
        self.run_one(cmd::bind(combo.mods.0, &combo.key, callback))
            .await?;
        Ok(())
    }

    /// Bind a key to commands duckwm runs itself, so the key never waits on us.
    pub async fn bind_commands(
        &self,
        combo: &str,
        cmds: impl IntoIterator<Item = Cmd>,
    ) -> Result<()> {
        let combo = parse_combo(combo)?;
        let cmds = cmds.into_iter().map(Into::into).collect();
        self.run_one(cmd::bind_commands(combo.mods.0, &combo.key, cmds))
            .await?;
        Ok(())
    }

    /// A rule for every window: f(wm, event, id), event one of
    /// "pre_map", "map", "unmap", "prop". Returns its id, for remove_rule.
    pub async fn add_rule<F, Fut, R>(&self, f: F) -> Result<Value>
    where
        F: Fn(Client, String, u32) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R>> + Send + 'static,
        R: Into<Reply>,
    {
        let callback = self.register(move |wm, args| {
            let (event, id) = match args.kind {
                Some(call_args::Kind::Rule(r)) => (r.event, r.id),
                _ => (String::new(), 0),
            };
            f(wm, event, id)
        });
        self.run_one(cmd::add_rule(callback)).await
    }

    /// Be told what changed, once it's committed: f(wm, notice) with the
    /// topic, the scene serial and the node it's about. duckwm doesn't wait
    /// for it; the commands it returns run as a request of their own.
    /// Returns its id, for unsubscribe.
    pub async fn subscribe<F, Fut, R>(&self, topic: proto::Topic, f: F) -> Result<u32>
    where
        F: Fn(Client, proto::Notice) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R>> + Send + 'static,
        R: Into<Reply>,
    {
        let callback = self.register(move |wm, args| {
            let notice = match args.kind {
                Some(call_args::Kind::Notice(n)) => n,
                _ => proto::Notice::default(),
            };
            f(wm, notice)
        });
        let id = self.run_one(cmd::subscribe(topic, callback)).await?;
        id.as_int()
            .and_then(|i| u32::try_from(i).ok())
            .ok_or(Error::Unexpected("subscribe: expected an id"))
    }

    /// Open the input bar; f gets the line on Enter.
    pub async fn read_line<F, Fut, R>(
        &self,
        prompt: &str,
        initial: Option<&str>,
        f: F,
    ) -> Result<()>
    where
        F: Fn(Client, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R>> + Send + 'static,
        R: Into<Reply>,
    {
        let callback = self.register(move |wm, args| {
            let text = match args.kind {
                Some(call_args::Kind::Line(l)) => l.text,
                _ => String::new(),
            };
            f(wm, text)
        });
        self.run_one(cmd::read_line(prompt, callback, initial))
            .await?;
        Ok(())
    }

    // ---- arrangers ----
    /// The current workspace gets this arranger.
    pub async fn set_arranger(&self, a: &Arranger) -> Result<()> {
        self.run_one(cmd::set_arranger(self.factory(a), Some(a.name())))
            .await?;
        Ok(())
    }

    /// Node id (a workspace or group) gets this arranger.
    pub async fn register_arranger(&self, id: u32, a: &Arranger) -> Result<()> {
        self.run_one(cmd::register_arranger(id, self.factory(a), Some(a.name())))
            .await?;
        Ok(())
    }

    /// New workspaces, and those whose arranger went away, get a; the current one now.
    pub async fn default_arranger(&self, a: &Arranger) -> Result<()> {
        let f = self.factory(a);
        self.run([
            cmd::set_default_arranger(f, Some(a.name())),
            cmd::set_arranger(f, Some(a.name())),
        ])
        .await?;
        Ok(())
    }

    /// One arranger callback and one factory per arranger: its state lives in
    /// duckwm, so every workspace shares the same function.
    fn factory(&self, a: &Arranger) -> proto::Callback {
        if let Some(f) = self.inner.factories.lock().unwrap().get(a.name()) {
            return *f;
        }
        let arr = a.clone();
        let made = self.register(move |_, args| {
            let reply = match args.kind {
                Some(call_args::Kind::Arranger(s)) => Ok(arr.call(Snapshot::new(s))),
                _ => Err(Error::Unexpected("arranger called without a snapshot")),
            };
            std::future::ready(reply)
        });
        let factory = self.register(move |_, _| {
            std::future::ready(Ok(Reply {
                made: Some(made.id),
                ..Reply::none()
            }))
        });
        self.inner
            .factories
            .lock()
            .unwrap()
            .insert(a.name().to_owned(), factory);
        factory
    }

    // ---- which duckwm ----
    /// From duckwm's executable name (duckwm-river on Wayland).
    pub fn platform(&self) -> Platform {
        let exe = self
            .inner
            .peer_pid
            .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok());
        match exe.and_then(|p| p.file_name().map(|n| n.to_string_lossy().contains("river"))) {
            Some(true) => Platform::River,
            _ => Platform::X11,
        }
    }

    /// True for the first connection to this duckwm process, from any client
    /// using this library or duckwm_py: for things to do once per session.
    pub fn startup(&self) -> bool {
        *self.inner.startup.get_or_init(|| {
            let Some(pid) = self.inner.peer_pid else {
                return false;
            };
            let run = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
            let mark = Path::new(&run).join(format!("duckwm-started-{pid}"));
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(mark)
                .is_ok()
        })
    }

    // ---- typed queries ----
    /// Every bound key combination, once each.
    pub async fn get_keybinds(&self) -> Result<Vec<KeyCombo>> {
        let v = self.run_one(Cmd::GetKeybinds(Empty {})).await?;
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
    pub async fn get_workspace_states(&self) -> Result<Vec<Workspace>> {
        let v = self.run_one(Cmd::GetWorkspaceStates(Empty {})).await?;
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
                        id: w
                            .get("id")
                            .and_then(Value::as_int)
                            .and_then(|i| u32::try_from(i).ok()),
                    }),
                    _ => Err(Error::Unexpected(
                        "get_workspace_states: expected { position, state }[]",
                    )),
                }
            })
            .collect()
    }

    // ---- names ----
    /// Run `f` with the binds, rules and settings it sets up applying only in
    /// `path` (`"DP-1/web/chat"`); nested, the paths add up. The scope is the
    /// connection's, and is put back after `f`, even when it fails:
    ///
    /// ```ignore
    /// wm.within("web", async || wm.bind("super+u", on_key).await).await?;
    /// ```
    pub async fn within<T, E: From<Error>>(
        &self,
        path: &str,
        f: impl AsyncFnOnce() -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let outer = self.get_scope().await?.as_str().map(str::to_owned);
        let full = match &outer {
            Some(o) => format!("{o}/{path}"),
            None => path.to_owned(),
        };
        self.set_scope(Some(&full)).await?;
        let out = f().await;
        let back = self.set_scope(outer.as_deref()).await;
        let out = out?;
        back?;
        Ok(out)
    }
}

fn parse_combo(combo: &str) -> Result<KeyCombo> {
    combo
        .parse()
        .map_err(|e: crate::ParseComboError| Error::Invalid(e.to_string()))
}
