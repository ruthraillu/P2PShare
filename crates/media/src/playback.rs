//! Shared audio/video sender timestamp mapping. Buffer compressed media, not pixels.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaybackConfig {
    pub buffer_ms: u32,
}
impl PlaybackConfig {
    pub fn validate(self) -> Result<()> {
        ensure!(self.buffer_ms <= 1000, "playback.buffer_ms must be 0..1000");
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub struct Timeline {
    origin_us: u64,
    origin: Instant,
    newest_us: u64,
    last_received: Instant,
}
impl Timeline {
    pub fn new(micros: u64, received: Instant) -> Self {
        Self {
            origin_us: micros,
            origin: received,
            newest_us: micros,
            last_received: received,
        }
    }
    /// Re-anchor after restart/long suspension; ordinary jitter never moves deadlines.
    pub fn observe(&mut self, micros: u64, received: Instant) -> bool {
        let reset = micros.saturating_add(2_000_000) < self.newest_us
            || received.saturating_duration_since(self.last_received) > Duration::from_secs(3);
        if reset {
            *self = Self::new(micros, received);
        }
        self.newest_us = self.newest_us.max(micros);
        self.last_received = received;
        reset
    }
    pub fn deadline(self, micros: u64, buffer_ms: u32) -> Instant {
        let delta = Duration::from_micros(micros.abs_diff(self.origin_us));
        let base = if micros >= self.origin_us {
            self.origin + delta
        } else {
            self.origin.checked_sub(delta).unwrap_or(self.origin)
        };
        base + Duration::from_millis(buffer_ms.into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_and_video_share_deadlines_despite_bursty_arrival() {
        let start = Instant::now();
        let mut timeline = Timeline::new(1_000_000, start);
        assert!(!timeline.observe(1_080_000, start + Duration::from_millis(160)));
        assert_eq!(
            timeline.deadline(1_080_000, 200),
            start + Duration::from_millis(280)
        );
        assert_eq!(
            timeline.deadline(1_090_000, 200),
            start + Duration::from_millis(290)
        );
        assert_eq!(
            timeline.deadline(990_000, 200),
            start + Duration::from_millis(190)
        );
        assert!(timeline.observe(0, start + Duration::from_secs(4)));
        assert_eq!(timeline.deadline(0, 1000), start + Duration::from_secs(5));
        assert!(PlaybackConfig { buffer_ms: 1001 }.validate().is_err());
    }
}
