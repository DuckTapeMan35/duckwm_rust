# duckwm (Rust)

A Rust client for [duckwm](https://github.com/DuckTapeMan35/duckwm)'s socket,
generated from `proto/duckwm.proto` like
[duckwm_py](https://github.com/DuckTapeMan35/duckwm_py).

```toml
[dependencies]
duckwm = { git = "https://github.com/DuckTapeMan35/duckwm_rs" }
tokio = { version = "1", features = ["rt", "macros"] }
```

```rust
use duckwm::{Client, cmd};

#[tokio::main(flavor = "current_thread")]
async fn main() -> duckwm::Result<()> {
    let wm = Client::connect().await?;            // $XDG_RUNTIME_DIR/duckwm.sock
    for combo in wm.get_keybinds().await? {
        println!("{combo}");                      // super+shift+q
    }
    for ws in wm.get_workspace_states().await? {
        println!("{} {:?}", ws.position, ws.state); // 1 Focused
    }

    // Callbacks run inside the transaction that called them.
    wm.bind("super+u", |wm| async move {
        println!("focused: {:?}", wm.get_focused().await?);
        Ok(())
    })
    .await?;
    // Run by duckwm itself: works even while this process is busy.
    wm.bind_commands("super+Return", [cmd::spawn(&["kitty"])]).await?;

    // Several commands as one transaction.
    wm.run([cmd::focus(3), cmd::kill_client()]).await?;

    wm.closed().await; // serve the binds until duckwm goes away
    Ok(())
}
```

The client is async, on tokio, and a cheap handle: clone it freely. For a
whole config, `duckwm::forever(setup)` runs `setup` on every connection and
waits for duckwm when it's gone (see `examples/config.rs`); `layouts::tile()`
and `layouts::fair()` are ready-made arrangers, and `tree` and `geometry` build
your own.

Nodes can be named and found by path: `wm.set_name(id, Some("web"))`,
`wm.find("DP-1/web")`. `wm.within("web", async || ...).await` makes the
binds, rules and gaps set up inside apply only there, and puts the scope back
after, even when the closure fails.

A process running as another user (root, say) connects with
`Client::connect_to("/run/user/<uid>/duckwm.sock")`.

- The client is blocking; from async code, call it inside `spawn_blocking`.
- A process running as another user (root, say) connects with
  `Client::connect_to("/run/user/<uid>/duckwm.sock")`.

## Regenerating

`src/proto/duckwm.v1.rs` (the messages) and `src/api.rs` (a method per
command) are generated and committed, so using the crate needs nothing but
cargo. After a protocol change in duckwm:

```sh
DUCKWM_SRC=~/dev/duckwm ./scripts/gen   # or just ./scripts/gen in the dev shell
```

The generator in `gen/` parses the schema with protox, in Rust, so there's no
protoc to install. CI regenerates by itself when duckwm's schema changes.
