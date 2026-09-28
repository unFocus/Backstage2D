//! Sample projects, built in code with fixed IDs so their files are stable.
//! `samples/bounce.bs2d/` in the repo is `bounce()` saved to disk; a test
//! keeps them in sync.

use crate::animation::{Animation, Ease, EasePreset, Key, LoopMode, Marker, Property, Track, Value};
use crate::geom::{Color, Vec2};
use crate::id::{AnimId, CompId, DrawingId, NodeId};
use crate::node::{Drawing, Instance, Node, NodeKind, Props, Repeat, TimeMode};
use crate::project::{Composition, EditorPrefs, Project, ProjectSettings};
use crate::shape::{Paint, Shape, Stroke};
use crate::time::Time;
use std::collections::BTreeMap;

pub mod ids {
    use super::*;

    pub const STAGE: CompId = CompId::from_raw(0x5747_0001);
    pub const BALL: CompId = CompId::from_raw(0xba11_0001);
    pub const BLINKER: CompId = CompId::from_raw(0xb117_0001);

    pub const STAGE_ROOT: NodeId = NodeId::from_raw(0x5747_1000);
    pub const GROUND: NodeId = NodeId::from_raw(0x5747_1001);
    pub const FREE_BALL: NodeId = NodeId::from_raw(0x5747_1002);
    pub const SYNCED_BALL: NodeId = NodeId::from_raw(0x5747_1003);
    pub const EYES: NodeId = NodeId::from_raw(0x5747_1004);
    pub const STAGE_MAIN: AnimId = AnimId::from_raw(0x5747_2000);

    pub const BALL_ROOT: NodeId = NodeId::from_raw(0xba11_1000);
    pub const BALL_BODY: NodeId = NodeId::from_raw(0xba11_1001);
    pub const BALL_BOUNCE: AnimId = AnimId::from_raw(0xba11_2000);
    pub const BALL_SQUASH: AnimId = AnimId::from_raw(0xba11_2001);

    pub const BLINKER_ROOT: NodeId = NodeId::from_raw(0xb117_1000);
    pub const BLINKER_EYE: NodeId = NodeId::from_raw(0xb117_1001);
    pub const BLINKER_BLINK: AnimId = AnimId::from_raw(0xb117_2000);
}

use ids::*;

fn secs(num: i64, den: i64) -> Time {
    Time::from_ratio(num, den)
}

fn num(at: Time, v: f32, ease: Ease) -> Key {
    Key::new(at, Value::Number(v), ease)
}

/// A bouncing ball on a 550×400 stage:
/// - `Ball`: a circle with a looping `bounce` (eased Y) and a one-shot
///   `squash` (ScaleX/ScaleY), with a `land` marker.
/// - `Blinker`: a flipbook of three eye drawings, animated with hold keys.
/// - The stage: ground, a free-running ball, a synced ball whose clock is
///   offset by 0.25 s and that slides across over the stage's `main`
///   animation, and a synced blinker.
pub fn bounce() -> Project {
    let compositions = [stage(), ball(), blinker()].into_iter().map(|c| (c.id, c)).collect();
    Project {
        settings: ProjectSettings::default(),
        editor: EditorPrefs::default(),
        root: STAGE,
        compositions,
        assets: BTreeMap::new(),
    }
}

