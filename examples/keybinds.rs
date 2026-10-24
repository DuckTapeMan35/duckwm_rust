fn main() -> duckwm::Result<()> {
    let mut wm = duckwm::Client::connect()?;
    for combo in wm.get_keybinds()? {
        println!("{combo}");
    }
    for ws in wm.get_workspace_states()? {
        println!("workspace {}: {:?}", ws.position, ws.state);
    }
    Ok(())
}
