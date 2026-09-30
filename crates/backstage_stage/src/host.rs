//! The stage's document: the single writer. The editor loads it with
//! `Load` and asks for changes with `Submit`; the stage applies each entry,
//! numbers it, and reports it with `Committed`, which the editor copies.
//! See `docs/adr/0002-stage-process-isolation.md`.

use backstage_core::{Document, Entry, Project};
use backstage_protocol::{Snapshot, ToTools};

#[derive(Default)]
pub struct DocumentHost {
    doc: Option<Document>,
}

impl DocumentHost {
    /// Replaces the document with `base` plus `log` replayed onto it, and
    /// returns the `Loaded` reply.
    pub fn load(&mut self, base: &Snapshot, log: &[Entry]) -> Result<ToTools, String> {
        let project = base.project().map_err(|e| format!("loading the project: {e}"))?;
        let doc = Document::replay(project, log).map_err(|e| format!("replaying the log: {e}"))?;
        let reply = ToTools::Loaded { seq: doc.seq(), hash: doc.hash() };
        self.doc = Some(doc);
        Ok(reply)
    }

    /// Applies an entry and returns `Committed`, or `Rejected` with the
    /// document unchanged.
    pub fn submit(&mut self, request: u64, entry: &Entry) -> ToTools {
        let Some(doc) = self.doc.as_mut() else {
            return ToTools::Rejected { request, reason: "no document loaded".into() };
        };
        match doc.apply(entry) {
            Ok(seq) => ToTools::Committed { seq, request: Some(request), entry: entry.clone() },
            Err(e) => ToTools::Rejected { request, reason: e.to_string() },
        }
    }

    pub fn project(&self) -> Option<&Project> {
        self.doc.as_ref().map(Document::project)
    }
}

#[cfg(test)]
mod tests {
    use super::DocumentHost;
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{Command, Document, Entry, Props};
    use backstage_protocol::{Snapshot, ToTools};

    fn sample_snapshot() -> Snapshot {
        Snapshot::of(&sample::bounce()).unwrap()
    }

    fn nudge(x: f32) -> Entry {
        Entry::Do(Command::SetRest { comp: STAGE, node: GROUND, rest: Props::at(x, 0.0) })
    }

    #[test]
    fn submit_before_load_is_rejected() {
        let mut host = DocumentHost::default();
        assert!(host.project().is_none());
        assert_eq!(
            host.submit(1, &nudge(1.0)),
            ToTools::Rejected { request: 1, reason: "no document loaded".into() }
        );
    }

    #[test]
    fn load_replays_the_log_and_reports_its_hash() {
        let log = [nudge(1.0), nudge(2.0), Entry::Undo];
        let mut host = DocumentHost::default();
        let expected = Document::replay(sample::bounce(), &log).unwrap();
        assert_eq!(
            host.load(&sample_snapshot(), &log),
            Ok(ToTools::Loaded { seq: 3, hash: expected.hash() })
        );
        assert_eq!(host.project(), Some(expected.project()));
    }

    #[test]
    fn bad_loads_are_errors() {
        let mut host = DocumentHost::default();
        let err = host.load(&Snapshot { files: vec![] }, &[]).unwrap_err();
        assert!(err.starts_with("loading the project"), "{err}");
        let err = host.load(&sample_snapshot(), &[Entry::Undo]).unwrap_err();
        assert_eq!(err, "replaying the log: log entry 0: nothing to undo");
        assert!(host.project().is_none(), "a failed load keeps the old document (none)");
    }

    #[test]
    fn commits_are_numbered_and_rejections_are_not() {
        let mut host = DocumentHost::default();
        host.load(&sample_snapshot(), &[]).unwrap();
        assert_eq!(
            host.submit(10, &nudge(1.0)),
            ToTools::Committed { seq: 1, request: Some(10), entry: nudge(1.0) }
        );
        let ToTools::Rejected { request: 11, reason } = host.submit(11, &Entry::Redo) else { panic!() };
        assert_eq!(reason, "nothing to redo");
        assert_eq!(
            host.submit(12, &Entry::Undo),
            ToTools::Committed { seq: 2, request: Some(12), entry: Entry::Undo }
        );
        assert_eq!(
            host.submit(13, &Entry::Redo),
            ToTools::Committed { seq: 3, request: Some(13), entry: Entry::Redo }
        );
    }

    #[test]
    fn a_new_load_replaces_the_document() {
        let mut host = DocumentHost::default();
        host.load(&sample_snapshot(), &[nudge(5.0)]).unwrap();
        assert_eq!(
            host.load(&sample_snapshot(), &[]),
            Ok(ToTools::Loaded { seq: 0, hash: Document::new(sample::bounce()).hash() })
        );
        assert_eq!(host.project(), Some(&sample::bounce()));
    }
}
