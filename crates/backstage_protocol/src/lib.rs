//! Messages and transport shared by the stage and tools processes.
//!
//! Control messages travel as length-prefixed postcard frames over a Unix
//! socket. Rendered stage frames travel through a shared-memory ring
//! ([`FrameRing`]) so the socket only carries small notifications. See
//! `docs/adr/0002-stage-process-isolation.md`.
//!
//! Document data (the project snapshot and log entries) travels as RON text
//! inside the postcard messages. Core types leave default fields out when
//! serializing (`skip_serializing_if`), which only a self-describing format
//! can read back; postcard can't.

mod frame_ring;

pub use frame_ring::{FRAME_SLOTS, FrameRing};

use backstage_core::{AnimId, CompId, Entry, LoadError, NodeId, Project, SaveError, Time};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};
use std::path::PathBuf;

/// Bumped on any incompatible change to the messages below.
pub const PROTOCOL_VERSION: u32 = 8;

/// Upper bound on a single control message, to reject garbage early. Big
/// enough for a project snapshot.
const MAX_MESSAGE_LEN: u32 = 64 << 20;

/// A whole project as its canonical files (`backstage_core::to_files`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// `(path relative to the project directory, contents)`.
    pub files: Vec<(String, String)>,
}

impl Snapshot {
    pub fn of(project: &Project) -> Result<Self, SaveError> {
        let files = backstage_core::to_files(project)?;
        Ok(Self {
            files: files
                .into_iter()
                .map(|(path, text)| (path.to_string_lossy().into_owned(), text))
                .collect(),
        })
    }

    /// Parses and validates the project.
    pub fn project(&self) -> Result<Project, LoadError> {
        let files: Vec<_> =
            self.files.iter().map(|(path, text)| (PathBuf::from(path), text.clone())).collect();
        backstage_core::from_files(&files)
    }
}

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
    /// Pointer input over the stage section.
    Pointer(PointerEvent),
    /// Abandon the on-stage gesture in progress, if any (Escape), committing
    /// nothing.
    CancelGesture,
    Shutdown,
    /// The document to edit: a base project and the log to replay onto it.
    /// Sent on every (re)connect. The stage answers with `Loaded`.
    Load {
        base: Snapshot,
        #[serde(with = "ron_text")]
        log: Vec<Entry>,
    },
    /// Asks the stage to apply an entry. Answered with `Committed` or
    /// `Rejected` carrying the same `request`.
    Submit {
        request: u64,
        #[serde(with = "ron_text")]
        entry: Entry,
    },
    /// What the editor shows and its playhead: `composition` (the root if
    /// `None`) plays `animation` (its default if `None`), its clock reads
    /// `time` now, and it advances only if `playing`. Without one, the stage
    /// plays the root from zero. (The IDs and `Time` are plain strings, so
    /// they are safe in postcard.)
    Transport {
        composition: Option<CompId>,
        animation: Option<AnimId>,
        time: Time,
        playing: bool,
    },
    /// The editor's selection, which the stage mirrors (ADR 0005): nodes of
    /// composition `comp`, in order, the last one primary. Sent on every
    /// change and on every connect.
    Selection {
        comp: CompId,
        nodes: Vec<NodeId>,
    },
}

/// Pointer input over the stage section, in logical pixels. Only the
/// primary button presses (`Down`) and releases (`Up`); between them the
/// pointer is held, and `Move`s keep coming even outside the section.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PointerEvent {
    Move(PointerAt),
    Down(PointerAt),
    Up(PointerAt),
    /// The pointer left the stage section (while not held).
    Leave,
}

