use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{mpsc, oneshot, watch};

use crate::client::{Client, Error, Result};
use crate::proto::{self, client_message, server_message};
use crate::reply::Reply;
use crate::value::Value;
use crate::wire::{read_frame, write_frame};

pub(crate) type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub(crate) type Handler =
    Arc<dyn Fn(Client, proto::CallArgs) -> BoxFuture<Result<Reply>> + Send + Sync>;

/// What every Client handle shares.
pub(crate) struct Inner {
    pub ops: mpsc::UnboundedSender<Op>,
    pub handlers: Mutex<HashMap<u64, Handler>>,
    /// One factory per arranger name: see Client::factory.
    pub factories: Mutex<HashMap<String, proto::Callback>>,
    pub closed: watch::Receiver<bool>,
    pub peer_pid: Option<i32>,
    pub startup: OnceLock<bool>,
    next_id: AtomicU64,
}

impl Inner {
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

pub(crate) type Done = oneshot::Sender<Result<Vec<Value>>>;

pub(crate) enum Op {
    /// scope 0: top level; otherwise the callback the request was made in.
    Request {
        scope: u64,
        commands: Vec<proto::Command>,
        done: Done,
    },
    Return {
        scope: u64,
        ret: proto::Return,
    },
}

impl Op {
    fn scope(&self) -> u64 {
        match self {
            Op::Request { scope, .. } | Op::Return { scope, .. } => *scope,
        }
    }
}

/// Connect and start the task; the Client is the first handle.
pub(crate) fn start(stream: UnixStream) -> Client {
    let peer_pid = stream.peer_cred().ok().and_then(|c| c.pid());
    let (ops_tx, ops_rx) = mpsc::unbounded_channel();
    let (closed_tx, closed_rx) = watch::channel(false);
    let inner = Arc::new(Inner {
        ops: ops_tx,
        handlers: Mutex::new(HashMap::new()),
        factories: Mutex::new(HashMap::new()),
        closed: closed_rx,
        peer_pid,
        startup: OnceLock::new(),
        next_id: AtomicU64::new(1),
    });
    tokio::spawn(run(stream, Arc::downgrade(&inner), ops_rx, closed_tx));
    Client::new(inner, 0)
}

#[derive(Default)]
struct State {
    /// The calls being answered, innermost last.
    frames: Vec<u64>,
    next_frame: u64,
    /// Requests sent and not yet answered, each with how deep in calls it was sent.
    inflight: Vec<(usize, Done)>,
    queue: VecDeque<Op>,
}

impl State {
    /// Who may send now: the innermost call, or the top level.
    fn active(&self) -> u64 {
        self.frames.last().copied().unwrap_or(0)
    }

    /// A request from a call that has already returned is a top-level one.
    fn scope_of(&self, scope: u64) -> u64 {
        if self.frames.contains(&scope) {
            scope
        } else {
            0
        }
    }

    /// One request at a time per level: duckwm answers before anything else goes.
    fn busy(&self) -> bool {
        self.inflight
            .last()
            .is_some_and(|(depth, _)| *depth == self.frames.len())
    }

    async fn flush(&mut self, wr: &mut OwnedWriteHalf) -> std::io::Result<()> {
        while !self.busy() {
            let active = self.active();
            let Some(pos) = self
                .queue
                .iter()
                .position(|op| self.scope_of(op.scope()) == active)
            else {
                break;
            };
            match self.queue.remove(pos).unwrap() {
                Op::Request { commands, done, .. } => {
                    let msg = client_message::Kind::Request(proto::Request { commands });
                    write_frame(wr, &proto::ClientMessage { kind: Some(msg) }).await?;
                    self.inflight.push((self.frames.len(), done));
                }
                Op::Return { ret, .. } => {
                    let msg = client_message::Kind::Ret(ret);
                    write_frame(wr, &proto::ClientMessage { kind: Some(msg) }).await?;
                    self.frames.pop();
                }
            }
        }
        Ok(())
    }

    fn incoming(&mut self, msg: proto::ServerMessage, inner: &Weak<Inner>) {
        match msg.kind {
            Some(server_message::Kind::Response(r)) => {
                let Some((_, done)) = self.inflight.pop() else {
                    return;
                };
                let _ = done.send(if r.error.is_empty() {
                    Ok(r.results.into_iter().map(Value::from).collect())
                } else {
                    Err(Error::Rejected(r.error))
                });
            }
            Some(server_message::Kind::Call(call)) => {
                // Not set off by our request: a request still in flight crossed
                // this call on the wire. duckwm runs it inside the call and
                // answers it first, so it belongs to the new frame now, and
                // keeps that level busy until its response arrives.
                if !call.nested {
                    let depth = self.frames.len();
                    for (d, _) in self.inflight.iter_mut().filter(|(d, _)| *d == depth) {
                        *d = depth + 1;
                    }
                }
                self.next_frame += 1;
                let frame = self.next_frame;
                self.frames.push(frame);
                self.call(frame, call, inner);
            }
            None => {}
        }
    }

    /// Run the callback in a task of its own; its Return comes back as an Op.
    fn call(&mut self, frame: u64, call: proto::Call, inner: &Weak<Inner>) {
        let fail = |why: &str| proto::Return {
            err: Some(why.into()),
            ..Default::default()
        };
        let Some(inner) = inner.upgrade() else {
            self.queue.push_back(Op::Return {
                scope: frame,
                ret: fail("client gone"),
            });
            return;
        };
        let handler = inner.handlers.lock().unwrap().get(&call.callback).cloned();
        let Some(handler) = handler else {
            self.queue.push_back(Op::Return {
                scope: frame,
                ret: fail("no such callback"),
            });
            return;
        };
        let ops = inner.ops.clone();
        let client = Client::new(inner, frame);
        let args = call.args.unwrap_or_default();
        tokio::spawn(async move {
            // Its own task, so a panic still answers duckwm instead of stalling it.
            let ret = match tokio::spawn(handler(client, args)).await {
                Ok(Ok(reply)) => reply.into_return(),
                Ok(Err(e)) => fail(&e.to_string()),
                Err(e) => fail(&format!("callback panicked: {e}")),
            };
            let _ = ops.send(Op::Return { scope: frame, ret });
        });
    }

    /// The connection is gone: everyone waiting hears so.
    fn fail_all(&mut self) {
        for (_, done) in self.inflight.drain(..) {
            let _ = done.send(Err(Error::Closed));
        }
        for op in self.queue.drain(..) {
            if let Op::Request { done, .. } = op {
                let _ = done.send(Err(Error::Closed));
            }
        }
    }
}

async fn run(
    stream: UnixStream,
    inner: Weak<Inner>,
    mut ops: mpsc::UnboundedReceiver<Op>,
    closed: watch::Sender<bool>,
) {
    let (mut rd, mut wr) = stream.into_split();
    let (msgs_tx, mut msgs) = mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        loop {
            let msg = read_frame(&mut rd).await;
            let end = msg.is_err();
            if msgs_tx.send(msg).is_err() || end {
                break;
            }
        }
    });

    let mut st = State::default();
    loop {
        tokio::select! {
            msg = msgs.recv() => match msg {
                Some(Ok(m)) => st.incoming(m, &inner),
                _ => break, // read error or EOF: duckwm is gone
            },
            op = ops.recv() => match op {
                Some(op) => st.queue.push_back(op),
                None => break, // every Client dropped
            },
        }
        if st.flush(&mut wr).await.is_err() {
            break;
        }
    }
    reader.abort();
    st.fail_all();
    let _ = closed.send(true);
}
