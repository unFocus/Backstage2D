//! Recovery directories: the editor's autosave, and finding what a crashed
//! editor left behind.
//!
//! Each editor run writes `$XDG_STATE_HOME/backstage2d/recovery/<name>/`:
//! `base.bs2d/` (the project as opened), `log.ron` (one committed entry per
//! line), and `meta.ron` (where the project is saved, and how much of the
//! log that covers). The running editor holds a lock on `log.ron`; the OS
//! drops it when the process dies, however it dies, so an unlocked
//! directory is an orphan. See
//! `docs/adr/0004-commands-and-document-authority.md`.

use backstage_core::{Document, Entry, Project};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const BASE_DIR: &str = "base.bs2d";
pub(crate) const LOG_FILE: &str = "log.ron";
pub(crate) const META_FILE: &str = "meta.ron";

/// `meta.ron`: where the project lives, and how much of the log is already
/// saved there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecoveryMeta {
    /// The project's directory; `None` if it was never saved.
    pub source: Option<PathBuf>,
    /// The number of log entries whose result is saved at `source`.
    pub saved_seq: u64,
}

/// A recovery directory this editor writes to. Holds the lock on its log
/// until dropped.
pub struct Recovery {
    dir: PathBuf,
    log: File,
}

impl Recovery {
    /// `$XDG_STATE_HOME/backstage2d/recovery/`, which holds one directory
    /// per editor run.
    pub fn root() -> io::Result<PathBuf> {
        let state = match std::env::var_os("XDG_STATE_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => {
                PathBuf::from(std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?)
                    .join(".local/state")
            }
        };
        Ok(state.join("backstage2d/recovery"))
    }

    /// A fresh directory for this run: `<root>/<pid>-<unix seconds>/`.
    pub fn default_dir() -> io::Result<PathBuf> {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let dir = Self::root()?.join(format!("{}-{secs}", std::process::id()));
        // Opening a second project in the same second needs its own name.
        let mut unique = dir.clone();
        for n in 2.. {
            if !unique.exists() {
                break;
            }
            unique = dir.with_file_name(format!("{}-{secs}-{n}", std::process::id()));
        }
        Ok(unique)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Saves `project` as the base in `dir` and starts an empty, locked log.
    /// `source` is where the project was opened from, if anywhere.
    pub fn create(dir: PathBuf, project: &Project, source: Option<&Path>) -> io::Result<Self> {
        backstage_core::save(project, &dir.join(BASE_DIR)).map_err(io::Error::other)?;
        let log = OpenOptions::new().create_new(true).append(true).open(dir.join(LOG_FILE))?;
        lock(&log)?;
        let recovery = Self { dir, log };
        recovery.write_meta(&RecoveryMeta { source: source.map(Path::to_owned), saved_seq: 0 })?;
        Ok(recovery)
    }

    /// Takes over an orphaned directory: locks its log, cuts off a last
    /// line the crash left incomplete, and reads it back. New entries are
    /// appended after the recovered ones.
    pub fn resume(dir: &Path) -> Result<(Self, Recovered), String> {
        let path = dir.join(LOG_FILE);
        let err = |e: io::Error| format!("{}: {e}", path.display());
        let mut log = OpenOptions::new().read(true).append(true).open(&path).map_err(err)?;
        lock(&log).map_err(err)?;
        let mut text = String::new();
        log.read_to_string(&mut text).map_err(err)?;
        let complete = complete_lines(&text);
        log.set_len(complete.len() as u64).map_err(err)?;
        let recovered = parse(dir, complete)?;
        Ok((Self { dir: dir.to_owned(), log }, recovered))
    }

    pub(crate) fn append(&mut self, entry: &Entry) -> io::Result<()> {
        let mut line = entry.to_line();
        line.push('\n');
        self.log.write_all(line.as_bytes())?;
        self.log.sync_data()
    }

    /// Replaces `meta.ron` atomically.
    pub(crate) fn write_meta(&self, meta: &RecoveryMeta) -> io::Result<()> {
        let mut text = backstage_core::io::to_ron_line(meta).map_err(io::Error::other)?;
        text.push('\n');
        let tmp = self.dir.join(format!("{META_FILE}.tmp"));
        let mut file = File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, self.dir.join(META_FILE))
    }

    /// Deletes the directory, unsaved edits and all.
    pub fn remove(self) {
        drop(self.log);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Takes the exclusive lock on a log without waiting.
fn lock(log: &File) -> io::Result<()> {
    match log.try_lock() {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => {
            Err(io::Error::new(io::ErrorKind::WouldBlock, "in use by another Backstage2D editor"))
        }
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// The text up to and including its last newline. Anything after it was
/// cut off by a crash.
fn complete_lines(text: &str) -> &str {
    &text[..text.rfind('\n').map_or(0, |i| i + 1)]
}

/// A recovery directory read back.
#[derive(Debug)]
pub struct Recovered {
    pub base: Project,
    pub log: Vec<Entry>,
    pub meta: RecoveryMeta,
}

impl Recovered {
    /// The document the log rebuilds (history included), and the project as
    /// it was last saved (or opened).
    pub fn replay(&self) -> Result<(Document, Project), String> {
        let saved_seq = usize::try_from(self.meta.saved_seq).unwrap_or(usize::MAX);
        let Some(saved_part) = self.log.get(..saved_seq) else {
            return Err(format!("saved_seq {saved_seq} is past the log's {} entries", self.log.len()));
        };
        let saved = Document::replay(self.base.clone(), saved_part).map_err(|e| e.to_string())?;
        let doc = Document::replay(self.base.clone(), &self.log).map_err(|e| e.to_string())?;
        Ok((doc, saved.project().clone()))
    }
}

/// Reads a recovery directory back without taking it over. A last log line
/// without its newline was cut off by a crash and is ignored. A directory
/// from before `meta.ron` existed reads as never saved.
pub fn read_recovery(dir: &Path) -> Result<Recovered, String> {
    let path = dir.join(LOG_FILE);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(dir, complete_lines(&text))
}

fn parse(dir: &Path, log_text: &str) -> Result<Recovered, String> {
    let base = backstage_core::load(&dir.join(BASE_DIR)).map_err(|e| e.to_string())?;
    let log_path = dir.join(LOG_FILE);
    let log = log_text
        .lines()
        .enumerate()
        .map(|(i, line)| Entry::from_line(line).map_err(|e| format!("{}:{}: {e}", log_path.display(), i + 1)))
        .collect::<Result<_, _>>()?;
    let meta_path = dir.join(META_FILE);
    let meta = match fs::read_to_string(&meta_path) {
        Ok(text) => backstage_core::io::from_ron_line(text.trim_end())
            .map_err(|e| format!("{}: {e}", meta_path.display()))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => RecoveryMeta::default(),
        Err(e) => return Err(format!("{}: {e}", meta_path.display())),
    };
    Ok(Recovered { base, log, meta })
}

/// A project directory's name without its extension, or "Untitled".
pub fn project_name(path: Option<&Path>) -> String {
    path.and_then(Path::file_stem).map_or_else(|| "Untitled".to_owned(), |s| s.to_string_lossy().into_owned())
}

/// An orphaned recovery directory with unsaved edits.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub dir: PathBuf,
    /// The project's name, or "Untitled".
    pub name: String,
    /// Log entries after the last save.
    pub unsaved: u64,
    /// When the log was last written.
    pub modified: SystemTime,
}

/// Finds orphaned directories under `root` that hold unsaved edits, newest
/// first. Along the way it deletes orphans with nothing unsaved, skips
/// directories another editor holds, and leaves unreadable ones alone
/// (they are reported on stderr, never deleted).
pub fn scan(root: &Path) -> Vec<Candidate> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| dir.is_dir())
        .filter_map(|dir| match inspect(&dir) {
            Ok(candidate) => candidate,
            Err(Skip::InUse) => None,
            Err(Skip::Unreadable(e)) => {
                eprintln!("recovery: leaving {} alone: {e}", dir.display());
                None
            }
        })
        .collect();
    found.sort_by_key(|c| std::cmp::Reverse(c.modified));
    found
}