impl PointerEvent {
    /// Where it happened, except for `Leave`.
    pub fn at(&self) -> Option<PointerAt> {
        match *self {
            PointerEvent::Move(at) | PointerEvent::Down(at) | PointerEvent::Up(at) => Some(at),
            PointerEvent::Leave => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerAt {
    pub x: f32,
    pub y: f32,
    pub modifiers: Modifiers,
}

impl PointerAt {
    /// At `(x, y)` with no modifiers held.
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y, modifiers: Modifiers::default() }
    }
}

/// Keyboard modifiers held during a pointer event. `ctrl` is the primary
/// shortcut modifier (Command on macOS, once there's a macOS editor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
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
    /// The document from `Load` is ready: `seq` entries replayed, and its
    /// `Document::hash`, for the tools process to compare with its copy.
    Loaded {
        seq: u64,
        hash: u64,
    },
    /// An entry was applied and given sequence number `seq`. `request` is
    /// set when it came from a `Submit`.
    Committed {
        seq: u64,
        request: Option<u64>,
        #[serde(with = "ron_text")]
        entry: Entry,
    },
    /// A submitted entry was not applied; the document is unchanged.
    Rejected {
        request: u64,
        reason: String,
    },
    /// Nodes picked on the stage (ADR 0005): the editor applies `mode` to
    /// its selection, if it still edits `comp`, and sends `Selection` back.
    /// The stage has already applied it to its mirror.
    Picked {
        comp: CompId,
        nodes: Vec<NodeId>,
        mode: PickMode,
    },
    /// Where the stage sits in the stage section: stage point `p` is at
    /// `origin + p × scale` in logical pixels of the section. Sent before
    /// the first frame and whenever it changes (a resize, a new document,
    /// a new stage size).
    Framing {
        origin: (f32, f32),
        scale: f32,
    },
}

/// How a pick changes the selection. Shared by the stage (its mirror) and
/// the editor (the selection itself) so they can't disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickMode {
    /// The picked nodes become the selection.
    Replace,
    /// Picked nodes not yet selected are added.
    Add,
    /// Selected picked nodes are removed; the others are added.
    Toggle,
}

impl PickMode {
    /// The selection after picking `picked` with `current` selected. Nodes
    /// added go at the end, so the last one picked becomes the primary.
    pub fn apply(self, current: &[NodeId], picked: &[NodeId]) -> Vec<NodeId> {
        let mut out = match self {
            PickMode::Replace => Vec::new(),
            PickMode::Add | PickMode::Toggle => current.to_vec(),
        };
        for &node in picked {
            match out.iter().position(|n| *n == node) {
                Some(i) if self == PickMode::Toggle => {
                    out.remove(i);
                }
                Some(_) => {}
                None => out.push(node),
            }
        }
        out
    }
}

/// Serializes a document value as a one-line RON string (see the crate
/// docs for why).
mod ron_text {
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};

    pub fn serialize<T: Serialize, S: Serializer>(value: &T, s: S) -> Result<S::Ok, S::Error> {
        backstage_core::io::to_ron_line(value).map_err(serde::ser::Error::custom)?.serialize(s)
    }

    pub fn deserialize<'de, T: DeserializeOwned, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
        let text = String::deserialize(d)?;
        backstage_core::io::from_ron_line(&text).map_err(serde::de::Error::custom)
    }
}

