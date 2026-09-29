//! The document: a project plus its undo history, changed only by
//! [`Entry`]s.
//!
//! The stage applies each entry and numbers it; the tools process applies
//! the same entries in the same order to its copy. Undo and redo are entries
//! too, so both copies agree on the project *and* the history, and a stage
//! rebuilt by replaying the log can still undo. See
//! `docs/adr/0002-stage-process-isolation.md`.

use crate::command::{Command, CommandError};
use crate::io;
use crate::project::Project;
use serde::{Deserialize, Serialize};

/// One step in the command log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Entry {
    Do(Command),
    Undo,
    Redo,
}

impl Entry {
    /// The entry as one line of RON, for the command log.
    pub fn to_line(&self) -> String {
        io::to_ron_line(self).expect("entries always serialize")
    }

    pub fn from_line(line: &str) -> Result<Self, ron::error::SpannedError> {
        io::from_ron_line(line)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EntryError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("log entry {index}: {error}")]
pub struct ReplayError {
    pub index: usize,
    pub error: EntryError,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    project: Project,
    /// Inverses of the edits made, newest last.
    undo: Vec<Command>,
    /// Inverses of the undos made, newest last. Cleared by a new edit.
    redo: Vec<Command>,
    /// How many entries have been applied.
    seq: u64,
}

impl Document {
    /// A document with no history. `project` should be valid.
    pub fn new(project: Project) -> Self {
        Self { project, undo: Vec::new(), redo: Vec::new(), seq: 0 }
    }

    /// Builds the document that applying `entries` to `project` gives.
    pub fn replay<'a>(
        project: Project,
        entries: impl IntoIterator<Item = &'a Entry>,
    ) -> Result<Self, ReplayError> {
        let mut doc = Self::new(project);
        for (index, entry) in entries.into_iter().enumerate() {
            doc.apply(entry).map_err(|error| ReplayError { index, error })?;
        }
        Ok(doc)
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The number of entries applied so far, which is also the sequence
    /// number of the last one.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Applies an entry and returns its sequence number. On error nothing
    /// changes, including the sequence number.
    pub fn apply(&mut self, entry: &Entry) -> Result<u64, EntryError> {
        match entry {
            Entry::Do(command) => {
                let inverse = command.apply(&mut self.project)?;
                self.undo.push(inverse);
                self.redo.clear();
            }
            Entry::Undo => {
                let inverse = self.undo.last().ok_or(EntryError::NothingToUndo)?;
                let redo = inverse.apply(&mut self.project)?;
                self.undo.pop();
                self.redo.push(redo);
            }
            Entry::Redo => {
                let redo = self.redo.last().ok_or(EntryError::NothingToRedo)?;
                let inverse = redo.apply(&mut self.project)?;
                self.redo.pop();
                self.undo.push(inverse);
            }
        }
        self.seq += 1;
        Ok(self.seq)
    }

    /// A fingerprint of the whole document (project, history, and sequence
    /// number), taken over its canonical RON. Two processes compare these
    /// to check that they hold the same document. Stable across runs and
    /// builds, since it hashes the file format rather than memory.
    pub fn hash(&self) -> u64 {
        let mut h = Fnv1a::default();
        h.write(&self.seq.to_le_bytes());
        for (path, text) in io::to_files(&self.project).expect("projects always serialize") {
            h.write(path.to_string_lossy().as_bytes());
            h.write(&[0]);
            h.write(text.as_bytes());
            h.write(&[0]);
        }
        for (name, stack) in [("undo", &self.undo), ("redo", &self.redo)] {
            h.write(name.as_bytes());
            for command in stack {
                h.write(io::to_ron_line(command).expect("commands always serialize").as_bytes());
                h.write(&[0]);
            }
        }
        h.0
    }
}

/// 64-bit FNV-1a: tiny, and defined by a spec, unlike std's `DefaultHasher`.
struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv1a {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Document, Entry, EntryError, ReplayError};
    use crate::command::{Command, CommandError};
    use crate::node::Props;
    use crate::sample::{self, ids::*};
    use crate::test_util::{pick, seed};
    use proptest::prelude::*;

    fn nudge(x: f32) -> Entry {
        Entry::Do(Command::SetRest { comp: STAGE, node: GROUND, rest: Props::at(x, 0.0) })
    }

