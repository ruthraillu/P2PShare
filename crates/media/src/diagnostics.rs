//! Bounded, opt-in Android copy buffer and frame cadence metrics. No pixels or capabilities.
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
static ENABLED: AtomicBool = AtomicBool::new(false);
const LIMIT: usize = 256 * 1024;
static BUFFER: OnceLock<Mutex<String>> = OnceLock::new();
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
pub fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
    if value {
        let mut b = BUFFER.get_or_init(Default::default).lock().unwrap();
        b.clear();
        b.push_str("P2PShare diagnostics: times are milliseconds; RTT=-1 means unavailable; receiver latency excludes sender/network before first packet.\n");
    }
}
#[cfg(target_os = "android")]
static ARCHIVE: OnceLock<Mutex<Option<Box<dyn Write + Send>>>> = OnceLock::new();

#[cfg(target_os = "android")]
pub fn set_archive(writer: Option<Box<dyn Write + Send>>) {
    *ARCHIVE.get_or_init(Default::default).lock().unwrap() = writer;
}

pub fn append(text: &str) {
    if !enabled() {
        return;
    }
    #[cfg(target_os = "android")]
    if let Some(writer) = ARCHIVE.get_or_init(Default::default).lock().unwrap().as_mut() {
        let _ = writer.write_all(text.as_bytes());
    }
    let mut b = BUFFER.get_or_init(Default::default).lock().unwrap();
    b.push_str(text);
    if b.len() > LIMIT {
        let mut cut = b.len() - LIMIT;
        while !b.is_char_boundary(cut) {
            cut += 1;
        }
        if let Some(end) = b[cut..].find('\n') {
            cut += end + 1;
        }
        b.drain(..cut);
    }
}
pub fn export() -> String {
    BUFFER.get_or_init(Default::default).lock().unwrap().clone()
}
#[derive(Clone, Copy)]
pub struct BufferWriter;
impl Write for BufferWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        append(&String::from_utf8_lossy(bytes));
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
// No subscriber dependency is required: callers supply a closure returning BufferWriter.
pub struct FrameMeter {
    name: &'static str,
    since: Instant,
    last: Option<Instant>,
    count: u64,
    bytes: u64,
    stutters: u64,
    gaps: VecDeque<f64>,
    threshold_ms: f64,
    total: u64,
}
impl FrameMeter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            since: Instant::now(),
            last: None,
            count: 0,
            bytes: 0,
            stutters: 0,
            gaps: VecDeque::new(),
            threshold_ms: 100.,
            total: 0,
        }
    }
    pub fn set_fps(&mut self, fps: u32) {
        self.threshold_ms = 100f64.max(2500. / fps.max(1) as f64);
    }
    pub fn with_fps(mut self, fps: u32) -> Self {
        self.set_fps(fps);
        self
    }
    pub fn frame(&mut self, bytes: usize) {
        let now = Instant::now();
        if let Some(last) = self.last {
            let gap = now.duration_since(last).as_secs_f64() * 1000.;
            if gap > self.threshold_ms {
                self.stutters += 1;
            }
            if self.gaps.len() == 120 {
                self.gaps.pop_front();
            }
            self.gaps.push_back(gap);
        }
        self.last = Some(now);
        self.count += 1;
        self.total += 1;
        self.bytes += bytes as u64;
    }
    pub fn report(&mut self) {
        let seconds = self.since.elapsed().as_secs_f64();
        if seconds < 1. {
            return;
        }
        if enabled() {
            let mut gaps: Vec<_> = self.gaps.iter().copied().collect();
            gaps.sort_by(f64::total_cmp);
            let p95 = gaps
                .get(gaps.len().saturating_sub(1) * 95 / 100)
                .copied()
                .unwrap_or(0.);
            tracing::info!(
                stage = self.name,
                fps = self.count as f64 / seconds,
                bitrate_bps = self.bytes as f64 * 8. / seconds,
                total_frames = self.total,
                stutters = self.stutters,
                gap_p95_ms = p95,
                gap_max_ms = gaps.last().copied().unwrap_or(0.),
                stall_ms = self
                    .last
                    .map(|t| t.elapsed().as_secs_f64() * 1000.)
                    .unwrap_or(0.),
                "frame_cadence"
            );
        }
        self.count = 0;
        self.bytes = 0;
        self.gaps.clear();
        self.since = Instant::now();
    }
}

pub struct LatencyMeter {
    name: &'static str,
    since: Instant,
    values: Vec<f64>,
}
impl LatencyMeter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            since: Instant::now(),
            values: Vec::new(),
        }
    }
    pub fn record(&mut self, ms: f64) {
        if !enabled() {
            self.values.clear();
            self.since = Instant::now();
            return;
        }
        if self.values.len() < 240 {
            self.values.push(ms);
        }
        if self.since.elapsed().as_secs_f64() >= 1. {
            self.values.sort_by(f64::total_cmp);
            if !self.values.is_empty() {
                tracing::info!(
                    stage = self.name,
                    mean_ms = self.values.iter().sum::<f64>() / self.values.len() as f64,
                    p95_ms = self.values[(self.values.len() - 1) * 95 / 100],
                    max_ms = self.values.last().copied().unwrap_or(0.),
                    samples = self.values.len(),
                    "stage_latency"
                );
            }
            self.values.clear();
            self.since = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_utf8_copy_buffer() {
        set_enabled(true);
        append(&"é".repeat(LIMIT));
        let b = export();
        assert!(b.len() <= LIMIT);
        assert!(std::str::from_utf8(b.as_bytes()).is_ok());
        set_enabled(false);
        let before = export();
        append("off");
        assert_eq!(export(), before);
    }
}
