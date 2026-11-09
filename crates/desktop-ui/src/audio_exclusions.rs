//! Optional Windows desktop capture from allowed process trees only.
//! Discovery is infrequent; all PCM mixing uses a bounded common QPC timeline.
use super::{CaptureTarget, Device};
use crate::wasapi::{AUDCLNT_BUFFERFLAGS_SILENT, active_audio_pids};
use anyhow::{Context, Result, ensure};
use p2pshare_media::audio::{AudioExclusions, FRAMES, RATE, SAMPLES};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{CloseHandle, FILETIME},
    System::{
        Diagnostics::ToolHelp::*,
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};
#[derive(Clone, Debug)]
struct Process {
    parent: u32,
    name: String,
    born: u64,
}
fn process_snapshot() -> Result<HashMap<u32, Process>> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
        let result = (|| -> Result<_> {
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            Process32FirstW(snapshot, &mut entry)?;
            let mut result = HashMap::new();
            loop {
                let pid = entry.th32ProcessID;
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let mut node = Process {
                    parent: entry.th32ParentProcessID,
                    name: String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase(),
                    born: 0,
                };
                if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    let (mut born, mut exit, mut kernel, mut user) = (
                        FILETIME::default(),
                        FILETIME::default(),
                        FILETIME::default(),
                        FILETIME::default(),
                    );
                    let valid =
                        GetProcessTimes(handle, &mut born, &mut exit, &mut kernel, &mut user)
                            .is_ok();
                    let _ = CloseHandle(handle);
                    if valid {
                        node.born =
                            ((born.dwHighDateTime as u64) << 32) | born.dwLowDateTime as u64;
                    }
                }
                result.insert(pid, node);
                match Process32NextW(snapshot, &mut entry) {
                    Ok(()) => {}
                    Err(error) => {
                        ensure!(
                            error.code().0 as u32 == 0x80070012,
                            "Process enumeration failed: {error}"
                        );
                        break;
                    }
                }
            }
            Ok(result)
        })();
        let _ = CloseHandle(snapshot);
        result
    }
}
fn ancestor(tree: &HashMap<u32, Process>, child: u32, parent: u32) -> bool {
    let mut current = child;
    let mut visited = BTreeSet::new();
    while visited.insert(current) && visited.len() <= 256 {
        if current == parent {
            return true;
        }
        let Some(node) = tree.get(&current) else {
            return false;
        };
        let Some(up) = tree.get(&node.parent) else {
            return false;
        };
        if up.born > 0 && node.born > 0 && up.born > node.born {
            return false;
        }
        current = node.parent;
    }
    false
}
fn selected(
    tree: &HashMap<u32, Process>,
    active: &BTreeSet<u32>,
    excluded: &[String],
    own: u32,
) -> BTreeMap<u32, u64> {
    let names: BTreeSet<_> = excluded.iter().map(|s| s.to_lowercase()).collect();
    let blocked: BTreeSet<_> = tree
        .iter()
        .filter_map(|(pid, p)| (names.contains(&p.name) || *pid == own).then_some(*pid))
        .collect();
    let candidates: BTreeSet<_> = active
        .iter()
        .copied()
        .filter(|pid| {
            tree.get(pid).is_some_and(|p| p.born != 0)
                && *pid != own
                && !blocked
                    .iter()
                    .any(|b| ancestor(tree, *pid, *b) || ancestor(tree, *b, *pid))
        })
        .collect();
    candidates
        .iter()
        .filter(|pid| {
            !candidates
                .iter()
                .any(|other| other != *pid && ancestor(tree, **pid, *other))
        })
        .map(|pid| (*pid, tree[pid].born))
        .collect()
}
fn qpc() -> Result<u64> {
    unsafe {
        let (mut count, mut freq) = (0, 0);
        QueryPerformanceFrequency(&mut freq)?;
        QueryPerformanceCounter(&mut count)?;
        ensure!(freq > 0 && count >= 0, "Invalid performance clock");
        Ok((count as u128 * 10_000_000 / freq as u128) as u64)
    }
}
#[derive(Default)]
struct Timeline {
    next: u64,
    blocks: BTreeMap<u64, Box<[f32; SAMPLES]>>,
    late: u64,
}
impl Timeline {
    fn add(&mut self, start: i128, pcm: &[f32], silent_frames: usize) {
        let frames = if silent_frames > 0 {
            silent_frames
        } else {
            pcm.len() / 2
        };
        let mut offset = (self.next as i128 - start).clamp(0, frames as i128) as usize;
        self.late += offset as u64;
        if silent_frames > 0 {
            return;
        }
        let end =
            (self.next as i128 + FRAMES as i128 * 12 - start).clamp(0, frames as i128) as usize;
        while offset < end {
            let frame = (start + offset as i128) as u64;
            let inside = frame as usize % FRAMES;
            let count = (FRAMES - inside).min(end - offset);
            let block = self
                .blocks
                .entry(frame / FRAMES as u64)
                .or_insert_with(|| Box::new([0.; SAMPLES]));
            for i in 0..count * 2 {
                let sample = pcm[offset * 2 + i];
                if sample.is_finite() {
                    block[inside * 2 + i] += sample;
                }
            }
            offset += count;
        }
    }
    fn emit(&mut self, now: u64, out: &mut VecDeque<f32>) -> Option<u64> {
        // Leave 30 ms for independent loopback clients to deliver the same sample times.
        let ready = now.saturating_sub(RATE as u64 * 30 / 1000);
        if ready < self.next + FRAMES as u64 {
            return None;
        }
        if ready > self.next + FRAMES as u64 * 8 {
            self.next = (ready / FRAMES as u64 - 2) * FRAMES as u64;
            self.blocks
                .retain(|key, _| *key >= self.next / FRAMES as u64);
            tracing::warn!("audio_filtered_queue_trimmed");
        }
        let first = self.next;
        while ready >= self.next + FRAMES as u64 {
            let block = self
                .blocks
                .remove(&(self.next / FRAMES as u64))
                .unwrap_or_else(|| Box::new([0.; SAMPLES]));
            out.extend(block.iter().map(|s| s.clamp(-1., 1.)));
            self.next += FRAMES as u64;
        }
        Some(first)
    }
}
struct Input {
    born: u64,
    device: Device,
}
// Discovery can involve slow COM calls. Keep it off the real-time capture/playback worker.
struct Discovery {
    rx: std::sync::mpsc::Receiver<Result<BTreeMap<u32, Process>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    wake: std::sync::Arc<(std::sync::Mutex<()>, std::sync::Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Discovery {
    fn new(names: Vec<String>) -> Result<Self> {
        use std::sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc::sync_channel,
        };
        let (tx, rx) = sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let wake = Arc::new((Mutex::new(()), Condvar::new()));
        let waiting = wake.clone();
        let thread = std::thread::Builder::new()
            .name("p2pshare-audio-discovery".into())
            .spawn(move || {
                use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
                if let Err(error) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() } {
                    let _ = tx.try_send(Err(error.into()));
                    return;
                }
                let _apartment = super::Apartment;
                while !stopped.load(Ordering::Acquire) {
                    let result = (|| -> Result<_> {
                        let active = active_audio_pids()?;
                        let tree = process_snapshot()?;
                        let roots = selected(&tree, &active, &names, std::process::id());
                        ensure!(roots.len() <= 32, "Too many allowed audio process trees");
                        Ok(roots.keys().map(|pid| (*pid, tree[pid].clone())).collect())
                    })();
                    if matches!(
                        tx.try_send(result),
                        Err(std::sync::mpsc::TrySendError::Disconnected(_))
                    ) {
                        break;
                    }
                    let guard = waiting.0.lock().unwrap();
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let _guard = waiting
                        .1
                        .wait_timeout(guard, Duration::from_millis(500))
                        .unwrap();
                }
            })?;
        Ok(Self {
            rx,
            stop,
            wake,
            thread: Some(thread),
        })
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        let _guard = self.wake.0.lock().unwrap();
        self.wake.1.notify_all();
        drop(_guard);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub(super) struct FilteredCapture {
    discovery: Discovery,
    inputs: BTreeMap<u32, Input>,
    planned: BTreeMap<u32, u64>,
    #[cfg(test)]
    input_energy: f64,
    retry: HashMap<u32, Instant>,
    epoch: u64,
    micros: u64,
    timeline: Timeline,
    report: Instant,
    received_frames: u64,
    invalid_timestamp_frames: u64,
}
impl FilteredCapture {
    pub fn new(config: AudioExclusions, micros: u64) -> Result<Self> {
        config.validate()?;
        let capture = Self {
            discovery: Discovery::new(config.processes)?,
            inputs: BTreeMap::new(),
            planned: BTreeMap::new(),
            #[cfg(test)]
            input_energy: 0.,
            retry: HashMap::new(),
            epoch: qpc()?,
            micros,
            timeline: Timeline::default(),
            report: Instant::now(),
            received_frames: 0,
            invalid_timestamp_frames: 0,
        };
        Ok(capture)
    }
    fn refresh(&mut self, targets: BTreeMap<u32, Process>) -> Result<()> {
        let planned = targets
            .iter()
            .map(|(pid, p)| (*pid, p.born))
            .collect::<BTreeMap<_, _>>();
        let changed = planned != self.planned;
        self.planned = planned;
        self.inputs
            .retain(|pid, input| targets.get(pid).is_some_and(|p| p.born == input.born));
        self.retry.retain(|pid, _| targets.contains_key(pid));
        if changed {
            self.timeline.blocks.clear();
        }
        for (pid, process) in targets {
            let born = process.born;
            if self.inputs.contains_key(&pid)
                || self.retry.get(&pid).is_some_and(|t| *t > Instant::now())
            {
                continue;
            }
            match Device::for_capture(CaptureTarget::Process(pid)) {
                Ok(device) => {
                    tracing::info!(pid,process=%process.name,"audio_filtered_process_started");
                    self.inputs.insert(pid, Input { born, device });
                    self.retry.remove(&pid);
                }
                Err(error) => {
                    tracing::warn!(pid,%error,"audio_filtered_process_unavailable");
                    self.retry
                        .insert(pid, Instant::now() + Duration::from_secs(5));
                }
            }
        }
        Ok(())
    }
    pub fn capture(&mut self, out: &mut VecDeque<f32>) -> Result<Option<u64>> {
        loop {
            match self.discovery.rx.try_recv() {
                Ok(plan) => self.refresh(plan?)?,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    anyhow::bail!("Audio discovery worker stopped")
                }
            }
        }
        let mut failed = Vec::new();
        for (pid, input) in &self.inputs {
            let result = (|| -> Result<()> {
                let client = input
                    .device
                    .capture
                    .as_ref()
                    .context("Missing process capture")?;
                unsafe {
                    for _ in 0..16 {
                        if client.GetNextPacketSize()? == 0 {
                            break;
                        }
                        let (mut data, mut frames, mut flags, mut position, mut stamp) =
                            (std::ptr::null_mut(), 0, 0, 0, 0);
                        client.GetBuffer(
                            &mut data,
                            &mut frames,
                            &mut flags,
                            Some(&mut position),
                            Some(&mut stamp),
                        )?;
                        // Invalid timestamp packets cannot be mixed safely. Release every acquired buffer.
                        self.received_frames += frames as u64;
                        if flags & 4 != 0 || stamp == 0 {
                            self.invalid_timestamp_frames += frames as u64;
                        }
                        if flags & 1 != 0 {
                            tracing::debug!(pid, "audio_filtered_discontinuity");
                        }
                        if frames <= RATE && flags & 4 == 0 && stamp > 0 {
                            let start = ((stamp as i128 - self.epoch as i128) * RATE as i128
                                + 5_000_000)
                                .div_euclid(10_000_000);
                            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                                self.timeline.add(start, &[], frames as usize);
                            } else {
                                let pcm = std::slice::from_raw_parts(
                                    data.cast::<f32>(),
                                    frames as usize * 2,
                                );
                                #[cfg(test)]
                                {
                                    self.input_energy += pcm
                                        .iter()
                                        .filter(|s| s.is_finite())
                                        .map(|s| s.abs() as f64)
                                        .sum::<f64>();
                                }
                                self.timeline.add(start, pcm, 0);
                            }
                        }
                        client.ReleaseBuffer(frames)?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                tracing::warn!(pid,%error,"audio_filtered_capture_failed");
                failed.push(*pid);
            }
        }
        for pid in failed {
            self.inputs.remove(&pid);
            self.retry
                .insert(pid, Instant::now() + Duration::from_secs(1));
        }
        let now = qpc()?.saturating_sub(self.epoch) as u128 * RATE as u128 / 10_000_000;
        let first = self
            .timeline
            .emit(now as u64, out)
            .map(|frame| self.micros + frame * 1_000_000 / RATE as u64);
        if self.report.elapsed() >= Duration::from_secs(5) {
            tracing::info!(
                sources = self.inputs.len(),
                late_frames = self.timeline.late,
                received_frames = self.received_frames,
                invalid_timestamp_frames = self.invalid_timestamp_frames,
                queued_blocks = self.timeline.blocks.len(),
                "audio_filter_metrics"
            );
            self.report = Instant::now();
        }
        Ok(first)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn node(parent: u32, name: &str, born: u64) -> Process {
        Process {
            parent,
            name: name.into(),
            born,
        }
    }
    #[test]
    fn process_roots_exclude_names_descendants_self_and_ancestors_and_deduplicate() {
        let tree = HashMap::from([
            (1, node(0, "explorer.exe", 1)),
            (2, node(1, "discord.exe", 2)),
            (3, node(2, "helper.exe", 3)),
            (4, node(1, "browser.exe", 2)),
            (5, node(4, "renderer.exe", 3)),
            (6, node(1, "p2pshare.exe", 4)),
            (7, node(1, "serein.exe", 5)),
        ]);
        assert_eq!(
            selected(
                &tree,
                &BTreeSet::from([1, 2, 3, 4, 5, 6, 7]),
                &["Discord.EXE".into(), "serein.exe".into()],
                6
            ),
            BTreeMap::from([(4, 2)])
        );
    }
    #[test]
    fn inaccessible_excluded_parent_still_blocks_its_child() {
        let tree = HashMap::from([
            (2, node(1, "discord.exe", 0)),
            (3, node(2, "helper.exe", 3)),
        ]);
        assert!(selected(&tree, &BTreeSet::from([3]), &["discord.exe".into()], 99).is_empty());
    }
    #[test]
    fn reused_parent_pid_does_not_change_old_process_ancestry() {
        let tree = HashMap::from([
            (1, node(0, "discord.exe", 9)),
            (2, node(1, "player.exe", 1)),
        ]);
        assert!(!ancestor(&tree, 2, 1));
    }
    #[test]
    fn simultaneous_packets_mix_by_sample_time_not_callback_order() {
        let mut timeline = Timeline::default();
        timeline.add(0, &vec![0.2; SAMPLES], 0);
        timeline.add(FRAMES as i128, &vec![0.6; SAMPLES], 0);
        timeline.add(0, &vec![0.3; SAMPLES], 0);
        let mut out = VecDeque::new();
        assert_eq!(timeline.emit((FRAMES * 4) as u64, &mut out), Some(0));
        assert_eq!(out.len(), SAMPLES);
        assert!(out.iter().all(|s| (*s - 0.5).abs() < 0.00001));
        assert_eq!(timeline.blocks.len(), 1);
    }
    #[test]
    fn late_and_future_packets_are_bounded_and_silence_does_not_overwrite() {
        let mut t = Timeline::default();
        t.add(0, &vec![0.5; SAMPLES], 0);
        t.add(0, &[], FRAMES);
        t.add(999999, &vec![1.; SAMPLES], 0);
        assert_eq!(t.blocks.len(), 1);
        let mut out = VecDeque::new();
        t.emit((FRAMES * 4) as u64, &mut out);
        t.add(0, &vec![1.; SAMPLES], 0);
        assert_eq!(t.blocks.len(), 0);
        assert!(out.iter().all(|s| *s == 0.5));
        assert_eq!(t.late, FRAMES as u64);
    }
    #[test]
    #[ignore = "reads local session metadata and captures allowed app audio for 3 seconds; no playback, files or network"]
    fn native_filter_probe() -> Result<()> {
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
            .ok()?;
        }
        let _apartment = super::super::Apartment;
        let mut filter = FilteredCapture::new(AudioExclusions::default(), 0)?;
        let start = Instant::now();
        let mut packets = 0;
        let mut output_energy = 0.;
        while start.elapsed() < Duration::from_secs(3) {
            let mut out = VecDeque::new();
            if filter.capture(&mut out)?.is_some() {
                packets += out.len() / SAMPLES;
                output_energy += out.iter().map(|s| s.abs() as f64).sum::<f64>();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        ensure!(packets >= 200, "Filtered sample clock did not advance");
        println!(
            "Filtered native probe: {packets} packets, {} allowed process trees, {} captured frames, {} invalid-timestamp frames",
            filter.inputs.len(),
            filter.received_frames,
            filter.invalid_timestamp_frames
        );
        println!(
            "Native mixing: input_energy={}, output_energy={output_energy}, late_frames={}",
            filter.input_energy, filter.timeline.late
        );
        if filter.input_energy > 0.0001 {
            ensure!(
                output_energy > 0.0001,
                "Non-silent native audio did not reach the filtered mix"
            );
        }
        if filter.received_frames > 0 {
            ensure!(
                filter.invalid_timestamp_frames < filter.received_frames,
                "Native capture did not provide usable audio timestamps"
            );
        }
        Ok(())
    }
}