/// Writes one length-prefixed message. A message over the size limit is an
/// error, since the peer would reject it and hang up.
pub fn write_message<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let bytes = postcard::to_stdvec(msg).map_err(io::Error::other)?;
    if bytes.len() > MAX_MESSAGE_LEN as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("message too large: {} bytes", bytes.len()),
        ));
    }
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
    use backstage_core::sample::{self, ids};
    use backstage_core::{Command, Props};
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
            any_pointer().prop_map(ToStage::Pointer),
            Just(ToStage::CancelGesture),
            Just(ToStage::Shutdown),
            (proptest::collection::vec((".*", ".*"), 0..4), proptest::collection::vec(any_entry(), 0..4))
                .prop_map(|(files, log)| ToStage::Load { base: Snapshot { files }, log }),
            (any::<u64>(), any_entry()).prop_map(|(request, entry)| ToStage::Submit { request, entry }),
            (
                proptest::option::of(any::<u64>()),
                proptest::option::of(any::<u64>()),
                any::<i64>(),
                any::<bool>()
            )
                .prop_map(|(comp, anim, flicks, playing)| ToStage::Transport {
                    composition: comp.map(backstage_core::CompId::from_raw),
                    animation: anim.map(backstage_core::AnimId::from_raw),
                    time: backstage_core::Time::from_flicks(flicks),
                    playing,
                }),
            (any::<u64>(), proptest::collection::vec(any::<u64>(), 0..4)).prop_map(|(comp, nodes)| {
                ToStage::Selection {
                    comp: backstage_core::CompId::from_raw(comp),
                    nodes: nodes.into_iter().map(backstage_core::NodeId::from_raw).collect(),
                }
            }),
        ]
    }

    fn any_pointer() -> impl Strategy<Value = PointerEvent> {
        let at = (-1e6f32..1e6, -1e6f32..1e6, any::<(bool, bool, bool)>()).prop_map(
            |(x, y, (shift, ctrl, alt))| PointerAt { x, y, modifiers: Modifiers { shift, ctrl, alt } },
        );
        prop_oneof![
            at.clone().prop_map(PointerEvent::Move),
            at.clone().prop_map(PointerEvent::Down),
            at.prop_map(PointerEvent::Up),
            Just(PointerEvent::Leave),
        ]
    }

    /// Entries whose text form must survive exactly: arbitrary floats and
    /// strings go through RON.
    fn any_entry() -> impl Strategy<Value = Entry> {
        prop_oneof![
            Just(Entry::Undo),
            Just(Entry::Redo),
            (-1e6f32..1e6, -1e6f32..1e6).prop_map(|(x, y)| Entry::Do(Command::SetRest {
                comp: ids::STAGE,
                node: ids::GROUND,
                rest: Props::at(x, y)
            })),
            ".*".prop_map(|name| Entry::Do(Command::RenameNode {
                comp: ids::STAGE,
                node: ids::GROUND,
                name
            })),
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
            (any::<u64>(), any::<u64>()).prop_map(|(seq, hash)| ToTools::Loaded { seq, hash }),
            (any::<u64>(), proptest::option::of(any::<u64>()), any_entry())
                .prop_map(|(seq, request, entry)| ToTools::Committed { seq, request, entry }),
            (any::<u64>(), ".*").prop_map(|(request, reason)| ToTools::Rejected { request, reason }),
            (
                any::<u64>(),
                proptest::collection::vec(any::<u64>(), 0..4),
                prop_oneof![Just(PickMode::Replace), Just(PickMode::Add), Just(PickMode::Toggle)]
            )
                .prop_map(|(comp, nodes, mode)| ToTools::Picked {
                    comp: backstage_core::CompId::from_raw(comp),
                    nodes: nodes.into_iter().map(NodeId::from_raw).collect(),
                    mode,
                }),
            (-1e6f32..1e6, -1e6f32..1e6, 0.0f32..100.0)
                .prop_map(|(x, y, scale)| ToTools::Framing { origin: (x, y), scale }),
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

    /// Why document data goes as RON text: core types skip default fields
    /// when serializing, and postcard can't tell a field was skipped.
    #[test]
    fn core_types_do_not_survive_postcard() {
        let key = backstage_core::Key::new(
            backstage_core::Time::ZERO,
            backstage_core::Value::Number(1.0),
            backstage_core::Ease::Linear,
        );
        let bytes = postcard::to_stdvec(&key).unwrap();
        assert!(postcard::from_bytes::<backstage_core::Key>(&bytes).is_err());

        let entry = Entry::Do(Command::SetKey {
            comp: ids::BALL,
            anim: ids::BALL_BOUNCE,
            node: ids::BALL_BODY,
            property: backstage_core::Property::Y,
            key,
        });
        let msg = ToStage::Submit { request: 1, entry };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn picks_replace_add_and_toggle_with_the_last_picked_primary() {
        use ids::{EYES, FREE_BALL, GROUND};
        let current = [GROUND, EYES];
        assert_eq!(PickMode::Replace.apply(&current, &[FREE_BALL]), [FREE_BALL]);
        assert_eq!(PickMode::Replace.apply(&current, &[]), []);
        assert_eq!(
            PickMode::Add.apply(&current, &[FREE_BALL, GROUND]),
            [GROUND, EYES, FREE_BALL],
            "no duplicates"
        );
        assert_eq!(PickMode::Add.apply(&current, &[]), current);
        assert_eq!(PickMode::Toggle.apply(&current, &[GROUND]), [EYES], "selected: out");
        assert_eq!(
            PickMode::Toggle.apply(&current, &[FREE_BALL]),
            [GROUND, EYES, FREE_BALL],
            "new: in, primary"
        );
        assert_eq!(PickMode::Toggle.apply(&[], &[GROUND, GROUND]), [], "twice: in and out again");
    }

    #[test]
    fn snapshots_carry_the_whole_project() {
        let project = sample::bounce();
        let msg = ToStage::Load { base: Snapshot::of(&project).unwrap(), log: vec![Entry::Undo] };
        let ToStage::Load { base, log } = round_trip(&msg) else { panic!() };
        assert_eq!(base.project().unwrap(), project);
        assert_eq!(log, vec![Entry::Undo]);

        let broken = Snapshot { files: vec![] };
        assert!(broken.project().is_err());
    }

    #[test]
    fn oversized_messages_are_not_sent() {
        let mut buf = Vec::new();
        let err = write_message(&mut buf, &ToTools::Log("x".repeat(MAX_MESSAGE_LEN as usize))).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(buf.is_empty());
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
            ToStage::Pointer(PointerEvent::Move(PointerAt::new(10.5, 20.0))),
            ToStage::Pointer(PointerEvent::Down(PointerAt {
                x: 1.0,
                y: 2.0,
                modifiers: Modifiers { shift: true, ctrl: false, alt: true },
            })),
            ToStage::Pointer(PointerEvent::Up(PointerAt::new(3.0, 4.0))),
            ToStage::Pointer(PointerEvent::Leave),
            ToStage::CancelGesture,
            ToStage::Shutdown,
            ToStage::Load {
                base: Snapshot { files: vec![("project.ron".into(), "()".into())] },
                log: vec![Entry::Undo, Entry::Redo],
            },
            ToStage::Submit { request: 7, entry: Entry::Undo },
            ToStage::Transport {
                composition: Some(ids::BALL),
                animation: Some(ids::STAGE_MAIN),
                time: backstage_core::Time::from_ratio(1, 2),
                playing: true,
            },
            ToStage::Selection { comp: ids::STAGE, nodes: vec![ids::GROUND, ids::EYES] },
            ToStage::Transport {
                composition: None,
                animation: None,
                time: backstage_core::Time::ZERO,
                playing: false,
            },
        ] {
            out += &format!("{msg:?} => {}\n", hex(&msg));
        }
        for msg in [
            ToTools::Hello { version: PROTOCOL_VERSION, pid: 4242, adapter: "gpu".into() },
            ToTools::Surface { generation: 2, path: "/run/x".into(), width: 640, height: 480, stride: 2560 },
            ToTools::FrameReady { generation: 2, slot: 1, seq: 300 },
            ToTools::Log("hi".into()),
            ToTools::Heartbeat,
            ToTools::Loaded { seq: 3, hash: 0xfeed },
            ToTools::Committed { seq: 4, request: Some(7), entry: Entry::Undo },
            ToTools::Committed { seq: 5, request: None, entry: Entry::Redo },
            ToTools::Rejected { request: 8, reason: "no".into() },
            ToTools::Picked { comp: ids::STAGE, nodes: vec![ids::GROUND], mode: PickMode::Replace },
            ToTools::Picked { comp: ids::STAGE, nodes: vec![], mode: PickMode::Add },
            ToTools::Picked { comp: ids::BALL, nodes: vec![ids::BALL_BODY], mode: PickMode::Toggle },
            ToTools::Framing { origin: (24.0, 31.5), scale: 0.88 },
        ] {
            out += &format!("{msg:?} => {}\n", hex(&msg));
        }
        insta::assert_snapshot!(out);
    }
}