enum Skip {
    InUse,
    Unreadable(String),
}

/// A candidate, or `None` if the directory had nothing unsaved and was
/// deleted.
fn inspect(dir: &Path) -> Result<Option<Candidate>, Skip> {
    let path = dir.join(LOG_FILE);
    let log = File::open(&path).map_err(|e| Skip::Unreadable(format!("{}: {e}", path.display())))?;
    match log.try_lock_shared() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(Skip::InUse),
        Err(TryLockError::Error(e)) => return Err(Skip::Unreadable(e.to_string())),
    }
    let recovered = read_recovery(dir).map_err(Skip::Unreadable)?;
    let (doc, saved) = recovered.replay().map_err(Skip::Unreadable)?;
    if doc.project() == &saved {
        // Still holding the lock, so no other editor is taking it over.
        let _ = fs::remove_dir_all(dir);
        return Ok(None);
    }
    let modified = log.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
    Ok(Some(Candidate {
        dir: dir.to_owned(),
        name: project_name(recovered.meta.source.as_deref()),
        unsaved: (recovered.log.len() as u64).saturating_sub(recovered.meta.saved_seq),
        modified,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{Command, Props};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    fn tempdir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "backstage-tools-recovery-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn nudge(x: f32) -> Entry {
        Entry::Do(Command::SetRest { comp: STAGE, node: GROUND, rest: Props::at(x, 0.0) })
    }

    /// A recovery directory holding `log`, still locked.
    fn write(dir: PathBuf, source: Option<&Path>, log: &[Entry], saved_seq: u64) -> Recovery {
        let mut recovery = Recovery::create(dir, &sample::bounce(), source).unwrap();
        for entry in log {
            recovery.append(entry).unwrap();
        }
        recovery.write_meta(&RecoveryMeta { source: source.map(Path::to_owned), saved_seq }).unwrap();
        recovery
    }

    fn set_age(dir: &Path, secs_ago: u64) {
        let file = File::options().append(true).open(dir.join(LOG_FILE)).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(secs_ago)).unwrap();
    }

    #[test]
    fn scan_offers_orphans_with_unsaved_edits_newest_first() {
        let root = tempdir();
        let source = PathBuf::from("/projects/bounce.bs2d");
        let live = write(root.join("live"), None, &[nudge(1.0)], 0);
        drop(write(root.join("older"), Some(&source), &[nudge(1.0), nudge(2.0), nudge(3.0)], 1));
        set_age(&root.join("older"), 60);
        drop(write(root.join("newer"), None, &[nudge(1.0)], 0));
        drop(write(root.join("undone"), None, &[nudge(1.0), Entry::Undo], 0));
        drop(write(root.join("saved"), Some(&source), &[nudge(1.0)], 1));
        drop(write(root.join("empty"), None, &[], 0));
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken").join(LOG_FILE), "Nonsense\n").unwrap();

        let found = scan(&root);
        let summary: Vec<_> = found
            .iter()
            .map(|c| (c.dir.file_name().unwrap().to_str().unwrap(), c.name.as_str(), c.unsaved))
            .collect();
        assert_eq!(summary, [("newer", "Untitled", 1), ("older", "bounce", 2)]);

        assert!(live.dir().exists(), "in use: skipped");
        assert!(root.join("broken").exists(), "unreadable: left alone");
        for gone in ["undone", "saved", "empty"] {
            assert!(!root.join(gone).exists(), "{gone}: nothing unsaved, so deleted");
        }
        drop(live);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_of_a_missing_root_finds_nothing() {
        assert!(scan(&tempdir()).is_empty());
    }

    #[test]
    fn resume_refuses_a_directory_in_use() {
        let dir = tempdir();
        let live = write(dir.clone(), None, &[nudge(1.0)], 0);
        let err = Recovery::resume(&dir).err().unwrap();
        assert!(err.contains("in use"), "{err}");
        drop(live);
        let (_resumed, recovered) = Recovery::resume(&dir).unwrap();
        assert_eq!(recovered.log, [nudge(1.0)]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replay_rejects_a_saved_seq_past_the_log() {
        let dir = tempdir();
        drop(write(dir.clone(), None, &[nudge(1.0)], 2));
        let err = read_recovery(&dir).unwrap().replay().unwrap_err();
        assert!(err.contains("past the log"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }
}
