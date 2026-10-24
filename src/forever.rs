use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use crate::client::{Client, Error, Result};
use crate::wire::socket_path;

/// Run setup on every connection, then serve until it closes.
///
/// Everything setup registers dies with the connection on duckwm's side, so
/// it's simply redone on the next one. A config that fails (an error, or
/// duckwm rejecting it) is retried with a growing delay instead of hammering duckwm.
pub async fn forever<F, Fut>(setup: F) -> !
where
    F: Fn(Client) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    forever_at(socket_path(), setup).await
}

/// [`forever`] at a given socket path.
pub async fn forever_at<F, Fut>(path: impl Into<PathBuf>, setup: F) -> !
where
    F: Fn(Client) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let path = path.into();
    let mut delay = Duration::from_secs(1);
    let mut waiting = false;
    loop {
        let wm = match Client::connect_to(&path).await {
            Ok(wm) => wm,
            Err(_) => {
                if !waiting {
                    eprintln!("duckwm: waiting for duckwm");
                    waiting = true;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        waiting = false;
        eprintln!("duckwm: connected");
        match setup(wm.clone()).await {
            Ok(()) => {
                delay = Duration::from_secs(1);
                wm.closed().await;
                eprintln!("duckwm: disconnected");
                continue;
            }
            Err(Error::Closed) => {
                eprintln!("duckwm: disconnected");
                continue;
            }
            Err(Error::Rejected(why)) => eprintln!("duckwm: duckwm rejected the config: {why}"),
            Err(e) => eprintln!("duckwm: config failed: {e}"),
        }
        drop(wm);
        // Broken config: back off until it's fixed (or duckwm restarts).
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}
