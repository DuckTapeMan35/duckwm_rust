use duckwm::cmd;

#[tokio::main(flavor = "current_thread")]
async fn main() -> duckwm::Result<()> {
    let wm = duckwm::Client::connect().await?;
    println!(
        "{:?} duckwm, first connection: {}",
        wm.platform(),
        wm.startup()
    );
    let results = wm
        .run([cmd::get_focused(), cmd::get_current_workspace()])
        .await?;
    println!("{results:?}");
    Ok(())
}
