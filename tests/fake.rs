use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use duckwm::proto::{self, client_message, command, server_message};
use duckwm::{Client, Error, Value, layouts};

// ---- the fake ----
struct Fake {
    stream: UnixStream,
}

impl Fake {
    async fn recv(&mut self) -> client_message::Kind {
        let n = self.stream.read_u32_le().await.unwrap() as usize;
        let mut body = vec![0; n];
        self.stream.read_exact(&mut body).await.unwrap();
        proto::ClientMessage::decode(body.as_slice())
            .unwrap()
            .kind
            .unwrap()
    }

    async fn send(&mut self, kind: server_message::Kind) {
        let body = proto::ServerMessage { kind: Some(kind) }.encode_to_vec();
        self.stream.write_u32_le(body.len() as u32).await.unwrap();
        self.stream.write_all(&body).await.unwrap();
    }

    async fn request(&mut self) -> Vec<command::Kind> {
        match self.recv().await {
            client_message::Kind::Request(r) => {
                r.commands.into_iter().map(|c| c.kind.unwrap()).collect()
            }
            other => panic!("expected a request, got {other:?}"),
        }
    }

    async fn ret(&mut self) -> proto::Return {
        match self.recv().await {
            client_message::Kind::Ret(r) => r,
            other => panic!("expected a return, got {other:?}"),
        }
    }

    async fn respond(&mut self, results: Vec<Value>) {
        let results = results.into_iter().map(Into::into).collect();
        let r = proto::Response {
            results,
            error: String::new(),
        };
        self.send(server_message::Kind::Response(r)).await;
    }

    async fn reject(&mut self, why: &str) {
        let r = proto::Response {
            results: vec![],
            error: why.into(),
        };
        self.send(server_message::Kind::Response(r)).await;
    }
    /// A call set off by the request being answered, as duckwm marks it.
    async fn call(&mut self, callback: u64, args: proto::call_args::Kind) {
        self.call_as(callback, args, true).await;
    }

    async fn call_as(&mut self, callback: u64, args: proto::call_args::Kind, nested: bool) {
        let args = Some(proto::CallArgs { kind: Some(args) });
        let call = proto::Call {
            callback,
            args,
            nested,
        };
        self.send(server_message::Kind::Call(call)).await;
    }
}

/// A connected client and the fake duckwm at the other end.
async fn pair() -> (Client, Fake) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("duckwm-test-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path: PathBuf = dir.join("duckwm.sock");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let (client, accepted) = tokio::join!(Client::connect_to(&path), listener.accept());
    (
        client.unwrap(),
        Fake {
            stream: accepted.unwrap().0,
        },
    )
}

fn bind(mods: i64, key: &str) -> Value {
    Value::Map(vec![
        ("mods".into(), mods.into()),
        ("key".into(), key.into()),
    ])
}

// ---- tests ----
#[tokio::test]
async fn a_request_and_its_results() {
    let (wm, mut duck) = pair().await;
    let server = tokio::spawn(async move {
        assert_eq!(
            duck.request().await,
            vec![command::Kind::GetZoom(proto::Empty {})]
        );
        duck.respond(vec![Value::Num(0.5)]).await;
        duck
    });
    assert_eq!(wm.get_zoom().await.unwrap(), Value::Num(0.5));
    server.await.unwrap();
}

