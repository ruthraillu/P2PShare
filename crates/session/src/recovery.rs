//! Bounded retry scheduling; only the lexicographically smaller peer sends offers.
use std::time::{Duration, Instant};
pub(super) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(25);
pub(super) struct Recovery {
    pub token: Option<String>,
    pub failures: u32,
    pub started: Instant,
    pub retry_at: Option<Instant>,
    pub ready: bool,
    pub negotiated: bool,
    pub local_candidates: std::collections::HashSet<String>,
    pub remote_candidates: std::collections::HashSet<String>,
    pub rx_packets: u64,
    pub tx_packets: u64,
}
impl Recovery {
    pub fn new(now: Instant) -> Self {
        Self {
            token: None,
            failures: 0,
            started: now,
            retry_at: None,
            ready: false,
            negotiated: false,
            local_candidates: Default::default(),
            remote_candidates: Default::default(),
            rx_packets: 0,
            tx_packets: 0,
        }
    }
    pub fn failed(&mut self, now: Instant) -> Duration {
        self.ready = false;
        let delay = Duration::from_secs((2u64 << self.failures.min(4)).min(30));
        if self.retry_at.is_none() {
            self.retry_at = Some(now + delay);
            self.failures = self.failures.saturating_add(1);
        }
        self.retry_at.unwrap().saturating_duration_since(now)
    }
    pub fn expired(&self, now: Instant) -> bool {
        !self.ready
            && self.retry_at.is_none()
            && now.duration_since(self.started) >= HANDSHAKE_TIMEOUT
    }
    pub fn due(&self, now: Instant) -> bool {
        self.retry_at.is_some_and(|at| now >= at)
    }
    pub fn attempting(&mut self, now: Instant, token: Option<String>) {
        self.started = now;
        self.token = token;
        self.retry_at = None;
        self.ready = false;
        self.negotiated = false;
        self.remote_candidates.clear();
        self.rx_packets = 0;
        self.tx_packets = 0;
    }
    pub fn connected(&mut self) {
        self.ready = true;
        self.retry_at = None;
        self.failures = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retries_back_off_without_per_tick_rescheduling_and_success_resets() {
        let now = Instant::now();
        let mut r = Recovery::new(now);
        assert!(!r.expired(now + Duration::from_secs(24)));
        assert!(r.expired(now + HANDSHAKE_TIMEOUT));
        for seconds in [2, 4, 8, 16, 30, 30] {
            let delay = r.failed(now);
            assert_eq!(delay, Duration::from_secs(seconds));
            assert!(!r.due(now));
            assert!(r.due(now + delay));
            assert_eq!(r.failed(now), delay);
            r.attempting(now, None);
        }
        r.connected();
        assert!(!r.expired(now + Duration::from_secs(100)));
        assert_eq!(r.failed(now), Duration::from_secs(2));
    }
}
