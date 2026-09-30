//! The editor's copy of the document.
//!
//! The stage is the only writer: the editor submits entries and applies
//! only what the stage reports as committed, in sequence order. The copy
//! keeps every committed entry, sends them all with `Load` when a stage
//! (re)starts, and appends each one to a recovery log on disk. Saving writes
//! the project and marks the recovery log as saved. See
//! `docs/adr/0004-commands-and-document-authority.md`.

use crate::recovery::{self, Recovery, RecoveryMeta};
use backstage_core::{Document, Entry, Project, SaveError};
use backstage_protocol::{Snapshot, ToStage};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
/// A project directory's extension.
pub const PROJECT_EXTENSION: &str = "bs2d";

#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    /// The copy no longer matches the stage. Nothing was applied; restart
    /// the stage so it replays the copy's log.
    #[error("out of sync with the stage: {0}")]
    OutOfSync(String),
    /// The entry *was* applied, but writing it to disk failed. Autosave is
    /// off from here on; the copy in memory still covers stage crashes.
    #[error("autosave failed: {0}")]
    AutosaveFailed(io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SaveToError {
    /// Nothing was saved; the copy still counts as unsaved.
    #[error(transparent)]
    Project(#[from] SaveError),
    /// The project *was* saved, but marking the recovery log failed.
    /// Autosave is off from here on.
    #[error("saved, but autosave failed: {0}")]
    AutosaveFailed(io::Error),
}

pub struct WorkingCopy {
    /// The project as opened, sent with every `Load`.
    base: Snapshot,
    /// Always `Document::replay(base, log)`.
    doc: Document,
    log: Vec<Entry>,
    /// Where the project is saved; `None` until the first Save As.
    path: Option<PathBuf>,
    /// The project as last opened or saved.
    saved: Project,
    /// A `Load` was sent and its `Loaded` hasn't been checked yet. Commits
    /// arriving meanwhile belong to the stage's previous document.
    awaiting_load: bool,
    next_request: u64,
    recovery: Option<Recovery>,
}

impl WorkingCopy {
    /// `path` is where `project` was loaded from, if anywhere.
    pub fn new(
        project: Project,
        path: Option<PathBuf>,
        recovery: Option<Recovery>,
    ) -> Result<Self, SaveError> {
        Ok(Self {
            base: Snapshot::of(&project)?,
            saved: project.clone(),
            doc: Document::new(project),
            log: Vec::new(),
            path,
            awaiting_load: false,
            next_request: 1,
            recovery,
        })
    }

    /// Takes over an orphaned recovery directory (see
    /// [`recovery::scan`]): the document comes back with its undo history,
    /// its path, and its saved state, and new edits go on being appended
    /// to the same directory.
    pub fn restore(dir: &Path) -> Result<Self, String> {
        let (recovery, recovered) = Recovery::resume(dir)?;
        let (doc, saved) = recovered.replay()?;
        Ok(Self {
            base: Snapshot::of(&recovered.base).map_err(|e| e.to_string())?,
            doc,
            log: recovered.log,
            path: recovered.meta.source,
            saved,
            awaiting_load: false,
            next_request: 1,
            recovery: Some(recovery),
        })
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The project directory's name without its extension, or "Untitled".
    pub fn display_name(&self) -> String {
        recovery::project_name(self.path.as_deref())
    }

    /// The project differs from the one last opened or saved. Undoing back
    /// to the saved state makes it clean again.
    pub fn is_dirty(&self) -> bool {
        self.doc.project() != &self.saved
    }

    /// The directory the log is autosaved to, if autosave is on.
    pub fn recovery_dir(&self) -> Option<&Path> {
        self.recovery.as_ref().map(Recovery::dir)
    }

    /// What to send a stage that just connected: the base and every
    /// committed entry, so it rebuilds this exact document. Commits are
    /// ignored until [`check_loaded`](Self::check_loaded) accepts the reply.
    pub fn load_message(&mut self) -> ToStage {
        self.awaiting_load = true;
        ToStage::Load { base: self.base.clone(), log: self.log.clone() }
    }

    /// A request to apply `entry`. The copy changes only when the stage
    /// commits it.
    pub fn submit(&mut self, entry: Entry) -> ToStage {
        let request = self.next_request;
        self.next_request += 1;
        ToStage::Submit { request, entry }
    }

    /// Applies an entry the stage committed as `seq`. Returns `false` if it
    /// was ignored because it belongs to the document the stage had before
    /// the last `Load` (after Open, on the same stage).
    pub fn committed(&mut self, seq: u64, entry: &Entry) -> Result<bool, CommitError> {
        if self.awaiting_load {
            return Ok(false);
        }
        let expected = self.doc.seq() + 1;
        if seq != expected {
            return Err(CommitError::OutOfSync(format!("got entry {seq}, expected {expected}")));
        }
        self.doc
            .apply(entry)
            .map_err(|e| CommitError::OutOfSync(format!("entry {seq} doesn't apply here: {e}")))?;
        self.log.push(entry.clone());
        if let Some(recovery) = self.recovery.as_mut()
            && let Err(e) = recovery.append(entry)
        {
            // A log with a hole in it can't be replayed; stop writing.
            self.recovery = None;
            return Err(CommitError::AutosaveFailed(e));
        }
        Ok(true)
    }

    /// A `Load` from [`load_message`](Self::load_message) hasn't been
    /// answered yet. The next `Loaded` from the stage is its answer.
    pub fn awaiting_load(&self) -> bool {
        self.awaiting_load
    }

    /// Checks a (re)started stage's `Loaded` against the copy. Commits are
    /// taken again once it has been checked.
    pub fn check_loaded(&mut self, seq: u64, hash: u64) -> Result<(), String> {
        self.awaiting_load = false;
        let (want_seq, want_hash) = (self.doc.seq(), self.doc.hash());
        if (seq, hash) == (want_seq, want_hash) {
            Ok(())
        } else {
            Err(format!(
                "stage has entry {seq} (hash {hash:016x}), editor has {want_seq} (hash {want_hash:016x})"
            ))
        }
    }

    /// Saves the project into `dir`, which becomes its path, and marks the
    /// recovery log as saved up to here.
    pub fn save_to(&mut self, dir: &Path) -> Result<(), SaveToError> {
        backstage_core::save(self.doc.project(), dir)?;
        self.path = Some(dir.to_owned());
        self.saved = self.doc.project().clone();
        let meta = RecoveryMeta { source: self.path.clone(), saved_seq: self.doc.seq() };
        if let Some(recovery) = &self.recovery
            && let Err(e) = recovery.write_meta(&meta)
        {
            self.recovery = None;
            return Err(SaveToError::AutosaveFailed(e));
        }
        Ok(())
    }

    /// Clean exit. The recovery directory is removed unless it holds
    /// unsaved edits, which it keeps for the restore prompt.
    pub fn finish(self) {
        if !self.is_dirty() {
            self.discard();
        }
    }

    /// Closes the copy and removes its recovery directory, unsaved edits
    /// included (the user chose not to save them).
    pub fn discard(self) {
        if let Some(recovery) = self.recovery {
            recovery.remove();
        }
    }
}

/// Where Save As should write, given the path the user picked: `.bs2d` is
/// added if it's missing. The target must not exist yet, or be a project
/// (or empty) directory the user chose to replace. A name that only exists
/// once `.bs2d` is added is refused, since the dialog never asked about
/// replacing it.
pub fn save_target(picked: &Path) -> Result<PathBuf, String> {
    let has_extension = picked.extension().is_some_and(|e| e == PROJECT_EXTENSION);
    let target = if has_extension {
        picked.to_owned()
    } else {
        let mut name = picked.file_name().ok_or("no file name given")?.to_owned();
        name.push(format!(".{PROJECT_EXTENSION}"));
        picked.with_file_name(name)
    };
    match fs::symlink_metadata(&target) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(target),
        Err(e) => Err(format!("{}: {e}", target.display())),
        Ok(_) if !has_extension => Err(format!("{} already exists", target.display())),
        Ok(meta) if meta.is_dir() => {
            let is_project = target.join("project.ron").is_file();
            let is_empty = fs::read_dir(&target).map_err(|e| e.to_string())?.next().is_none();
            if is_project || is_empty {
                Ok(target)
            } else {
                Err(format!("{} is a folder that isn't a Backstage2D project", target.display()))
            }
        }
        Ok(_) => Err(format!("{} is a file, not a project folder", target.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{LOG_FILE, META_FILE, read_recovery};
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{Command, Props};
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tempdir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "backstage-tools-copy-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn nudge(x: f32) -> Entry {
        Entry::Do(Command::SetRest { comp: STAGE, node: GROUND, rest: Props::at(x, 0.0) })
    }

    fn with_recovery() -> (WorkingCopy, PathBuf) {
        let dir = tempdir();
        let recovery = Recovery::create(dir.clone(), &sample::bounce(), None).unwrap();
        (WorkingCopy::new(sample::bounce(), None, Some(recovery)).unwrap(), dir)
    }

    fn log_text(dir: &Path) -> String {
        fs::read_to_string(dir.join(LOG_FILE)).unwrap()
    }

    #[test]
    fn commits_apply_in_order_and_are_autosaved() {
        let (mut copy, dir) = with_recovery();
        let log = [nudge(1.0), nudge(2.0), Entry::Undo];
        for (i, entry) in log.iter().enumerate() {
            copy.committed(i as u64 + 1, entry).unwrap();
        }
        assert_eq!(copy.document(), &Document::replay(sample::bounce(), &log).unwrap());
        assert_eq!(log_text(&dir).lines().count(), 3);
        let recovered = read_recovery(&dir).unwrap();
        assert_eq!(&Document::replay(recovered.base, &recovered.log).unwrap(), copy.document());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn out_of_order_or_unappliable_commits_change_nothing() {
        let (mut copy, dir) = with_recovery();
        copy.committed(1, &nudge(1.0)).unwrap();
        let before = (copy.document().clone(), copy.log.clone(), log_text(&dir));
        for (seq, entry) in [(3, nudge(2.0)), (1, nudge(2.0)), (2, Entry::Redo)] {
            let err = copy.committed(seq, &entry).unwrap_err();
            assert!(matches!(err, CommitError::OutOfSync(_)), "{err}");
        }
        assert_eq!((copy.document().clone(), copy.log.clone(), log_text(&dir)), before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_carries_the_base_and_every_commit() {
        let mut copy = WorkingCopy::new(sample::bounce(), None, None).unwrap();
        copy.committed(1, &nudge(1.0)).unwrap();
        copy.committed(2, &Entry::Undo).unwrap();
        let ToStage::Load { base, log } = copy.load_message() else { panic!() };
        assert_eq!(base.project().unwrap(), sample::bounce());
        assert_eq!(log, vec![nudge(1.0), Entry::Undo]);
    }

    #[test]
    fn check_loaded_compares_seq_and_hash() {
        let mut copy = WorkingCopy::new(sample::bounce(), None, None).unwrap();
        copy.committed(1, &nudge(1.0)).unwrap();
        let hash = copy.document().hash();
        assert_eq!(copy.check_loaded(1, hash), Ok(()));
        assert!(copy.check_loaded(0, hash).is_err());
        assert!(copy.check_loaded(1, hash ^ 1).unwrap_err().contains("hash"));
    }

    #[test]
    fn requests_get_increasing_ids() {
        let mut copy = WorkingCopy::new(sample::bounce(), None, None).unwrap();
        let ids: Vec<_> = (0..3)
            .map(|_| match copy.submit(Entry::Undo) {
                ToStage::Submit { request, .. } => request,
                _ => panic!(),
            })
            .collect();
        assert_eq!(ids, [1, 2, 3]);
        assert_eq!(copy.document().seq(), 0, "submitting doesn't change the copy");
    }

    #[test]
    fn a_cut_off_last_line_is_ignored_but_a_bad_line_is_an_error() {
        let (mut copy, dir) = with_recovery();
        copy.committed(1, &nudge(1.0)).unwrap();
        let path = dir.join(LOG_FILE);
        let good = log_text(&dir);

        fs::write(&path, format!("{good}Do(SetRest(comp:")).unwrap();
        assert_eq!(read_recovery(&dir).unwrap().log, vec![nudge(1.0)]);

        fs::write(&path, format!("{good}Nonsense\n")).unwrap();
        let err = read_recovery(&dir).unwrap_err();
        assert!(err.contains("log.ron:2"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finish_keeps_the_directory_only_if_it_holds_unsaved_edits() {
        let (copy, dir) = with_recovery();
        copy.finish();
        assert!(!dir.exists());

        let (mut copy, dir) = with_recovery();
        copy.committed(1, &nudge(1.0)).unwrap();
        copy.finish();
        assert!(dir.join(LOG_FILE).exists());
        fs::remove_dir_all(dir).unwrap();

        // Undone back to the saved state: nothing to keep.
        let (mut copy, dir) = with_recovery();
        copy.committed(1, &nudge(1.0)).unwrap();
        copy.committed(2, &Entry::Undo).unwrap();
        copy.finish();
        assert!(!dir.exists());

        let (mut copy, dir) = with_recovery();
        copy.committed(1, &nudge(1.0)).unwrap();
        copy.discard();
        assert!(!dir.exists(), "discard drops unsaved edits");
    }

    #[test]
    fn saving_writes_the_project_and_marks_the_log() {
        let (mut copy, dir) = with_recovery();
        let target = tempdir().join("saved.bs2d");
        assert_eq!(copy.display_name(), "Untitled");
        copy.committed(1, &nudge(1.0)).unwrap();
        assert!(copy.is_dirty());

        copy.save_to(&target).unwrap();
        assert!(!copy.is_dirty());
        assert_eq!(copy.path(), Some(target.as_path()));
        assert_eq!(copy.display_name(), "saved");
        assert_eq!(&backstage_core::load(&target).unwrap(), copy.document().project());
        let meta = read_recovery(&dir).unwrap().meta;
        assert_eq!(meta, RecoveryMeta { source: Some(target.clone()), saved_seq: 1 });

        // Undo is still possible after a save, and makes the copy dirty.
        copy.committed(2, &Entry::Undo).unwrap();
        assert!(copy.is_dirty());
        let recovered = read_recovery(&dir).unwrap();
        assert_eq!(&Document::replay(recovered.base, &recovered.log).unwrap(), copy.document());
        copy.committed(3, &Entry::Redo).unwrap();
        assert!(!copy.is_dirty(), "back at the saved state");

        // Saved with no later edits: nothing left to recover.
        copy.finish();
        assert!(!dir.exists());
        fs::remove_dir_all(target.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_new_recovery_directory_records_its_source() {
        let dir = tempdir();
        let source = PathBuf::from("/somewhere/x.bs2d");
        Recovery::create(dir.clone(), &sample::bounce(), Some(&source)).unwrap();
        assert_eq!(read_recovery(&dir).unwrap().meta, RecoveryMeta { source: Some(source), saved_seq: 0 });

        // Directories from before meta.ron read as never saved.
        fs::remove_file(dir.join(META_FILE)).unwrap();
        assert_eq!(read_recovery(&dir).unwrap().meta, RecoveryMeta::default());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn commits_wait_for_the_loaded_check_after_a_load() {
        let mut copy = WorkingCopy::new(sample::bounce(), None, None).unwrap();
        copy.load_message();
        // A commit for the stage's previous document, still in flight.
        assert!(!copy.committed(7, &nudge(1.0)).unwrap());
        assert_eq!(copy.document().seq(), 0);
        let hash = copy.document().hash();
        copy.check_loaded(0, hash).unwrap();
        assert!(copy.committed(1, &nudge(1.0)).unwrap());
        assert_eq!(copy.document().seq(), 1);
    }

    #[test]
    fn save_target_adds_the_extension_and_refuses_other_things() {
        let dir = tempdir();
        fs::create_dir_all(&dir).unwrap();
        let ext = |name: &str| dir.join(name);

        assert_eq!(save_target(&ext("new")), Ok(ext("new.bs2d")));
        assert_eq!(save_target(&ext("new.bs2d")), Ok(ext("new.bs2d")));

        backstage_core::save(&sample::bounce(), &ext("old.bs2d")).unwrap();
        assert_eq!(save_target(&ext("old.bs2d")), Ok(ext("old.bs2d")), "replacing a project");
        assert!(save_target(&ext("old")).unwrap_err().contains("already exists"), "never asked to replace");

        fs::create_dir(ext("empty.bs2d")).unwrap();
        assert_eq!(save_target(&ext("empty.bs2d")), Ok(ext("empty.bs2d")));

        fs::create_dir(ext("photos.bs2d")).unwrap();
        fs::write(ext("photos.bs2d/cat.jpg"), "").unwrap();
        assert!(save_target(&ext("photos.bs2d")).unwrap_err().contains("isn't a Backstage2D project"));

        fs::write(ext("file.bs2d"), "").unwrap();
        assert!(save_target(&ext("file.bs2d")).unwrap_err().contains("is a file"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restore_carries_on_where_the_crashed_editor_stopped() {
        let (mut copy, dir) = with_recovery();
        let target = tempdir().join("restored.bs2d");
        copy.committed(1, &nudge(1.0)).unwrap();
        copy.save_to(&target).unwrap();
        copy.committed(2, &nudge(2.0)).unwrap();
        let before = copy.document().clone();
        // The crash: no finish, and a line cut off mid-write.
        drop(copy);
        let mut log = fs::OpenOptions::new().append(true).open(dir.join(LOG_FILE)).unwrap();
        log.write_all(b"Do(SetRest(comp:").unwrap();
        drop(log);

        let mut copy = WorkingCopy::restore(&dir).unwrap();
        assert_eq!(copy.document(), &before, "history included");
        assert_eq!(copy.document().hash(), before.hash());
        assert_eq!(copy.path(), Some(target.as_path()));
        assert!(copy.is_dirty());
        assert_eq!(copy.recovery_dir(), Some(dir.as_path()));

        // New edits append after the recovered ones, cleanly.
        copy.committed(3, &Entry::Undo).unwrap();
        assert!(!copy.is_dirty(), "back at the saved state");
        let recovered = read_recovery(&dir).unwrap();
        assert_eq!(recovered.log, [nudge(1.0), nudge(2.0), Entry::Undo]);
        assert_eq!(&Document::replay(recovered.base, &recovered.log).unwrap(), copy.document());

        assert!(WorkingCopy::restore(&dir).is_err(), "still in use by `copy`");
        copy.finish();
        assert!(!dir.exists());
        fs::remove_dir_all(target.parent().unwrap()).unwrap();
    }

    #[test]
    fn default_dir_is_under_the_state_home() {
        let dir = Recovery::default_dir().unwrap();
        assert!(dir.to_string_lossy().contains("backstage2d/recovery/"), "{}", dir.display());
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(&format!("{}-", std::process::id())), "{name}");
    }
}
