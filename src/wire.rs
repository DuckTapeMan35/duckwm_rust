use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::proto::{ClientMessage, ServerMessage};

/// Larger frames are refused rather than read.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// `$XDG_RUNTIME_DIR/duckwm.sock`, or `/tmp/duckwm-<uid>.sock`, like duckwm.
pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(run) => PathBuf::from(run).join("duckwm.sock"),
        None => {
            // /proc/self belongs to whoever we run as: our uid without libc.
            let uid = std::fs::metadata("/proc/self")
                .map(|m| m.uid())
                .unwrap_or(0);
            PathBuf::from(format!("/tmp/duckwm-{uid}.sock"))
        }
    }
}

/// One frame: a little-endian u32 length, then the message.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, msg: &ClientMessage) -> io::Result<()> {
    let body = msg.encode_to_vec();
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    w.write_all(&frame).await
}

/// The next frame. UnexpectedEof means duckwm closed the connection.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<ServerMessage> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {n} bytes is larger than {MAX_FRAME}"),
        ));
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).await?;
    ServerMessage::decode(body.as_slice())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
