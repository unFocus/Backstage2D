# ADR 0004: Edit commands, the document log, and document authority

**Status:** Accepted (2026-09-30). Implemented in M2; §5 extended for
Save and Open in M3 (2026-09-30).

## Context
[ADR 0002](0002-stage-process-isolation.md) requires:
- every edit to be a command;
- the stage to be the single writer, deciding the order of edits;
- the tools process to keep a copy, so a stage crash loses at most the
  gesture in progress.

[ADR 0003](0003-document-model.md) says commands edit the `Project` by ID
and that undo inverts them. This ADR records how that was built.

## Decision

### 1. Commands are small, invertible primitives
`backstage_core::Command` has one variant per kind of change:
- `SetRest`
- `SetKey` / `RemoveKey`
- `InsertTrack` / `RemoveTrack`
- `AddNode` / `RemoveNode`
- `MoveNode`
- `RenameNode`, `RenameAnimation`
- `SetAnimationTiming`
- `SetSettings`, `SetEditorPrefs`
- `Batch`

Everything is addressed by ID.

- **`apply` returns the inverse,** built from the state it replaced. Undo is
  applying the inverse, and the inverse's inverse is the redo. No command
  has to know how to undo itself ahead of time.
- **A failed command changes nothing.** Lookups happen before any change.
  After the change, only the composition it touched is revalidated
  (`Project::validate_composition`, plus the recursion check). If that fails,
  the change is rolled back. Commands and `load` therefore enforce the same
  rules, and a valid project stays valid.
- **`Batch` is all or nothing.** One gesture is one batch, which is one undo
  step.
- **New objects get their IDs from the caller,** never from `apply`, so
  replaying the same commands always gives the same project.
- **Inverses restore exact order.** Removing a track's last key removes the
  track, and the inverse puts it back at the same index (`InsertTrack`).
  `RemoveNode` takes the subtree and every track that keyed it, and its
  inverse rebuilds them top-down. Order matters because documents are
  compared by hash.

### 2. The document changes only through log entries
`backstage_core::Document` holds three things:
- the project;
- the undo and redo stacks, which hold inverses;
- a sequence number.

It changes only through `Entry::{Do(Command), Undo, Redo}`.
- **Undo and redo are entries in the log,** not local operations. Every
  replica agrees on the history as well as the project, and a stage rebuilt
  by replaying the log can still undo.
- **A rejected entry changes nothing,** including the sequence number.
- **`Document::replay(base, log)`** rebuilds a document and names the entry
  that failed.
- **`Document::hash()`** is FNV-1a over the canonical RON of the project,
  the history, and the sequence number. It hashes the file format, not
  memory, so it's stable across runs and builds (std's `DefaultHasher` is
  not).
- **Each entry serializes to one line of RON,** in the same dialect as the
  project files, for the on-disk log.

### 3. The stage is the writer; the editor replicates
- **Stage:** `Submit { request, entry }` → it applies the entry, then replies
  `Committed { seq, request, entry }` or `Rejected { request, reason }`.
  Before `Load` it has no document, rejects everything, and draws nothing.
- **Editor** (`WorkingCopy`): it never changes its copy directly. It applies
  only `Committed` entries, strictly in sequence order. A gap, a duplicate,
  or an entry that won't apply means it is **out of sync**. The editor then
  restarts the stage, and the editor's copy is the reference.
- **On every connect** the editor sends `Load { base, log }`: the project as
  opened, plus every committed entry. The stage replays them and answers
  `Loaded { seq, hash }`.
  - The editor sends edits only after that hash matches its own
    (`stage_ready`).
  - A mismatch would repeat on every restart, so the editor shows a warning
    and doesn't restart.
- **Entries submitted but not committed** when a stage dies are dropped, not
  resubmitted. That's the "gesture in progress" ADR 0002 allows losing.

### 4. Document data travels as RON text inside postcard
Core types leave default fields out when serializing
(`skip_serializing_if`). Only a self-describing format can read that back;
postcard can't. So protocol messages stay postcard, but document data
inside them is RON text:
- `#[serde(with = "ron_text")]` for entries and logs;
- `Snapshot` for projects: the project's canonical files, as produced by
  `backstage_core::to_files`.

