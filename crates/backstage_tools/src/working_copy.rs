//! The editor's copy of the document.
//!
//! The stage is the only writer: the editor submits entries and applies
//! only what the stage reports as committed, in sequence order. The copy
//! keeps every committed entry, sends them all with `Load` when a stage
//! (re)starts, and appends each one to a recovery log on disk. See
//! `docs/adr/0002-stage-process-isolation.md`.

use backstage_core::{Document, Entry, Project};
use backstage_protocol::{Snapshot, ToStage};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const BASE_DIR: &str = "base.bs2d";
const LOG_FILE: &str = "log.ron";

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

pub struct WorkingCopy {
    /// The project as opened, sent with every `Load`.
    base: Snapshot,
    /// Always `Document::replay(base, log)`.
    doc: Document,
    log: Vec<Entry>,
    next_request: u64,
    recovery: Option<Recovery>,
}

impl WorkingCopy {
    pub fn new(project: Project, recovery: Option<Recovery>) -> Result<Self, backstage_core::SaveError> {
        Ok(Self {
            base: Snapshot::of(&project)?,
            doc: Document::new(project),
            log: Vec::new(),
            next_request: 1,
            recovery,
        })
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// The directory the log is autosaved to, if autosave is on.
    pub fn recovery_dir(&self) -> Option<&Path> {
        self.recovery.as_ref().map(|r| r.dir.as_path())
    }

    /// What to send a stage that just connected: the base and every
    /// committed entry, so it rebuilds this exact document.
    pub fn load_message(&self) -> ToStage {
        ToStage::Load { base: self.base.clone(), log: self.log.clone() }
    }

    /// A request to apply `entry`. The copy changes only when the stage
    /// commits it.
    pub fn submit(&mut self, entry: Entry) -> ToStage {
        let request = self.next_request;
        self.next_request += 1;
        ToStage::Submit { request, entry }
    }

    /// Applies an entry the stage committed as `seq`.
    pub fn committed(&mut self, seq: u64, entry: &Entry) -> Result<(), CommitError> {
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
        Ok(())
    }

    /// Checks a restarted stage's `Loaded` against the copy.
    pub fn check_loaded(&self, seq: u64, hash: u64) -> Result<(), String> {
        let (want_seq, want_hash) = (self.doc.seq(), self.doc.hash());
        if (seq, hash) == (want_seq, want_hash) {
            Ok(())
        } else {
            Err(format!(
                "stage has entry {seq} (hash {hash:016x}), editor has {want_seq} (hash {want_hash:016x})"
            ))
        }
    }

    /// Clean exit. The recovery directory is removed only if it holds no
    /// edits: until there is Save (M3), it is the only copy of that work.
    pub fn finish(self) {
        if let Some(recovery) = self.recovery
            && self.log.is_empty()
        {
            drop(recovery.log);
            let _ = fs::remove_dir_all(&recovery.dir);
        }
    }
}

/// The on-disk copy: `base.bs2d/` (the project as opened) plus `log.ron`,
/// one committed entry per line.
pub struct Recovery {
    dir: PathBuf,
    log: File,
}

impl Recovery {
    /// A fresh directory for this editor run:
    /// `$XDG_STATE_HOME/backstage2d/recovery/<pid>-<unix seconds>/`.
    pub fn default_dir() -> io::Result<PathBuf> {
        let state = match std::env::var_os("XDG_STATE_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => {
                PathBuf::from(std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?)
                    .join(".local/state")
            }
        };
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Ok(state.join("backstage2d/recovery").join(format!("{}-{secs}", std::process::id())))
    }

    /// Saves `project` as the base in `dir` and starts an empty log.
    pub fn create(dir: PathBuf, project: &Project) -> io::Result<Self> {
        backstage_core::save(project, &dir.join(BASE_DIR)).map_err(io::Error::other)?;
        let log = OpenOptions::new().create_new(true).append(true).open(dir.join(LOG_FILE))?;
        Ok(Self { dir, log })
    }

    fn append(&mut self, entry: &Entry) -> io::Result<()> {
        let mut line = entry.to_line();
        line.push('\n');
        self.log.write_all(line.as_bytes())?;
        self.log.sync_data()
    }
}

/// Reads a recovery directory back: the base project and the committed
/// entries. A last line without its newline was cut off by a crash and is
/// ignored.
pub fn read_recovery(dir: &Path) -> Result<(Project, Vec<Entry>), String> {
    let project = backstage_core::load(&dir.join(BASE_DIR)).map_err(|e| e.to_string())?;
    let path = dir.join(LOG_FILE);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let complete = match text.rfind('\n') {
        Some(end) => &text[..end],
        None => "",
    };
    let entries = complete
        .lines()
        .enumerate()
        .map(|(i, line)| Entry::from_line(line).map_err(|e| format!("{}:{}: {e}", path.display(), i + 1)))
        .collect::<Result<_, _>>()?;
    Ok((project, entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{Command, Props};
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
        let recovery = Recovery::create(dir.clone(), &sample::bounce()).unwrap();
        (WorkingCopy::new(sample::bounce(), Some(recovery)).unwrap(), dir)
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
        let (project, entries) = read_recovery(&dir).unwrap();
        assert_eq!(&Document::replay(project, &entries).unwrap(), copy.document());
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
        let mut copy = WorkingCopy::new(sample::bounce(), None).unwrap();
        copy.committed(1, &nudge(1.0)).unwrap();
        copy.committed(2, &Entry::Undo).unwrap();
        let ToStage::Load { base, log } = copy.load_message() else { panic!() };
        assert_eq!(base.project().unwrap(), sample::bounce());
        assert_eq!(log, vec![nudge(1.0), Entry::Undo]);
    }

    #[test]
    fn check_loaded_compares_seq_and_hash() {
        let mut copy = WorkingCopy::new(sample::bounce(), None).unwrap();
        copy.committed(1, &nudge(1.0)).unwrap();
        let hash = copy.document().hash();
        assert_eq!(copy.check_loaded(1, hash), Ok(()));
        assert!(copy.check_loaded(0, hash).is_err());
        assert!(copy.check_loaded(1, hash ^ 1).unwrap_err().contains("hash"));
    }

    #[test]
    fn requests_get_increasing_ids() {
        let mut copy = WorkingCopy::new(sample::bounce(), None).unwrap();
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
        assert_eq!(read_recovery(&dir).unwrap().1, vec![nudge(1.0)]);

        fs::write(&path, format!("{good}Nonsense\n")).unwrap();
        let err = read_recovery(&dir).unwrap_err();
        assert!(err.contains("log.ron:2"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finish_keeps_the_directory_only_if_it_holds_edits() {
        let (copy, dir) = with_recovery();
        copy.finish();
        assert!(!dir.exists());

        let (mut copy, dir) = with_recovery();
        copy.committed(1, &nudge(1.0)).unwrap();
        copy.finish();
        assert!(dir.join(LOG_FILE).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn default_dir_is_under_the_state_home() {
        let dir = Recovery::default_dir().unwrap();
        assert!(dir.to_string_lossy().contains("backstage2d/recovery/"), "{}", dir.display());
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(&format!("{}-", std::process::id())), "{name}");
    }
}