fn stage() -> Composition {
    let ease_in_out = Ease::Preset(EasePreset::EaseInOut);
    let nodes = BTreeMap::from([
        (
            STAGE_ROOT,
            Node::new("stage", NodeKind::Group).with_children(vec![GROUND, EYES, FREE_BALL, SYNCED_BALL]),
        ),
        (
            GROUND,
            Node::new(
                "ground",
                NodeKind::Shape(Shape::rect(
                    Vec2::new(0.0, 340.0),
                    Vec2::new(550.0, 60.0),
                    Some(Paint::Solid(Color::rgb8(0x6a, 0xbf, 0x4b))),
                    None,
                )),
            ),
        ),
        (
            FREE_BALL,
            Node::new(
                "free ball",
                NodeKind::Instance(Instance {
                    comp: BALL,
                    time: TimeMode::Free { animation: Some(BALL_BOUNCE) },
                }),
            )
            .with_rest(Props::at(400.0, 300.0)),
        ),
        (
            SYNCED_BALL,
            Node::new(
                "synced ball",
                NodeKind::Instance(Instance {
                    comp: BALL,
                    time: TimeMode::Synced {
                        animation: BALL_BOUNCE,
                        offset: secs(1, 4),
                        repeat: Repeat::Natural,
                    },
                }),
            )
            .with_rest(Props::at(120.0, 300.0)),
        ),
        (
            EYES,
            Node::new(
                "eyes",
                NodeKind::Instance(Instance {
                    comp: BLINKER,
                    time: TimeMode::Synced {
                        animation: BLINKER_BLINK,
                        offset: Time::ZERO,
                        repeat: Repeat::Natural,
                    },
                }),
            )
            .with_rest(Props::at(275.0, 90.0)),
        ),
    ]);
    let mut main = Animation::new("main", Time::from_secs(4), LoopMode::PingPong);
    main.tracks.push(Track {
        node: SYNCED_BALL,
        property: Property::X,
        keys: vec![num(Time::ZERO, 120.0, ease_in_out), num(Time::from_secs(4), 260.0, Ease::Linear)],
    });
    main.markers.push(Marker { name: "start".into(), at: Time::ZERO });
    Composition {
        id: STAGE,
        name: "Stage".into(),
        root: STAGE_ROOT,
        nodes,
        animations: BTreeMap::from([(STAGE_MAIN, main)]),
        default_animation: Some(STAGE_MAIN),
    }
}

fn ball() -> Composition {
    let nodes = BTreeMap::from([
        (BALL_ROOT, Node::new("ball", NodeKind::Group).with_children(vec![BALL_BODY])),
        (
            BALL_BODY,
            Node::new(
                "body",
                NodeKind::Shape(Shape::ellipse(
                    Vec2::ZERO,
                    Vec2::splat(30.0),
                    Some(Paint::RadialGradient {
                        center: Vec2::new(-10.0, -10.0),
                        radius: 40.0,
                        stops: vec![
                            crate::shape::GradientStop { offset: 0.0, color: Color::rgb8(0xff, 0xd0, 0x80) },
                            crate::shape::GradientStop { offset: 1.0, color: Color::rgb8(0xe8, 0x6a, 0x10) },
                        ],
                    }),
                    Some(Stroke::solid(Color::rgb8(0x5a, 0x2a, 0x05), 2.0)),
                )),
            ),
        ),
    ]);
    // The body's origin is its center; y = 0 is resting on the ground.
    let mut bounce = Animation::new("bounce", Time::from_secs(1), LoopMode::Loop);
    bounce.tracks.push(Track {
        node: BALL_BODY,
        property: Property::Y,
        keys: vec![
            num(Time::ZERO, -180.0, Ease::Preset(EasePreset::EaseIn)),
            num(secs(1, 2), 0.0, Ease::Preset(EasePreset::EaseOut)),
            num(Time::from_secs(1), -180.0, Ease::Linear),
        ],
    });
    bounce.markers.push(Marker { name: "land".into(), at: secs(1, 2) });

    let mut squash = Animation::new("squash", secs(3, 10), LoopMode::Once);
    let squash_curve = |wide: f32| {
        vec![
            num(Time::ZERO, 1.0, Ease::Preset(EasePreset::EaseOut)),
            num(secs(1, 10), wide, Ease::Preset(EasePreset::EaseInOut)),
            num(secs(3, 10), 1.0, Ease::Linear),
        ]
    };
    squash.tracks.push(Track { node: BALL_BODY, property: Property::ScaleX, keys: squash_curve(1.3) });
    squash.tracks.push(Track { node: BALL_BODY, property: Property::ScaleY, keys: squash_curve(0.7) });
    squash.step = Some(secs(1, 30));

    Composition {
        id: BALL,
        name: "Ball".into(),
        root: BALL_ROOT,
        nodes,
        animations: BTreeMap::from([(BALL_BOUNCE, bounce), (BALL_SQUASH, squash)]),
        default_animation: Some(BALL_BOUNCE),
    }
}

