//! Editor-only node state: hide, lock, and outline ([`NodeFlags`]). These
//! are saved with the project but never affect playback. The editor's
//! stage applies them to what it draws; the player ignores them.

use crate::geom::Color;
use crate::id::NodeId;
use crate::node::NodeFlags;
use crate::project::Composition;
use std::collections::BTreeMap;

/// Each node's flags combined with its ancestors': a node is hidden,
/// locked, or outlined if it or any ancestor is. Nodes not reachable from
/// the root are left out.
pub fn effective(comp: &Composition) -> BTreeMap<NodeId, NodeFlags> {
    let mut out = BTreeMap::new();
    let mut stack = vec![(comp.root, NodeFlags::default())];
    while let Some((id, inherited)) = stack.pop() {
        let Some(node) = comp.nodes.get(&id) else { continue };
        let own = node.editor;
        let flags = NodeFlags {
            hidden: inherited.hidden || own.hidden,
            locked: inherited.locked || own.locked,
            outline: inherited.outline || own.outline,
        };
        out.insert(id, flags);
        stack.extend(node.children.iter().map(|&c| (c, flags)));
    }
    out
}

/// The outline colour of every outlined node: the colour of the nearest
/// node (itself or an ancestor) that has outline turned on, so a group's
/// subtree is drawn in the group's colour.
pub fn outline_colors(comp: &Composition) -> BTreeMap<NodeId, Color> {
    let mut out = BTreeMap::new();
    let mut stack: Vec<(NodeId, Option<Color>)> = vec![(comp.root, None)];
    while let Some((id, inherited)) = stack.pop() {
        let Some(node) = comp.nodes.get(&id) else { continue };
        let color = if node.editor.outline { Some(outline_color(id)) } else { inherited };
        if let Some(c) = color {
            out.insert(id, c);
        }
        stack.extend(node.children.iter().map(|&c| (c, color)));
    }
    out
}

/// The colour a node's outline is drawn in, like Flash's per-layer outline
/// colours: stable for a node, and varied between nodes.
pub fn outline_color(node: NodeId) -> Color {
    const PALETTE: [Color; 8] = [
        Color::rgb8(0x2f, 0x6f, 0xff),
        Color::rgb8(0xff, 0x40, 0x40),
        Color::rgb8(0x1f, 0xb3, 0x4a),
        Color::rgb8(0xff, 0x8c, 0x00),
        Color::rgb8(0xb0, 0x3c, 0xff),
        Color::rgb8(0x00, 0xb5, 0xc8),
        Color::rgb8(0xe0, 0x2c, 0x9a),
        Color::rgb8(0x8a, 0x6a, 0x00),
    ];
    // Mix the bits, so neighbouring IDs get different colours.
    let mixed = node.raw().wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 61;
    PALETTE[mixed as usize % PALETTE.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{self, ids::*};

    #[test]
    fn flags_are_inherited_down_the_tree() {
        let mut project = sample::bounce();
        let stage = project.compositions.get_mut(&STAGE).unwrap();
        stage.nodes.get_mut(&STAGE_ROOT).unwrap().editor.locked = true;
        stage.nodes.get_mut(&GROUND).unwrap().editor.hidden = true;
        let flags = effective(&project.compositions[&STAGE]);
        assert!(flags.values().all(|f| f.locked), "the root's lock covers everything");
        assert!(flags[&GROUND].hidden);
        assert!(!flags[&EYES].hidden, "a sibling isn't affected");
        assert!(!flags.values().any(|f| f.outline));
        assert_eq!(flags.len(), project.compositions[&STAGE].nodes.len());

        // Each flag is inherited on its own.
        for set in [
            |f: &mut NodeFlags| f.hidden = true,
            |f: &mut NodeFlags| f.locked = true,
            |f: &mut NodeFlags| f.outline = true,
        ] {
            let mut project = sample::bounce();
            let stage = project.compositions.get_mut(&STAGE).unwrap();
            set(&mut stage.nodes.get_mut(&STAGE_ROOT).unwrap().editor);
            let mut expected = NodeFlags::default();
            set(&mut expected);
            assert!(effective(&project.compositions[&STAGE]).values().all(|f| *f == expected));
        }
    }

    #[test]
    fn nothing_set_means_no_flags() {
        let project = sample::bounce();
        assert!(effective(&project.compositions[&STAGE]).values().all(|f| *f == NodeFlags::default()));
    }

    #[test]
    fn an_outlined_node_colours_its_subtree() {
        let mut project = sample::bounce();
        let stage = project.compositions.get_mut(&STAGE).unwrap();
        stage.nodes.get_mut(&GROUND).unwrap().children = vec![EYES];
        stage.nodes.get_mut(&STAGE_ROOT).unwrap().children.retain(|&n| n != EYES);
        stage.nodes.get_mut(&GROUND).unwrap().editor.outline = true;
        let colors = outline_colors(&project.compositions[&STAGE]);
        assert_eq!(colors.len(), 2);
        assert_eq!(colors[&GROUND], outline_color(GROUND));
        assert_eq!(colors[&EYES], outline_color(GROUND), "the group's colour, not its own");
    }

    #[test]
    fn outline_colors_are_stable_and_varied() {
        assert_eq!(outline_color(GROUND), outline_color(GROUND));
        let colors: std::collections::BTreeSet<_> =
            (0..64u64).map(|i| outline_color(NodeId::from_raw(0x5747_1000 + i)).to_rgba8()).collect();
        assert!(colors.len() >= 6, "only {} colours for 64 neighbouring IDs", colors.len());
    }
}
