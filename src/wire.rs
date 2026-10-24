use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use prost::Message;

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

/// One connection. Frames are a little-endian u32 length, then the message.
pub(crate) struct Connection {
    stream: UnixStream,
}

impl Connection {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            stream: UnixStream::connect(path)?,
        })
    }

    pub fn send(&mut self, msg: &ClientMessage) -> io::Result<()> {
        let body = msg.encode_to_vec();
        let mut frame = Vec::with_capacity(4 + body.len());
        frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
        frame.extend_from_slice(&body);
        self.stream.write_all(&frame)
    }

    /// The next message. UnexpectedEof means duckwm closed the connection.
    pub fn recv(&mut self) -> io::Result<ServerMessage> {
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len)?;
        let n = u32::from_le_bytes(len) as usize;
        if n > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame of {n} bytes is larger than {MAX_FRAME}"),
            ));
        }
        let mut body = vec![0u8; n];
        self.stream.read_exact(&mut body)?;
        ServerMessage::decode(body.as_slice())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}
