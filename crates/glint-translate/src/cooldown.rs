//! Per-engine cool-down after rate limiting.
//!
//! A 429 means "ask less often", so asking again on the next capture only extends the
//! block. After a rate limit an engine is skipped until its cool-down ends: the server's
//! `Retry-After` when it sends one, otherwise 15 s, 45 s and then 2 minutes for consecutive
//! limits. One success resets the count.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Cool-downs without a `Retry-After`, by consecutive rate limit.
const STEPS: [Duration; 3] = [
    Duration::from_secs(15),
    Duration::from_secs(45),
    Duration::from_secs(120),
];
/// Upper bound for a server-requested wait.
const MAX: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Default)]
pub(crate) struct Cooldowns {
    map: Mutex<HashMap<String, Entry>>,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    until: Instant,
    strikes: u32,
}

impl Cooldowns {
    /// Time left in `engine_id`'s cool-down, if one is running.
    pub fn remaining(&self, engine_id: &str, now: Instant) -> Option<Duration> {
        let map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.get(engine_id)
            .and_then(|e| e.until.checked_duration_since(now))
            .filter(|d| !d.is_zero())
    }

    /// Start (or extend) a cool-down after a rate limit; returns its length.
    pub fn limited(
        &self,
        engine_id: &str,
        retry_after: Option<Duration>,
        now: Instant,
    ) -> Duration {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        let strikes = map.get(engine_id).map_or(0, |e| e.strikes) + 1;
        let wait = match retry_after {
            Some(d) if !d.is_zero() => d.min(MAX),
            _ => STEPS[(strikes as usize - 1).min(STEPS.len() - 1)],
        };
        map.insert(
            engine_id.to_string(),
            Entry {
                until: now + wait,
                strikes,
            },
        );
        wait
    }

    pub fn succeeded(&self, engine_id: &str) {
        self.map
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(engine_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backs_off_in_steps_and_resets_on_success() {
        let c = Cooldowns::default();
        let t0 = Instant::now();
        assert_eq!(c.remaining("g", t0), None);
        assert_eq!(c.limited("g", None, t0), Duration::from_secs(15));
        assert_eq!(c.remaining("g", t0), Some(Duration::from_secs(15)));
        assert_eq!(
            c.remaining("g", t0 + Duration::from_secs(16)),
            None,
            "expired"
        );
        assert_eq!(c.limited("g", None, t0), Duration::from_secs(45));
        assert_eq!(c.limited("g", None, t0), Duration::from_secs(120));
        assert_eq!(
            c.limited("g", None, t0),
            Duration::from_secs(120),
            "capped step"
        );
        c.succeeded("g");
        assert_eq!(c.remaining("g", t0), None);
        assert_eq!(
            c.limited("g", None, t0),
            Duration::from_secs(15),
            "count reset"
        );
    }

    #[test]
    fn server_retry_after_wins_but_is_capped() {
        let c = Cooldowns::default();
        let t0 = Instant::now();
        assert_eq!(
            c.limited("g", Some(Duration::from_secs(30)), t0),
            Duration::from_secs(30)
        );
        assert_eq!(c.limited("g", Some(Duration::from_secs(9_999)), t0), MAX);
        assert_eq!(c.remaining("other", t0), None, "engines are independent");
    }
}