#[tokio::test]
async fn a_rejected_batch() {
    let (wm, mut duck) = pair().await;
    let _server = tokio::spawn(async move {
        duck.request().await;
        duck.reject("no such node").await;
        duck
    });
    match wm.focus(99).await {
        Err(Error::Rejected(why)) => assert_eq!(why, "no such node"),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn typed_keybinds() {
    let (wm, mut duck) = pair().await;
    let _server = tokio::spawn(async move {
        duck.request().await;
        duck.respond(vec![Value::List(vec![bind(65, "q"), bind(64, "Return")])])
            .await;
        duck
    });
    let combos: Vec<String> = wm
        .get_keybinds()
        .await
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(combos, ["super+shift+q", "super+Return"]);
}

/// A key press: duckwm calls the bind, the bind asks something inside the
/// transaction, and a request from elsewhere waits until the call returns.
#[tokio::test]
async fn a_callback_with_a_nested_request() {
    let (wm, mut duck) = pair().await;
    let presses = Arc::new(AtomicUsize::new(0));
    let p = presses.clone();

    let bound = tokio::spawn({
        let wm = wm.clone();
        async move {
            wm.bind("super+u", move |wm| {
                let p = p.clone();
                async move {
                    assert_eq!(wm.get_focused().await?, Value::Int(7));
                    p.fetch_add(1, Ordering::SeqCst);
                    Ok(duckwm::cmd::kill_client())
                }
            })
            .await
        }
    });
    let callback = match duck.request().await.pop().unwrap() {
        command::Kind::Bind(b) => {
            assert_eq!((b.mods, b.key.as_str()), (64, "u"));
            b.callback.unwrap().id
        }
        other => panic!("expected bind, got {other:?}"),
    };
    duck.respond(vec![Value::Nil]).await;
    bound.await.unwrap().unwrap();

    // The press. Its request arrives; while the call is being answered, a
    // top-level request from another task must wait. (Started once the call
    // is under way: sent at the same moment, the two would cross on the wire.)
    duck.call(callback, proto::call_args::Kind::Key(proto::Empty {}))
        .await;
    assert_eq!(
        duck.request().await,
        vec![command::Kind::GetFocused(proto::Empty {})]
    );
    let outside = tokio::spawn({
        let wm = wm.clone();
        async move { wm.get_zoom().await }
    });
    tokio::task::yield_now().await;
    duck.respond(vec![Value::Int(7)]).await;
    let ret = duck.ret().await;
    assert_eq!(ret.err, None);
    assert_eq!(
        ret.commands[0].kind,
        Some(command::Kind::KillClient(proto::Empty {}))
    );
    assert_eq!(presses.load(Ordering::SeqCst), 1);

    // Only now does the outside request go.
    assert_eq!(
        duck.request().await,
        vec![command::Kind::GetZoom(proto::Empty {})]
    );
    duck.respond(vec![Value::Num(1.0)]).await;
    assert_eq!(outside.await.unwrap().unwrap(), Value::Num(1.0));
}

/// A callback that fails, or panics, still answers duckwm, with the error.
#[tokio::test]
async fn a_failing_callback_reports_back() {
    let (wm, mut duck) = pair().await;
    let failing = wm.register(|_, _| async { Err::<(), _>(Error::Invalid("nope".into())) });
    duck.call(failing.id, proto::call_args::Kind::Key(proto::Empty {}))
        .await;
    assert_eq!(duck.ret().await.err.as_deref(), Some("nope"));

    let panicking = wm.register(|_, _| async {
        panic!("boom");
        #[allow(unreachable_code)]
        Ok(())
    });
    duck.call(panicking.id, proto::call_args::Kind::Key(proto::Empty {}))
        .await;
    assert!(
        duck.ret()
            .await
            .err
            .unwrap()
            .starts_with("callback panicked")
    );

    // An id nobody registered gets an error too, rather than silence.
    duck.call(999, proto::call_args::Kind::Key(proto::Empty {}))
        .await;
    assert_eq!(duck.ret().await.err.as_deref(), Some("no such callback"));
}

/// set_arranger: duckwm calls the factory, which names the arranger; then
/// calls that with a snapshot and gets the layout and state back.
#[tokio::test]
async fn the_arranger_handshake() {
    let (wm, mut duck) = pair().await;
    let set = tokio::spawn({
        let wm = wm.clone();
        async move { wm.set_arranger(&layouts::tile()).await }
    });
    let factory = match duck.request().await.pop().unwrap() {
        command::Kind::SetArranger(a) => {
            assert_eq!(a.name.as_deref(), Some("tile"));
            a.factory.unwrap().id
        }
        other => panic!("expected set_arranger, got {other:?}"),
    };

    // duckwm calls the factory while set_arranger is still in flight.
    duck.call(factory, proto::call_args::Kind::Factory(proto::Empty {}))
        .await;
    let arranger = duck
        .ret()
        .await
        .callback
        .expect("the factory names its arranger");
    duck.respond(vec![Value::Nil]).await;
    set.await.unwrap().unwrap();

    let win = |id| proto::WindowInfo { id, geometry: None };
    let snapshot = proto::Snapshot {
        event: proto::ArrangerEvent::Map as i32,
        id: 2,
        prev_id: Some(1),
        work_area: Some(proto::Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 500,
        }),
        windows: vec![win(1), win(2)],
        ..Default::default()
    };
    duck.call(arranger, proto::call_args::Kind::Arranger(snapshot))
        .await;
    let ret = duck.ret().await;
    assert_eq!(Value::from(ret.state.unwrap()).as_ids(), Some(vec![1, 2]));
    // The root container splits 1 | 2, 55/45.
    let split = ret.layout.unwrap().nodes[0]
        .constraints
        .iter()
        .find_map(|c| match &c.kind {
            Some(proto::constraint_spec::Kind::Split(s)) => Some(s.clone()),
            _ => None,
        });
    assert_eq!(split.unwrap().ratios, vec![0.55, 0.45]);
    // A mapped window gets focus.
    assert_eq!(
        ret.commands[0].kind,
        Some(command::Kind::Focus(proto::Focus { id: 2 }))
    );
}

#[tokio::test]
async fn hanging_up_fails_whats_pending() {
    let (wm, mut duck) = pair().await;
    let pending = tokio::spawn({
        let wm = wm.clone();
        async move { wm.get_zoom().await }
    });
    duck.request().await;
    drop(duck);
    assert!(matches!(pending.await.unwrap(), Err(Error::Closed)));
    wm.closed().await; // returns rather than hanging
    assert!(matches!(wm.get_zoom().await, Err(Error::Closed)));
}

/// A key press crossing a request on the wire: duckwm runs the request inside
/// the call and answers it first. The callback's own request must wait for
/// that answer, or the two responses would go to the wrong requests.
#[tokio::test]
async fn a_call_crossing_a_request() {
    let (wm, mut duck) = pair().await;
    let cb = wm.register(|wm, _| async move {
        assert_eq!(wm.get_focused().await?, Value::Int(7));
        Ok(())
    });

    let zoom = tokio::spawn({
        let wm = wm.clone();
        async move { wm.get_zoom().await }
    });
    assert_eq!(
        duck.request().await,
        vec![command::Kind::GetZoom(proto::Empty {})]
    );
    // The press was already on its way: not nested in get_zoom.
    duck.call_as(cb.id, proto::call_args::Kind::Key(proto::Empty {}), false)
        .await;
    // Give the callback every chance to jump the queue.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    duck.respond(vec![Value::Num(0.5)]).await;
    assert_eq!(
        duck.request().await,
        vec![command::Kind::GetFocused(proto::Empty {})]
    );
    duck.respond(vec![Value::Int(7)]).await;
    assert_eq!(duck.ret().await.err, None);
    assert_eq!(zoom.await.unwrap().unwrap(), Value::Num(0.5));
}
