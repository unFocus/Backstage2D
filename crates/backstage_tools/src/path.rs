//! Which composition the editor is editing, as a breadcrumb path from the
//! root: `[Stage, Ball]` means Ball, entered from the stage. Editor session
//! state, not part of the document.

use backstage_core::{CompId, Project};

/// The path for editing `comp`, entered from the end of `path`. Entering a
/// composition already on the path goes back to it instead, so recursive
/// entries can't grow the path forever.
pub fn enter(path: &[CompId], comp: CompId) -> Vec<CompId> {
    match path.iter().position(|&c| c == comp) {
        Some(i) => path[..=i].to_vec(),
        None => path.iter().copied().chain([comp]).collect(),
    }
}

/// The path cut back to `depth + 1` entries (0 is the root).
pub fn go_to(path: &[CompId], depth: usize) -> Vec<CompId> {
    path[..(depth + 1).min(path.len())].to_vec()
}

/// `path`, starting at the project's root and cut back before the first
/// composition that no longer exists.
pub fn prune(path: &[CompId], project: &Project) -> Vec<CompId> {
    let mut out = vec![project.root];
    out.extend(path.iter().skip(1).take_while(|c| project.compositions.contains_key(c)));
    out
}

/// The crumbs' labels: each composition's name.
pub fn crumbs(path: &[CompId], project: &Project) -> Vec<String> {
    path.iter()
        .map(|c| project.compositions.get(c).map_or_else(|| "?".to_owned(), |c| c.name.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::sample::{self, ids::*};

    #[test]
    fn entering_goes_deeper_and_crumbs_go_back() {
        let root = vec![STAGE];
        let ball = enter(&root, BALL);
        assert_eq!(ball, [STAGE, BALL]);
        let deeper = enter(&ball, BLINKER);
        assert_eq!(deeper, [STAGE, BALL, BLINKER]);
        assert_eq!(go_to(&deeper, 0), [STAGE]);
        assert_eq!(go_to(&deeper, 1), [STAGE, BALL]);
        assert_eq!(go_to(&deeper, 9), deeper);
        assert_eq!(enter(&deeper, BALL), [STAGE, BALL], "already on the path: back to it");
        assert_eq!(enter(&deeper, STAGE), [STAGE]);
    }

    #[test]
    fn pruning_keeps_the_root_and_drops_missing_compositions() {
        let mut project = sample::bounce();
        let path = vec![STAGE, BALL, BLINKER];
        assert_eq!(prune(&path, &project), path);
        project.compositions.remove(&BALL);
        assert_eq!(prune(&path, &project), [STAGE], "everything after a missing one goes");
        assert_eq!(prune(&[], &project), [STAGE]);
    }

    #[test]
    fn crumbs_are_composition_names() {
        let project = sample::bounce();
        assert_eq!(crumbs(&[STAGE, BALL], &project), ["Stage", "Ball"]);
    }
}