fn blinker() -> Composition {
    let eye = |height: f32| {
        Shape::ellipse(Vec2::ZERO, Vec2::new(12.0, height), Some(Paint::Solid(Color::BLACK)), None)
    };
    let drawings = vec![
        Drawing { id: DrawingId::from_raw(0xb117_3000), shape: eye(16.0) },
        Drawing { id: DrawingId::from_raw(0xb117_3001), shape: eye(8.0) },
        Drawing { id: DrawingId::from_raw(0xb117_3002), shape: eye(1.5) },
    ];
    let nodes = BTreeMap::from([
        (BLINKER_ROOT, Node::new("blinker", NodeKind::Group).with_children(vec![BLINKER_EYE])),
        (BLINKER_EYE, Node::new("eye", NodeKind::Flipbook(drawings))),
    ]);
    let index = |at: Time, i: u32| Key::new(at, Value::Index(i), Ease::Hold);
    let mut blink = Animation::new("blink", Time::from_secs(2), LoopMode::Loop);
    blink.tracks.push(Track {
        node: BLINKER_EYE,
        property: Property::Drawing,
        keys: vec![
            index(Time::ZERO, 0),
            index(secs(3, 2), 1),
            index(secs(19, 12), 2),
            index(secs(5, 3), 1),
            index(secs(7, 4), 0),
        ],
    });
    Composition {
        id: BLINKER,
        name: "Blinker".into(),
        root: BLINKER_ROOT,
        nodes,
        animations: BTreeMap::from([(BLINKER_BLINK, blink)]),
        default_animation: Some(BLINKER_BLINK),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn checked_in() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/bounce.bs2d")
    }

    #[test]
    fn sample_is_valid() {
        bounce().validate().unwrap();
    }

    /// The checked-in sample doubles as the file-format snapshot: any format
    /// change shows up as a diff of these files. Rewrite them with
    /// `BACKSTAGE_BLESS=1 cargo test -p backstage_core sample_matches`.
    #[test]
    fn sample_matches_checked_in_files() {
        let project = bounce();
        if std::env::var("BACKSTAGE_BLESS").is_ok_and(|v| v == "1") {
            crate::io::save(&project, &checked_in()).unwrap();
            return;
        }
        let tmp = crate::test_util::tempdir("sample");
        crate::io::save(&project, &tmp).unwrap();
        let listing = |dir: &Path| -> Vec<(PathBuf, String)> {
            let mut files = Vec::new();
            for sub in [dir.to_owned(), dir.join("compositions")] {
                for entry in std::fs::read_dir(&sub).unwrap().flatten() {
                    if entry.path().is_file() {
                        let rel = entry.path().strip_prefix(dir).unwrap().to_owned();
                        files.push((rel, std::fs::read_to_string(entry.path()).unwrap()));
                    }
                }
            }
            files.sort();
            files
        };
        let (expected, actual) = (listing(&checked_in()), listing(&tmp));
        let names = |f: &[(PathBuf, String)]| f.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();
        assert_eq!(names(&actual), names(&expected), "file list differs (bless to update)");
        for ((path, want), (_, got)) in expected.iter().zip(&actual) {
            assert!(
                want == got,
                "{} differs (bless to update):\n--- checked in\n{want}\n--- generated\n{got}",
                path.display()
            );
        }
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn checked_in_sample_loads() {
        assert_eq!(crate::io::load(&checked_in()).unwrap(), bounce());
    }
}
