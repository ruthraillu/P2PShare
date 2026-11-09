//! Platform-independent Opus codec and bounded, timestamp-driven stereo jitter mixer.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
pub const RATE: u32 = 48000;
pub const FRAMES: usize = 480;
pub const SAMPLES: usize = FRAMES * 2;
pub const PACKET_US: u64 = 10000;
/// Captured PCM is a sample clock, not a callback-arrival clock. Android may
/// release several 10-ms blocks in one burst; their timestamps must stay uniform.
#[derive(Default)]
pub struct CaptureClock { next: Option<u64> }
impl CaptureClock {
    pub fn reset(&mut self) { self.next=None; }
    pub fn packet(&mut self, received_us:u64)->u64 {
        let pts=self.next.unwrap_or_else(||received_us.saturating_sub(PACKET_US));
        self.next=Some(pts.saturating_add(PACKET_US));pts
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    pub enabled: bool,
    pub bitrate_kbps: u32,
    pub playback_buffer_ms: u32,
}
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bitrate_kbps: 192,
            playback_buffer_ms: 30,
        }
    }
}
impl AudioConfig {
    pub fn validate(self) -> Result<()> {
        ensure!(
            (64..=512).contains(&self.bitrate_kbps),
            "audio.bitrate_kbps must be 64..512"
        );
        ensure!(
            (10..=100).contains(&self.playback_buffer_ms),
            "audio.playback_buffer_ms must be 10..100"
        );
        Ok(())
    }
}
/// Windows whole-display audio filtering. Kept separate so codec/playback config stays Copy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AudioExclusions {
    pub enabled: bool,
    pub processes: Vec<String>,
}
impl Default for AudioExclusions {
    fn default()->Self{Self{enabled:false,processes:vec!["discord.exe".into(),"serein.exe".into()]}}
}
impl AudioExclusions {
    pub fn validate(&self)->Result<()> {
        ensure!(self.processes.len()<=32,"audio_exclusions.processes supports at most 32 executable names");
        for name in &self.processes {
            ensure!(!name.is_empty()&&name==name.trim()&&name.chars().count()<=260&&name.to_lowercase().ends_with(".exe")&&!name.chars().any(|c|c.is_control()||matches!(c,'/'|'\\'|':'|'*'|'?')),"audio_exclusions.processes must contain executable filenames, e.g. discord.exe");
        }
        Ok(())
    }
}
pub fn encoder(config: AudioConfig) -> Result<opus::Encoder> {
    config.validate()?;
    let mut encoder =
        opus::Encoder::new(RATE, opus::Channels::Stereo, opus::Application::LowDelay)?;
    encoder.set_bitrate(opus::Bitrate::Bits((config.bitrate_kbps * 1000) as i32))?;
    encoder.set_vbr(false)?;
    encoder.set_complexity(10)?;
    encoder.set_signal(opus::Signal::Music)?;
    Ok(encoder)
}
struct Voice {
    decoder: opus::Decoder,
    packets: BTreeMap<u64, (Arc<[u8]>, Instant)>,
    next: Option<u64>,
    start: Instant,
    last: Instant,
    missing: u32,
}
#[derive(Default, Clone, Copy, Debug)]
pub struct AudioStats {
    pub received: u64,
    pub played: u64,
    pub concealed: u64,
    pub late: u64,
    pub overflows: u64,
    pub resets: u64,
}
pub struct Mixer {
    voices: HashMap<String, Voice>,
    muted: HashSet<String>,
    volumes: HashMap<String, f32>,
    buffer: Duration,
    latency: crate::diagnostics::LatencyMeter,
    pub stats: AudioStats,
}
impl Mixer {
    pub fn new(buffer_ms: u32) -> Self {
        Self {
            voices: HashMap::new(),
            muted: HashSet::new(),
            volumes: HashMap::new(),
            buffer: Duration::from_millis(buffer_ms as u64),
            latency: crate::diagnostics::LatencyMeter::new("audio_receive_to_playback_queue"),
            stats: AudioStats::default(),
        }
    }
    pub fn set_muted(&mut self, peer: &str, muted: bool) {
        if muted {
            self.muted.insert(peer.into());
        } else {
            self.muted.remove(peer);
        }
    }
    pub fn set_volume(&mut self, peer: &str, percent: u8) {
        let percent = percent.min(100);
        if percent == 100 {
            self.volumes.remove(peer);
        } else {
            self.volumes.insert(peer.into(), percent as f32 / 100.0);
        }
    }
    pub fn retain(&mut self, peers: &HashSet<String>) {
        self.voices.retain(|p, _| peers.contains(p));
        self.muted.retain(|p| peers.contains(p));
        self.volumes.retain(|p, _| peers.contains(p));
    }
    pub fn clear(&mut self) {
        self.voices.clear();
    }
    pub fn push(
        &mut self,
        peer: &str,
        micros: u64,
        data: Arc<[u8]>,
        received: Instant,
    ) -> Result<()> {
        self.push_released(peer, micros, data, received, received)
    }
    /// Keep original receive latency while scheduling optionally delayed packet delivery.
    pub fn push_released(
        &mut self,
        peer: &str,
        micros: u64,
        data: Arc<[u8]>,
        received: Instant,
        released: Instant,
    ) -> Result<()> {
        ensure!(
            !data.is_empty() && data.len() <= 1275,
            "Invalid Opus packet size"
        );
        // Only the negotiated 10-ms stereo profile is accepted; bound remote decode work.
        ensure!(
            opus::packet::get_nb_samples(&data, RATE)? == FRAMES,
            "Unsupported Opus packet duration"
        );
        if self.voices.len() >= 6 && !self.voices.contains_key(peer) {
            return Ok(());
        }
        if !self.voices.contains_key(peer) {
            self.voices.insert(
                peer.into(),
                Voice {
                    decoder: opus::Decoder::new(RATE, opus::Channels::Stereo)?,
                    packets: BTreeMap::new(),
                    next: None,
                    start: released + self.buffer,
                    last: released,
                    missing: 0,
                },
            );
        }
        let voice = self.voices.get_mut(peer).unwrap();
        if released.saturating_duration_since(voice.last) > Duration::from_millis(200)
            || voice.next.is_some_and(|n| micros > n + 200000)
        {
            voice.decoder.reset_state()?;
            voice.packets.clear();
            voice.next = None;
            voice.start = released + self.buffer;
            voice.missing = 0;
            self.stats.resets += 1;
        }
        voice.last = released;
        self.stats.received += 1;
        if voice.next.is_some_and(|n| micros + 100 < n) {
            self.stats.late += 1;
            return Ok(());
        }
        if voice.packets.len() >= 12 {
            voice.packets.clear();
            voice.next = None;
            voice.start = released + self.buffer;
            voice.decoder.reset_state()?;
            self.stats.overflows += 1;
        }
        voice.packets.insert(micros, (data, received));
        Ok(())
    }
    /// Exactly 10 ms, including silence. Called by the device-driven playback worker.
    pub fn render(&mut self, now: Instant, out: &mut [f32; SAMPLES]) {
        out.fill(0.0);
        let mut pcm = [0.0; SAMPLES];
        for (peer, voice) in &mut self.voices {
            if now < voice.start || voice.last.elapsed() > Duration::from_millis(200) {
                continue;
            }
            let Some(first) = voice.packets.keys().next().copied().or(voice.next) else {
                continue;
            };
            let next = voice.next.unwrap_or(first);
            // RTP clock conversion can differ by one microsecond, so match with tolerance.
            let key = voice
                .packets
                .range(next.saturating_sub(100)..=next + 100)
                .next()
                .map(|(k, _)| *k);
            let data = key.and_then(|k| voice.packets.remove(&k));
            if let Some((_, received)) = &data {
                self.latency
                    .record(now.saturating_duration_since(*received).as_secs_f64() * 1000.);
            }
            let decoded = voice.decoder.decode_float(
                data.as_ref()
                    .map(|(packet, _)| packet.as_ref())
                    .unwrap_or(&[]),
                &mut pcm,
                false,
            );
            match decoded {
                Ok(FRAMES) => {
                    self.stats.played += 1;
                    if data.is_none() {
                        self.stats.concealed += 1;
                        voice.missing += 1;
                    } else {
                        voice.missing = 0;
                    }
                    if !self.muted.contains(peer) {
                        let gain = self.volumes.get(peer).copied().unwrap_or(1.0);
                        for (o, s) in out.iter_mut().zip(pcm) {
                            *o += s * gain;
                        }
                    }
                }
                _ => {
                    self.stats.concealed += 1;
                }
            }
            voice.next = Some(next + PACKET_US);
            voice.packets.retain(|k, _| *k + 100 >= next + PACKET_US);
            if voice.missing >= 10 {
                voice.next = None;
                voice.packets.clear();
                voice.start = now + self.buffer;
                voice.missing = 0;
                let _ = voice.decoder.reset_state();
            }
        }
        for sample in out {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn burst_capture_timestamps_play_without_missing_audio() -> Result<()> {
        let mut clock=CaptureClock::default();let mut encoder=encoder(AudioConfig::default())?;
        let now=Instant::now();let mut mixer=Mixer::new(30);let mut previous=None;
        for burst in 0..8 {
            let arrival=now+Duration::from_millis(burst*40);
            for _ in 0..4 {
                let pts=clock.packet(1_000_000+burst*40_000);
                if let Some(previous)=previous {assert_eq!(pts-previous,PACKET_US);}previous=Some(pts);
                let pcm=[0.05f32;SAMPLES];let packet=encoder.encode_vec_float(&pcm,1275)?;
                mixer.push("android",pts,packet.into(),arrival)?;
            }
            for i in 0..4 {let mut output=[0.0;SAMPLES];mixer.render(arrival+Duration::from_millis(30+i*10),&mut output);}
        }
        assert_eq!(mixer.stats.concealed,0);assert_eq!(mixer.stats.late,0);assert_eq!(mixer.stats.overflows,0);
        clock.reset();assert_eq!(clock.packet(5_000_000),4_990_000);Ok(())
    }
    #[test]
    fn independent_playback_levels_scale_pcm_and_preserve_mute() -> Result<()> {
        let mut encoder = encoder(AudioConfig::default())?;
        let mut pcm = [0.0; SAMPLES];
        for (i, s) in pcm.iter_mut().enumerate() {
            *s = (i as f32 * 0.0576).sin() * 0.1;
        }
        let packet: Arc<[u8]> = encoder.encode_vec_float(&pcm, 1275)?.into();
        let now = Instant::now();
        let render = |levels: &[(&str, u8, bool)]| -> Result<[f32; SAMPLES]> {
            let mut mixer = Mixer::new(0);
            for (peer, volume, muted) in levels {
                mixer.set_volume(peer, *volume);
                mixer.set_muted(peer, *muted);
                mixer.push(peer, 0, packet.clone(), now)?;
            }
            let mut out = [0.0; SAMPLES];
            mixer.render(now, &mut out);
            Ok(out)
        };
        let full = render(&[("a", 100, false)])?;
        assert!(full.iter().any(|s| s.abs() > 0.01));
        for (levels, scale) in [
            (vec![("a", 25, false)], 0.25),
            (vec![("a", 0, false)], 0.),
            (vec![("a", 25, true)], 0.),
            (vec![("a", 25, false), ("b", 50, false)], 0.75),
            (vec![("a", 25, true), ("b", 50, false)], 0.5),
            (vec![("a", 255, false)], 1.),
        ] {
            let out = render(&levels)?;
            for (sample, reference) in out.iter().zip(full) {
                assert!((*sample - reference * scale).abs() < 0.000001);
            }
        }
        let mut mixer = Mixer::new(0);
        mixer.set_volume("a", 37);
        mixer.set_muted("a", true);
        mixer.set_muted("a", false);
        assert_eq!(mixer.volumes["a"], 0.37);
        mixer.retain(&HashSet::new());
        assert!(mixer.volumes.is_empty());
        Ok(())
    }
    #[test]
    fn cbr_stereo_roundtrip_reordering_mute_and_loss() -> Result<()> {
        let mut encoder = encoder(AudioConfig::default())?;
        let mut mixer = Mixer::new(30);
        let start = Instant::now();
        let mut input = [0.0; SAMPLES];
        let mut packets = Vec::new();
        for n in 0..8 {
            for i in 0..FRAMES {
                input[i * 2] = ((n * FRAMES + i) as f32 * 0.0576).sin() * 0.3;
                input[i * 2 + 1] = 0.0;
            }
            let packet = encoder.encode_vec_float(&input, 1275)?;
            assert_eq!(packet.len(), 240); // 192 kbps * 10 ms / 8.
            packets.push(packet);
        }
        for n in [1, 0, 3, 2, 5, 4, 7] {
            mixer.push(
                "peer",
                1000000 + n as u64 * PACKET_US,
                packets[n].clone().into(),
                start,
            )?;
        }
        let mut out = [0.0; SAMPLES];
        let mut left = 0.0;
        let mut right = 0.0;
        for n in 0..6 {
            mixer.render(start + Duration::from_millis(30 + n * 10), &mut out);
            left += out.iter().step_by(2).map(|s| s * s).sum::<f32>();
            right += out.iter().skip(1).step_by(2).map(|s| s * s).sum::<f32>();
        }
        assert!(
            left > 1.0 && right < left * 0.02,
            "stereo channels must survive"
        );
        mixer.set_muted("peer", true);
        mixer.render(start + Duration::from_millis(90), &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(mixer.stats.concealed, 1);
        mixer.set_muted("peer", false);
        mixer.render(start + Duration::from_millis(100), &mut out);
        assert!(out.iter().any(|s| s.abs() > 0.01));
        Ok(())
    }
}
