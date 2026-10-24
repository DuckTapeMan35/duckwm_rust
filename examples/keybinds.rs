//! Print every bound key combination and the workspaces.

#[tokio::main(flavor = "current_thread")]
async fn main() -> duckwm::Result<()> {
    let wm = duckwm::Client::connect().await?;
    for combo in wm.get_keybinds().await? {
        println!("{combo}");
    }
    for ws in wm.get_workspace_states().await? {
        println!("workspace {}: {:?}", ws.position, ws.state);
    }
    Ok(())
}