A protocol test pins the postcard limitation down. The message size limit
is 64 MiB, and the sender refuses to send anything larger.
`PROTOCOL_VERSION` is 2.

### 5. Autosave
The editor writes a recovery directory for each run:
`$XDG_STATE_HOME/backstage2d/recovery/<pid>-<unix time>/`. It holds:
- `base.bs2d/`: the project as opened;
- `log.ron`: one committed entry per line, flushed to disk (`sync_data`)
  before the commit counts as handled.

Reading it back (`read_recovery`) ignores a last line without its newline,
since a crash cut it off. A disk error turns autosave off with a banner;
editing continues from memory.

It also holds `meta.ron`: `source` (the project's folder, if it has one)
and `saved_seq` (how many log entries are saved there). It's written
atomically when the directory is created and after every Save.

**Saving marks the log; it doesn't clear it.** Truncating the log to a new
base would drop the undo history with it: an `Undo` after the save
couldn't be replayed. So the log and `base.bs2d/` stay as they are, the
editor's `WorkingCopy`, `Load`, and the protocol are unchanged, and
`meta.ron` records how far the save got.

- **Dirty** means the project differs from the one last opened or saved,
  so undoing back to the saved state is clean again.
- **A clean exit** removes the directory unless the copy is dirty. "Don't
  Save" removes it even then. A dirty directory is kept for the restore
  prompt.
- **File → Open keeps the stage:** the editor makes a new `WorkingCopy`
  (with its own recovery directory) and sends its `Load` to the running
  stage. A commit for the old document can still arrive before `Loaded`,
  since the socket is ordered, so a copy ignores commits between sending
  `Load` and checking the `Loaded` it answers. If the old copy's own `Load`
  is still unanswered, the next `Loaded` wouldn't be the new one's, so the
  editor starts a fresh stage instead.

**Restoring after an editor crash** (`recovery.rs`):
- **A live directory is locked.** The editor holds an exclusive lock on its
  `log.ron` (`File::try_lock`). The OS releases it when the process dies,
  even on SIGKILL, so an unlocked directory is an orphan. This works the
  same on every platform and doesn't depend on PIDs, which get reused.
- **At startup, `scan`** takes a shared lock on each directory in turn:
  - a directory another editor holds is skipped;
  - an unreadable one is reported on stderr and never deleted;
  - one with nothing unsaved is deleted. "Nothing unsaved" means replaying
    the whole log gives the project at `saved_seq`, which is `base` if it was
    never saved. This is the cleanup policy for old directories.
  - The rest are candidates.
- **Only the newest candidate is offered per launch:** Restore, Discard
  (after a confirmation), or Not Now. The others are offered on later
  launches.
- **Restore takes over the directory.** `Recovery::resume` locks it, cuts
  off a log line the crash left incomplete, and the editor carries on
  appending there. The document is the replayed log, so the undo history
  and the saved state come back, and the path comes from `meta.ron`.

## Consequences
- **Every future editing feature only produces entries:** timeline and
  property panels (M3), on-stage drags (M4), and Save. Undo, crash
  recovery, and autosave come with them for free.
- **Each panel edit costs one local round trip** before the editor shows
  it. On a Unix socket that is well under a frame.
- **The renderer's mesh cache is cleared after every load and commit.** It
  is keyed by shape address, which adding or removing nodes can move or
  reuse. If that shows up in profiles, it can be keyed by content instead.
- **Two tests guard all of this end to end:**
  - `a_restarted_stage_replays_the_editors_log` (supervisor integration);
  - the UI smoke test: edit, kill the stage, check the replay matches, save,
    then undo in the real editor.

## Deferred
- **Log growth:** compaction or checkpoints, which would need a `base_seq`
  in `Load`. There is no cap on the undo history yet.
- **fsync cost:** batching it if M4 drags commit faster than per-entry
  fsync allows.
- **More than one orphan:** offered one per launch, newest first. A list to
  choose from can come later if this turns out to matter.
- **Deleting a directory on Windows** while its lock is held needs checking
  when the editor is ported there.
- **The arrow-key nudge:** a debug edit, removed once on-stage editing
  exists (M4).
- **Runtime state:** the playhead survives a stage restart, since the
  editor owns it and resends it (`ToStage::Transport`, M3). Mixer state
  from scripts (M6) doesn't yet.
