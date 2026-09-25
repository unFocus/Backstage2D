//! Crash and hang policy for the stage process, kept free of GTK so it can be
//! unit tested. Time is always passed in.

use std::time::{Duration, Instant};

/// No heartbeat or frame for this long means the stage is hung.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(3);
/// Give up auto-restarting after this many crashes in quick succession.
pub const MAX_QUICK_CRASHES: u32 = 3;
/// A crash this soon after a start counts as "quick".
pub const QUICK_CRASH_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterExit {
    Restart,
    GiveUp,
}

#[derive(Debug)]
pub struct StageHealth {
    connected: bool,
    started_at: Instant,
    last_seen: Instant,
    quick_crashes: u32,
}

impl StageHealth {
    pub fn new(now: Instant) -> Self {
        Self { connected: false, started_at: now, last_seen: now, quick_crashes: 0 }
    }

    /// A stage process was started. A manual (user-requested) start forgives
    /// earlier crashes; an automatic restart does not.
    pub fn started(&mut self, now: Instant, manual: bool) {
        self.connected = false;
        self.started_at = now;
        self.last_seen = now;
        if manual {
            self.quick_crashes = 0;
        }
    }

    pub fn connected(&mut self, now: Instant) {
        self.connected = true;
        self.last_seen = now;
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Any message from the current stage proves it is alive.
    pub fn saw_event(&mut self, now: Instant) {
        self.last_seen = now;
    }

    pub fn exited(&mut self, now: Instant) -> AfterExit {
        self.connected = false;
        if now.duration_since(self.started_at) < QUICK_CRASH_WINDOW {
            self.quick_crashes += 1;
        } else {
            self.quick_crashes = 0;
        }
        if self.quick_crashes >= MAX_QUICK_CRASHES { AfterExit::GiveUp } else { AfterExit::Restart }
    }

    /// True when a connected stage has gone quiet for too long.
    pub fn is_stalled(&self, now: Instant) -> bool {
        self.connected && now.duration_since(self.last_seen) > STALL_TIMEOUT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: Duration = Duration::from_secs(1);

    #[test]
    fn gives_up_after_repeated_quick_crashes() {
        let t0 = Instant::now();
        let mut h = StageHealth::new(t0);
        h.started(t0, true);
        assert_eq!(h.exited(t0 + SEC), AfterExit::Restart);
        h.started(t0 + SEC, false);
        assert_eq!(h.exited(t0 + 2 * SEC), AfterExit::Restart);
        h.started(t0 + 2 * SEC, false);
        assert_eq!(h.exited(t0 + 3 * SEC), AfterExit::GiveUp);
    }

    #[test]
    fn manual_start_forgives_crashes() {
        let t0 = Instant::now();
        let mut h = StageHealth::new(t0);
        for i in 0..2 {
            h.started(t0 + i * SEC, false);
            assert_eq!(h.exited(t0 + i * SEC), AfterExit::Restart);
        }
        h.started(t0 + 3 * SEC, true);
        assert_eq!(h.exited(t0 + 3 * SEC), AfterExit::Restart);
    }

    #[test]
    fn crash_after_long_run_resets_the_count() {
        let t0 = Instant::now();
        let mut h = StageHealth::new(t0);
        h.started(t0, false);
        assert_eq!(h.exited(t0), AfterExit::Restart);
        h.started(t0, false);
        assert_eq!(h.exited(t0), AfterExit::Restart);
        h.started(t0, false);
        assert_eq!(h.exited(t0 + QUICK_CRASH_WINDOW + SEC), AfterExit::Restart);
    }

    #[test]
    fn stall_only_counts_while_connected() {
        let t0 = Instant::now();
        let mut h = StageHealth::new(t0);
        h.started(t0, true);
        let late = t0 + STALL_TIMEOUT + SEC;
        assert!(!h.is_stalled(late), "not connected yet: the connect timeout handles this");
        h.connected(t0);
        assert!(h.is_stalled(late));
        h.saw_event(late);
        assert!(!h.is_stalled(late + SEC));
        h.exited(late);
        assert!(!h.is_stalled(late + 10 * SEC));
    }
}