    fn ground_x(doc: &Document) -> f32 {
        doc.project().compositions[&STAGE].nodes[&GROUND].rest.transform.position.x
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut doc = Document::new(sample::bounce());
        let start = ground_x(&doc);
        assert!(!doc.can_undo() && !doc.can_redo());

        assert_eq!(doc.apply(&nudge(10.0)), Ok(1));
        assert_eq!(doc.apply(&nudge(20.0)), Ok(2));
        assert_eq!(ground_x(&doc), 20.0);

        assert_eq!(doc.apply(&Entry::Undo), Ok(3));
        assert_eq!(ground_x(&doc), 10.0);
        doc.apply(&Entry::Undo).unwrap();
        assert_eq!(ground_x(&doc), start);
        assert!(!doc.can_undo() && doc.can_redo());

        doc.apply(&Entry::Redo).unwrap();
        doc.apply(&Entry::Redo).unwrap();
        assert_eq!(ground_x(&doc), 20.0);
        assert!(doc.can_undo() && !doc.can_redo());
        assert_eq!(doc.seq(), 6);
    }

    #[test]
    fn a_new_edit_discards_redo() {
        let mut doc = Document::new(sample::bounce());
        doc.apply(&nudge(10.0)).unwrap();
        doc.apply(&Entry::Undo).unwrap();
        assert!(doc.can_redo());
        doc.apply(&nudge(30.0)).unwrap();
        assert!(!doc.can_redo());
        assert_eq!(doc.apply(&Entry::Redo), Err(EntryError::NothingToRedo));
    }

    #[test]
    fn rejected_entries_change_nothing() {
        let mut doc = Document::new(sample::bounce());
        let before = doc.clone();
        assert_eq!(doc.apply(&Entry::Undo), Err(EntryError::NothingToUndo));
        assert_eq!(doc.apply(&Entry::Redo), Err(EntryError::NothingToRedo));
        let err = doc.apply(&Entry::Do(Command::RemoveNode { comp: STAGE, node: STAGE_ROOT })).unwrap_err();
        assert!(matches!(err, EntryError::Command(CommandError::RootNode { .. })));
        assert_eq!(doc, before);
        assert_eq!(doc.hash(), before.hash());
    }

    #[test]
    fn replay_reports_the_failing_entry() {
        let log = [nudge(1.0), Entry::Undo, Entry::Undo];
        let err = Document::replay(sample::bounce(), &log).unwrap_err();
        assert_eq!(err, ReplayError { index: 2, error: EntryError::NothingToUndo });
        assert_eq!(err.to_string(), "log entry 2: nothing to undo");
    }

    #[test]
    fn hash_covers_project_history_and_seq() {
        let base = Document::new(sample::bounce());
        let mut edited = base.clone();
        edited.apply(&nudge(10.0)).unwrap();
        assert_ne!(edited.hash(), base.hash(), "project changed");

        // Same project as `base` after the undo, but a different history.
        let mut undone = edited.clone();
        undone.apply(&Entry::Undo).unwrap();
        assert_eq!(undone.project(), base.project());
        assert_ne!(undone.hash(), base.hash());

        assert_eq!(base.hash(), Document::new(sample::bounce()).hash(), "deterministic");
    }

    #[test]
    fn entries_are_single_ron_lines() {
        let entry = nudge(12.5);
        let line = entry.to_line();
        assert!(!line.contains('\n'), "{line}");
        assert!(line.starts_with("Do(SetRest("), "{line}");
        assert!(line.contains("comp_") && line.contains("node_"), "IDs in their text form: {line}");
        assert_eq!(Entry::from_line(&line).unwrap(), entry);
        assert_eq!(Entry::from_line("Undo").unwrap(), Entry::Undo);
        assert!(Entry::from_line("Do(Nonsense)").is_err());
    }

    fn any_entry_kind() -> impl Strategy<Value = u8> {
        // Mostly edits, some undos and redos.
        prop_oneof![6 => Just(0u8), 2 => Just(1u8), 1 => Just(2u8)]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Replaying the accepted entries onto the base project gives an
        /// identical document, and every entry survives the log's text form.
        #[test]
        fn replay_gives_an_identical_document(
            steps in proptest::collection::vec((any_entry_kind(), seed()), 1..40)
        ) {
            let base = sample::bounce();
            let mut live = Document::new(base.clone());
            let mut log = Vec::new();
            for (kind, seed) in steps {
                let entry = match kind {
                    0 => Entry::Do(pick(live.project(), &seed)),
                    1 => Entry::Undo,
                    _ => Entry::Redo,
                };
                let before = live.clone();
                match live.apply(&entry) {
                    Ok(seq) => {
                        prop_assert_eq!(seq, before.seq() + 1);
                        let parsed = Entry::from_line(&entry.to_line());
                        prop_assert_eq!(parsed.as_ref(), Ok(&entry));
                        log.push(entry);
                    }
                    Err(_) => prop_assert_eq!(&live, &before),
                }
                prop_assert!(live.project().validate().is_ok());
            }
            let replayed = Document::replay(base, &log).unwrap();
            prop_assert_eq!(replayed.hash(), live.hash());
            prop_assert_eq!(replayed, live);
        }
    }
}
