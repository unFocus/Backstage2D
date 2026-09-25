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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToStage {
    Hello {
        version: u32,
    },
    /// Size of the stage section in physical pixels, plus the scale factor
    /// that maps logical (widget) coordinates to physical ones.
    Resize {
        width: u32,
        height: u32,
        scale: f64,
    },
    /// Pointer position in logical pixels, or `None` when it left the stage.
    Pointer(Option<(f32, f32)>),
    Shutdown,
}

/// Stage → tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToTools {
    Hello {
        version: u32,
        pid: u32,
        adapter: String,
    },
    /// A new frame ring was created (after a resize). Frames in older
    /// generations must no longer be read.
    Surface {
        generation: u64,
        path: String,
        width: u32,
        height: u32,
        stride: u32,
    },
    FrameReady {
        generation: u64,
        slot: u32,
        seq: u64,
    },
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
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("message too large: {len} bytes")));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    postcard::from_bytes(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn round_trip<T: Serialize + DeserializeOwned>(msg: &T) -> T {
        let mut buf = Vec::new();
        write_message(&mut buf, msg).unwrap();
        let mut r = buf.as_slice();
        let back = read_message(&mut r).unwrap();
        assert!(r.is_empty(), "trailing bytes after one message");
        back
    }

    fn any_to_stage() -> impl Strategy<Value = ToStage> {
        prop_oneof![
            any::<u32>().prop_map(|version| ToStage::Hello { version }),
            (any::<u32>(), any::<u32>(), 0.1f64..8.0).prop_map(|(width, height, scale)| ToStage::Resize {
                width,
                height,
                scale
            }),
            proptest::option::of((-1e6f32..1e6, -1e6f32..1e6)).prop_map(ToStage::Pointer),
            Just(ToStage::Shutdown),
        ]
    }

    fn any_to_tools() -> impl Strategy<Value = ToTools> {
        prop_oneof![
            (any::<u32>(), any::<u32>(), ".*").prop_map(|(version, pid, adapter)| ToTools::Hello {
                version,
                pid,
                adapter
            }),
            (any::<u64>(), ".*", any::<u32>(), any::<u32>(), any::<u32>()).prop_map(
                |(generation, path, width, height, stride)| ToTools::Surface {
                    generation,
                    path,
                    width,
                    height,
                    stride
                }
            ),
            (any::<u64>(), any::<u32>(), any::<u64>())
                .prop_map(|(generation, slot, seq)| ToTools::FrameReady { generation, slot, seq }),
            ".*".prop_map(ToTools::Log),
            Just(ToTools::Heartbeat),
        ]
    }

    proptest! {
        #[test]
        fn to_stage_round_trips(msg in any_to_stage()) {
            prop_assert_eq!(round_trip(&msg), msg);
        }

        #[test]
        fn to_tools_round_trips(msg in any_to_tools()) {
            prop_assert_eq!(round_trip(&msg), msg);
        }

        /// The tools process parses whatever the stage sends; garbage must
        /// produce an error, never a panic.
        #[test]
        fn garbage_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
            let mut r = bytes.as_slice();
            let _ = read_message::<ToTools>(&mut r);
            let mut r = bytes.as_slice();
            let _ = read_message::<ToStage>(&mut r);
        }
    }

    #[test]
    fn eof_between_messages_is_unexpected_eof() {
        let mut r: &[u8] = &[];
        assert_eq!(read_message::<ToStage>(&mut r).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn oversized_length_is_rejected_without_allocating() {
        let mut bytes = (MAX_MESSAGE_LEN + 1).to_le_bytes().to_vec();
        bytes.extend([0; 16]);
        let err = read_message::<ToTools>(&mut bytes.as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn truncated_message_is_an_error() {
        let mut buf = Vec::new();
        write_message(&mut buf, &ToTools::Log("hello".into())).unwrap();
        buf.pop();
        assert!(read_message::<ToTools>(&mut buf.as_slice()).is_err());
    }

    /// Wire-format regression check: any change here means old and new
    /// processes can't talk. Update the snapshot only together with a
    /// `PROTOCOL_VERSION` bump.
    #[test]
    fn wire_format_snapshot() {
        fn hex<T: Serialize>(msg: &T) -> String {
            postcard::to_stdvec(msg).unwrap().iter().map(|b| format!("{b:02x}")).collect()
        }
        let mut out = format!("PROTOCOL_VERSION = {PROTOCOL_VERSION}\n");
        for msg in [
            ToStage::Hello { version: PROTOCOL_VERSION },
            ToStage::Resize { width: 1920, height: 1080, scale: 1.25 },
            ToStage::Pointer(Some((10.5, 20.0))),
            ToStage::Pointer(None),
            ToStage::Shutdown,
        ] {
            out += &format!("{msg:?} => {}\n", hex(&msg));
        }
        for msg in [
            ToTools::Hello { version: PROTOCOL_VERSION, pid: 4242, adapter: "gpu".into() },
            ToTools::Surface { generation: 2, path: "/run/x".into(), width: 640, height: 480, stride: 2560 },
            ToTools::FrameReady { generation: 2, slot: 1, seq: 300 },
            ToTools::Log("hi".into()),
            ToTools::Heartbeat,
        ] {
            out += &format!("{msg:?} => {}\n", hex(&msg));
        }
        insta::assert_snapshot!(out);
    }
}
