# duckwm (Rust)

A Rust client for [duckwm](https://github.com/DuckTapeMan35/duckwm)'s socket,
generated from `proto/duckwm.proto` like
[duckwm_py](https://github.com/DuckTapeMan35/duckwm_py).

```toml
[dependencies]
duckwm = { git = "https://github.com/DuckTapeMan35/duckwm_rs" }
```

```rust
let mut wm = duckwm::Client::connect()?;      // $XDG_RUNTIME_DIR/duckwm.sock

for combo in wm.get_keybinds()? {
    println!("{combo}");                      // super+shift+q
}
for ws in wm.get_workspace_states()? {
    println!("{} {:?}", ws.position, ws.state); // 1 Focused
}

// Every command is a method, run on its own...
wm.focus(3)?;
wm.spawn(&["kitty"])?;

// ...or several as one transaction: all take effect, or none do.
use duckwm::{Cmd, proto};
wm.run([
    Cmd::Focus(proto::Focus { id: 3 }),
    Cmd::ToggleFloating(proto::Empty {}),
])?;
```

- The client is blocking; from async code, call it inside `spawn_blocking`.
- A process running as another user (root, say) connects with
  `Client::connect_to("/run/user/<uid>/duckwm.sock")`.
- `KeyCombo` parses and prints `super+shift+q` in one canonical order, so a
  combo from a config file and one from duckwm compare equal.
- Methods return a `Value`, except the typed `get_keybinds` and
  `get_workspace_states`. Commands that take a `Callback` (`bind`,
  `add_rule`, arrangers) need a client that serves callbacks, which this
  one doesn't yet.

## Regenerating

`src/proto/duckwm.v1.rs` (the messages) and `src/api.rs` (a method per
command) are generated and committed, so using the crate needs nothing but
cargo. After a protocol change in duckwm:

```sh
DUCKWM_SRC=~/dev/duckwm ./scripts/gen   # or just ./scripts/gen in the dev shell
```

The generator in `gen/` parses the schema with protox, in Rust, so there's no
protoc to install. CI regenerates by itself when duckwm's schema changes.
