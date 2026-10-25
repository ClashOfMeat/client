//! When the daemon talks to the web app. Heartbeats keep the app "online" for friends' visits;
//! observations go up at most every 30 s while an agent works, plus once more shortly after it
//! stops; a backlog drains in paced batches; the server's hint, Retry-After and failures slow
//! everything down; and every timer carries ±20% jitter so clients never line up.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::sync::ApiError;

const BUSY_HEARTBEAT: Duration = Duration::from_secs(30);
/// Even when idle, a heartbeat a minute keeps this app "online" for friends' visits.
const IDLE_HEARTBEAT: Duration = Duration::from_secs(60);
/// While an agent works, observations go up at most this often.
const UPLOAD_EVERY: Duration = Duration::from_secs(30);
/// Once the agent stops, what is left goes up after nothing new arrived for this long.
const FLUSH_AFTER: Duration = Duration::from_secs(10);
/// A backlog drains one batch per interval instead of back to back.
const DRAIN_EVERY: Duration = Duration::from_secs(2);
/// A live nudge waits until this long after the last sync.
const LIVE_AFTER_SYNC: Duration = Duration::from_secs(10);
/// Each upload makes the server announce a revision on our own live channel. One that arrives
/// this soon after an upload is that echo; following it would upload again, in a loop.
const OWN_ECHO: Duration = Duration::from_secs(15);
const FIRST_BACKOFF: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(600);

/// What the sync thread should do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Wait,
    /// A heartbeat, then one batch of observations and the finished tasks.
    Sync,
    /// One more batch of a backlog, without a heartbeat.
    Drain,
}

/// The result of a sync or drain, as far as timing goes.
pub struct Outcome<'a> {
    pub due: Due,
    pub uploaded: usize,
    /// The batch was full, so more is queued.
    pub more: bool,
    pub error: Option<&'a ApiError>,
    /// The server's `nextSyncInSeconds`, already clamped.
    pub next_sync: Option<Duration>,
    /// An agent worked in the last few minutes.
    pub busy: bool,
}

pub struct Cadence {
    jitter: fn(Duration) -> Duration,
    last_attempt: Instant,
    last_sync: Instant,
    last_upload: Option<Instant>,
    next_heartbeat: Instant,
    next_upload: Instant,
    drain_at: Option<Instant>,
    live_at: Option<Instant>,
    hold_until: Option<Instant>,
    failures: u32,
    /// The server asked for a slower pace, so the quick flush waits for it too.
    hinted: bool,
}

impl Cadence {
    pub fn new(now: Instant) -> Self {
        Self::with_jitter(now, jitter)
    }

    /// `jitter` spreads every interval; tests pass the identity.
    pub fn with_jitter(now: Instant, jitter: fn(Duration) -> Duration) -> Self {
        Self {
            jitter,
            last_attempt: now,
            last_sync: now,
            last_upload: None,
            next_heartbeat: now + jitter(BUSY_HEARTBEAT),
            next_upload: now + jitter(UPLOAD_EVERY),
            drain_at: None,
            live_at: None,
            hold_until: None,
            failures: 0,
            hinted: false,
        }
    }

    /// The live channel announced a revision. False when it is the echo of our own upload.
    pub fn on_live(&mut self, now: Instant) -> bool {
        if self
            .last_upload
            .is_some_and(|at| now.duration_since(at) < OWN_ECHO)
        {
            return false;
        }
        self.nudge(now);
        true
    }

    /// Syncs soon, but never sooner than 10 s after the last sync.
    pub fn nudge(&mut self, now: Instant) {
        self.live_at
            .get_or_insert((self.last_sync + LIVE_AFTER_SYNC).max(now));
    }

    /// `busy`: an agent is working right now. `quiet`: how long the queue has not changed.
    pub fn due(&self, now: Instant, busy: bool, pending: bool, quiet: Duration) -> Due {
        if self.hold_until.is_some_and(|until| now < until) {
            return Due::Wait;
        }
        if self.live_at.is_some_and(|at| now >= at) || now >= self.next_heartbeat {
            return Due::Sync;
        }
        if !pending {
            return Due::Wait;
        }
        if self.drain_at.is_some_and(|at| now >= at) {
            return Due::Drain;
        }
        let flush = !busy
            && !self.hinted
            && quiet >= FLUSH_AFTER
            && now.duration_since(self.last_attempt) >= FLUSH_AFTER;
        if flush || now >= self.next_upload {
            Due::Sync
        } else {
            Due::Wait
        }
    }

