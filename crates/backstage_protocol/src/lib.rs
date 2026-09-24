//! Messages and transport shared by the stage and tools processes.
//!
//! Control messages travel as length-prefixed postcard frames over a Unix
//! socket. Rendered stage frames travel through a shared-memory ring
//! ([`FrameRing`]) so the socket only carries small notifications. See
//! `docs/adr/0002-stage-process-isolation.md`.

mod frame_ring;

pub use frame_ring::{FRAME_SLOTS, FrameRing};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};

/// Bumped on any incompatible change to the messages below.
pub const PROTOCOL_VERSION: u32 = 1;

/// Upper bound on a single control message, to reject garbage early.
const MAX_MESSAGE_LEN: u32 = 1 << 20;

/// Tools → stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToStage {
    Hello { version: u32 },
    /// Size of the stage section in physical pixels, plus the scale factor
    /// that maps logical (widget) coordinates to physical ones.
    Resize { width: u32, height: u32, scale: f64 },
    /// Pointer position in logical pixels, or `None` when it left the stage.
    Pointer(Option<(f32, f32)>),
    Shutdown,
}

/// Stage → tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToTools {
    Hello { version: u32, pid: u32, adapter: String },
    /// A new frame ring was created (after a resize). Frames in older
    /// generations must no longer be read.
    Surface {
        generation: u64,
        path: String,
        width: u32,
        height: u32,
        stride: u32,
    },
    FrameReady { generation: u64, slot: u32, seq: u64 },
    Log(String),
    Heartbeat,
}

/// Writes one length-prefixed message.
pub fn write_message<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let bytes = postcard::to_stdvec(msg).map_err(io::Error::other)?;
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

/// Reads one length-prefixed message. Returns `UnexpectedEof` when the peer
/// hangs up.
pub fn read_message<T: DeserializeOwned>(r: &mut impl Read) -> io::Result<T> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len);
    if len > MAX_MESSAGE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message too large: {len} bytes"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    postcard::from_bytes(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trip() {
        let mut buf = Vec::new();
        write_message(&mut buf, &ToStage::Resize { width: 640, height: 480, scale: 1.25 }).unwrap();
        write_message(&mut buf, &ToStage::Shutdown).unwrap();
        let mut r = buf.as_slice();
        assert!(matches!(
            read_message::<ToStage>(&mut r).unwrap(),
            ToStage::Resize { width: 640, height: 480, .. }
        ));
        assert!(matches!(read_message::<ToStage>(&mut r).unwrap(), ToStage::Shutdown));
        assert_eq!(
            read_message::<ToStage>(&mut r).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
