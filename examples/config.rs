//! A whole config, run for good: arrangers on keys, a default, and a bind;
//! when duckwm restarts it waits for it and sets everything up again.

use duckwm::layouts::{fair, tile};
use duckwm::{Client, cmd};

async fn setup(wm: Client) -> duckwm::Result<()> {
    let (fair, tile) = (fair(), tile());
    let t = tile.clone();
    wm.bind("super+t", move |wm| {
        let t = t.clone();
        async move { wm.set_arranger(&t).await }
    })
    .await?;
    let f = fair.clone();
    wm.bind("super+ctrl+f", move |wm| {
        let f = f.clone();
        async move { wm.set_arranger(&f).await }
    })
    .await?;
    wm.bind_commands("super+Return", [cmd::spawn(&["kitty"])])
        .await?;
    wm.default_arranger(&fair).await?;
    if wm.startup() {
        wm.spawn(&["dunst"]).await?;
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    duckwm::forever(setup).await
}