    pub fn after(&mut self, now: Instant, outcome: Outcome) {
        let jitter = self.jitter;
        self.last_attempt = now;
        if outcome.due == Due::Sync {
            self.last_sync = now;
            self.live_at = None;
        }
        if outcome.uploaded > 0 {
            self.last_upload = Some(now);
        }
        self.hinted = outcome.next_sync.is_some();
        let heartbeat = if outcome.busy {
            BUSY_HEARTBEAT
        } else {
            IDLE_HEARTBEAT
        };
        if outcome.due == Due::Sync || outcome.next_sync.is_some() {
            self.next_heartbeat = now + jitter(outcome.next_sync.unwrap_or(heartbeat));
        }
        self.next_upload = now + jitter(outcome.next_sync.unwrap_or(UPLOAD_EVERY));
        self.drain_at = (outcome.error.is_none() && outcome.more)
            .then(|| now + jitter(outcome.next_sync.unwrap_or(DRAIN_EVERY)));
        if outcome.error.is_some_and(backs_off) {
            // 10 s, 20 s, 40 s ... up to 10 minutes; a success starts over.
            self.failures = self.failures.saturating_add(1);
            let wait = FIRST_BACKOFF
                .saturating_mul(1 << (self.failures - 1).min(16))
                .min(MAX_BACKOFF);
            let until = now + jitter(wait);
            self.hold_until = Some(until);
            // Then try again with a whole sync.
            self.next_heartbeat = until;
        } else {
            self.failures = 0;
            self.hold_until = None;
        }
    }
}

/// Server trouble and lost connections wait longer each time. A 429 or 503 with Retry-After
/// also sets `retry_after_until` (see `sync::fail`); a 401 or 426 stops syncing by itself.
/// A 400 or 413 reaches here only when no observation of the batch was accepted.
fn backs_off(error: &ApiError) -> bool {
    match error {
        ApiError::Transport(_) | ApiError::Invalid(_) => true,
        ApiError::Status { status, .. } => matches!(status, 400 | 408 | 413 | 500..=599),
    }
}

/// The server's pace for the next sync, `nextSyncInSeconds`, kept within 10 s to 10 minutes.
pub fn next_sync_hint(response: &Value) -> Option<Duration> {
    let seconds = response.get("nextSyncInSeconds")?.as_f64()?;
    seconds
        .is_finite()
        .then(|| Duration::from_secs_f64(seconds.clamp(10.0, 600.0)))
}

/// A random duration within ±20% of `base`.
pub fn jitter(base: Duration) -> Duration {
    base.mul_f64(0.8 + 0.4 * random_unit())
}

/// `base` plus up to a fifth more (1 to 30 s), so clients told to come back at the same moment,
/// such as a 429 pointing at the next minute, don't all return at once.
pub fn spread(base: Duration) -> Duration {
    base + Duration::from_secs_f64(random_unit() * (base.as_secs_f64() * 0.2).clamp(1.0, 30.0))
}

