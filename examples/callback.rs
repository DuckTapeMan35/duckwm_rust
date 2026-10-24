use duckwm::{Client, Reply, cmd, proto::Dir};

#[tokio::main(flavor = "current_thread")]
async fn main() -> duckwm::Result<()> {
    let wm = Client::connect().await?;

    wm.bind("super+u", |_| async {
        println!("super+u pressed");
        Ok(())
    })
    .await?;

    // Run by duckwm itself: the key works even while this process is busy.
    wm.bind_commands("super+y", [cmd::spawn(&["kitty"])])
        .await?;
    wm.bind_commands("super+ctrl+h", [cmd::resize_focused_edge(Dir::Left, 20)])
        .await?;

    // The Client a callback gets makes requests inside the transaction duckwm
    // is waiting on: they see, and change, the state as of this key press.
    wm.bind("super+i", |wm| async move {
        let focused = wm.get_focused().await?;
        if let Some(id) = focused.as_int() {
            let class = wm.get_window_class(id as u32).await?;
            println!("focused: {id} {class:?}");
        }
        Ok(())
    })
    .await?;

    // Or reply with commands for duckwm to run as part of it.
    wm.bind("super+ctrl+i", |_| async {
        Ok(Reply::commands([cmd::message("hi from Rust", 2000)]))
    })
    .await?;

    wm.add_rule(|_, event, id| async move {
        println!("rule: {event} {id}");
        Ok(())
    })
    .await?;

    println!("serving; ctrl+c to stop");
    wm.closed().await;
    Ok(())
}