/// A random number in [0, 1) from the standard library's per-instance random keys.
fn random_unit() -> f64 {
    (RandomState::new().build_hasher().finish() >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn same(base: Duration) -> Duration {
        base
    }

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn done(due: Due, uploaded: usize) -> Outcome<'static> {
        Outcome {
            due,
            uploaded,
            more: false,
            error: None,
            next_sync: None,
            busy: true,
        }
    }

    #[test]
    fn the_echo_of_our_own_upload_is_ignored() {
        let start = Instant::now();
        let mut cadence = Cadence::with_jitter(start, same);
        cadence.after(start + S(30), done(Due::Sync, 5));
        assert!(!cadence.on_live(start + S(31)));
        assert!(!cadence.on_live(start + S(44)));
        assert_eq!(cadence.due(start + S(45), true, false, S(0)), Due::Wait);
        // A revision well after the upload is real, and waits 10 s after the last sync.
        assert!(cadence.on_live(start + S(46)));
        assert_eq!(cadence.due(start + S(46), true, false, S(0)), Due::Sync);
        // A sync that uploaded nothing makes no echo.
        let mut cadence = Cadence::with_jitter(start, same);
        cadence.after(start + S(30), done(Due::Sync, 0));
        assert!(cadence.on_live(start + S(32)));
        assert_eq!(cadence.due(start + S(39), true, false, S(0)), Due::Wait);
        assert_eq!(cadence.due(start + S(40), true, false, S(0)), Due::Sync);
    }

    #[test]
    fn busy_uploads_wait_30_seconds_and_flush_10_seconds_after_the_agent_stops() {
        let start = Instant::now();
        let mut cadence = Cadence::with_jitter(start, same);
        cadence.after(start, done(Due::Sync, 3));
        // Working: new observations wait for the 30 s mark, however quiet.
        assert_eq!(cadence.due(start + S(20), true, true, S(15)), Due::Wait);
        assert_eq!(cadence.due(start + S(30), true, true, S(1)), Due::Sync);
        // Stopped: the rest goes once the queue has been still for 10 s.
        assert_eq!(cadence.due(start + S(12), false, true, S(9)), Due::Wait);
        assert_eq!(cadence.due(start + S(12), false, true, S(10)), Due::Sync);
        // Nothing queued: only the heartbeat, every 60 s once idle.
        cadence.after(
            start + S(12),
            Outcome {
                busy: false,
                ..done(Due::Sync, 1)
            },
        );
        assert_eq!(cadence.due(start + S(71), false, false, S(59)), Due::Wait);
        assert_eq!(cadence.due(start + S(72), false, false, S(60)), Due::Sync);
    }

    #[test]
    fn a_backlog_drains_one_batch_every_two_seconds() {
        let start = Instant::now();
        let mut cadence = Cadence::with_jitter(start, same);
        cadence.after(
            start,
            Outcome {
                more: true,
                ..done(Due::Sync, 100)
            },
        );
        assert_eq!(cadence.due(start + S(1), true, true, S(0)), Due::Wait);
        assert_eq!(cadence.due(start + S(2), true, true, S(0)), Due::Drain);
        cadence.after(start + S(2), done(Due::Drain, 40));
        assert_eq!(cadence.due(start + S(4), true, true, S(0)), Due::Wait);
    }

    #[test]
    fn failures_back_off_exponentially_and_a_success_resets() {
        let start = Instant::now();
        let mut cadence = Cadence::with_jitter(start, same);
        let error = ApiError::Transport("offline".into());
        let mut at = start;
        for wait in [10, 20, 40, 80, 160, 320, 600, 600] {
            cadence.after(
                at,
                Outcome {
                    error: Some(&error),
                    ..done(Due::Sync, 0)
                },
            );
            assert_eq!(
                cadence.due(at + S(wait) - S(1), true, true, S(99)),
                Due::Wait
            );
            at += S(wait);
            assert_eq!(cadence.due(at, true, true, S(99)), Due::Sync);
        }
        cadence.after(at, done(Due::Sync, 1));
        cadence.after(
            at,
            Outcome {
                error: Some(&error),
                ..done(Due::Sync, 0)
            },
        );
        assert_eq!(cadence.due(at + S(10), true, true, S(99)), Due::Sync);
        // Client errors other than a stuck batch don't back off.
        let gone = ApiError::Status {
            status: 404,
            message: String::new(),
            detail: None,
            reason: None,
            retry_after_s: None,
            setup_url: None,
        };
        cadence.after(
            at,
            Outcome {
                error: Some(&gone),
                ..done(Due::Sync, 0)
            },
        );
        assert_eq!(cadence.due(at + S(30), true, true, S(0)), Due::Sync);
    }

    #[test]
    fn the_server_hint_sets_the_pace_within_bounds() {
        assert_eq!(
            next_sync_hint(&json!({"nextSyncInSeconds": 120})),
            Some(S(120))
        );
        assert_eq!(
            next_sync_hint(&json!({"nextSyncInSeconds": 2})),
            Some(S(10))
        );
        assert_eq!(
            next_sync_hint(&json!({"nextSyncInSeconds": 86400})),
            Some(S(600))
        );
        assert_eq!(next_sync_hint(&json!({"nextSyncInSeconds": "soon"})), None);
        assert_eq!(next_sync_hint(&json!({})), None);
        let start = Instant::now();
        let mut cadence = Cadence::with_jitter(start, same);
        cadence.after(
            start,
            Outcome {
                next_sync: Some(S(120)),
                ..done(Due::Sync, 1)
            },
        );
        // Neither the heartbeat nor a quick flush goes before the server's pace.
        assert_eq!(cadence.due(start + S(119), false, true, S(100)), Due::Wait);
        assert_eq!(cadence.due(start + S(120), false, true, S(100)), Due::Sync);
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        for _ in 0..200 {
            let value = jitter(S(100));
            assert!(value >= S(80) && value <= S(120), "{value:?}");
            let later = spread(S(60));
            assert!(later >= S(60) && later <= S(72), "{later:?}");
        }
    }
}
