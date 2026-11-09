//! WASAPI window/process or display loopback and playback. No audio work depends on GUI/video pacing.
use crate::wasapi::*;
use anyhow::{Context, Result};
use p2pshare_media::audio::{AudioConfig, FRAMES, Mixer, PACKET_US, RATE, SAMPLES};
use p2pshare_session::{AudioPacket, Session};
use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::Win32::System::Com::*;
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
/// A missing/unresolved window PID must never become a desktop capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureTarget {
    System,
    Process(u32),
}
impl CaptureTarget {
    fn from_source(source: &p2pshare_media::CaptureSource) -> Option<Self> {
        match source.kind {
            p2pshare_media::SourceKind::Monitor => Some(Self::System),
            p2pshare_media::SourceKind::Window => {
                source.process_id.filter(|pid| *pid != 0).map(Self::Process)
            }
        }
    }
}
struct Device {
    client: IAudioClient,
    capture: Option<IAudioCaptureClient>,
    render: Option<IAudioRenderClient>,
    capacity: u32,
    _event: Option<AudioEvent>,
    excludes_own_playback: bool,
}
impl Device {
    fn new(capture: bool) -> Result<Self> {
        if capture {
            match Self::open(capture, Some(CaptureTarget::System)) {
                Ok(device) => return Ok(device),
                Err(error) => {
                    tracing::warn!(%error,"process_loopback_unavailable_using_default_device")
                }
            }
        }
        Self::open(capture, None)
    }
    fn for_capture(target: CaptureTarget) -> Result<Self> {
        match target {
            CaptureTarget::System => Self::new(true),
            // Do not fall back to system audio for a window share.
            CaptureTarget::Process(_) => Self::open(true, Some(target)),
        }
    }
    fn open(capture: bool, target: Option<CaptureTarget>) -> Result<Self> {
        let process = target.is_some();
        // Shared-mode audio engine converts the default device's native rate/layout.
        // Interfaces remain entirely on this worker's COM apartment.
        unsafe {
            let client: IAudioClient = if let Some(target) = target {
                match target {
                    CaptureTarget::Process(pid) => including_process(pid)?,
                    CaptureTarget::System => excluding_own_process()?,
                }
            } else {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                enumerator
                    .GetDefaultAudioEndpoint(eRender, eConsole)?
                    .Activate(CLSCTX_ALL, None)?
            };
            let format = WaveFormatEx {
                wFormatTag: 3,
                nChannels: 2,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * 8,
                nBlockAlign: 8,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
                | if capture {
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                } else {
                    0
                };
            let event = if process {
                Some(AudioEvent::new()?)
            } else {
                None
            };
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags | if process { 0x40000 } else { 0 },
                if process { 0 } else { 200000 },
                0,
                &format,
                None,
            )?;
            if let Some(event) = &event {
                client.SetEventHandle(event.0)?;
            }

            let capacity = client.GetBufferSize()?;
            let input = if capture {
                Some(client.GetService()?)
            } else {
                None
            };
            let output = if capture {
                None
            } else {
                Some(client.GetService()?)
            };
            client.Start()?;
            tracing::info!(
                capture,
                ?target,
                excludes_own_playback = process,
                capacity_frames = capacity,
                sample_rate = RATE,
                "wasapi_audio_started"
            );
            Ok(Self {
                client,
                capture: input,
                render: output,
                capacity,
                _event: event,
                excludes_own_playback: process,
            })
        }
    }
    fn capture(&self, pcm: &mut VecDeque<f32>) -> Result<()> {
        let capture = self.capture.as_ref().context("Missing loopback")?;
        unsafe {
            // Bound each poll even when a device delivers a burst after a stall.
            for _ in 0..16 {
                if capture.GetNextPacketSize()? == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                    tracing::warn!("audio_capture_discontinuity");
                }
                let samples = frames as usize * 2;
                if samples <= RATE as usize * 2 {
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                        pcm.extend(std::iter::repeat_n(0.0, samples));
                    } else {
                        pcm.extend(
                            std::slice::from_raw_parts(data.cast::<f32>(), samples)
                                .iter()
                                .copied(),
                        );
                    }
                }
                capture.ReleaseBuffer(frames)?;
            }
        }
        Ok(())
    }
    fn render(&self, mixer: &mut Mixer, underruns: &mut u64) -> Result<()> {
        unsafe {
            let padding = self.client.GetCurrentPadding()?;
            if padding == 0 {
                *underruns += 1;
            }
            // Keep at most 20 ms queued at the device. Never queue stale seconds of audio.
            let available = self
                .capacity
                .saturating_sub(padding)
                .min((FRAMES * 2) as u32);
            if available < FRAMES as u32 || padding > FRAMES as u32 {
                return Ok(());
            }
            let render = self.render.as_ref().context("Missing output")?;
            let mut pcm = [0.0; SAMPLES];
            mixer.render(Instant::now(), &mut pcm);
            let data = render.GetBuffer(FRAMES as u32)?;
            std::ptr::copy_nonoverlapping(pcm.as_ptr(), data.cast::<f32>(), SAMPLES);
            render.ReleaseBuffer(FRAMES as u32, 0)?;
        }
        Ok(())
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}
#[path="audio_exclusions.rs"]
mod exclusions;
enum CaptureInput{Plain(Device),Filtered(exclusions::FilteredCapture)}
impl CaptureInput{
 fn open(target:CaptureTarget,filter:&p2pshare_media::audio::AudioExclusions,micros:u64)->Result<Self>{
  if target==CaptureTarget::System&&filter.enabled{return Ok(Self::Filtered(exclusions::FilteredCapture::new(filter.clone(),micros)?))}
  Ok(Self::Plain(Device::for_capture(target)?))
 }
 fn capture(&mut self,out:&mut VecDeque<f32>,pts:&mut u64)->Result<()>{match self{Self::Plain(device)=>device.capture(out),Self::Filtered(filter)=>{if let Some(first)=filter.capture(out)?{*pts=first;}Ok(())}}}
 fn excludes_own_playback(&self)->bool{match self{Self::Plain(device)=>device.excludes_own_playback,Self::Filtered(_)=>true}}
}
pub struct AudioRuntime {
    stop: Arc<AtomicBool>,
    target: Arc<Mutex<Option<CaptureTarget>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl AudioRuntime {
    pub fn new(session: Arc<Session>, config: AudioConfig,filter:p2pshare_media::audio::AudioExclusions) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let target = Arc::new(Mutex::new(None));
        let requested = target.clone();
        let thread = std::thread::Builder::new()
            .name("p2pshare-audio".into())
            .spawn(move || {
                if let Err(error) = run(session, config, filter,stopped, requested) {
                    tracing::error!(%error,"audio_worker_failed");
                }
            })?;
        Ok(Self {
            stop,
            target,
            thread: Some(thread),
        })
    }
}
impl AudioRuntime {
    pub fn set_source(&self, source: Option<&p2pshare_media::CaptureSource>) {
        let target = source.and_then(CaptureTarget::from_source);
        if source.is_some_and(|s| s.kind == p2pshare_media::SourceKind::Window) && target.is_none()
        {
            tracing::error!("window_audio_process_unavailable_audio_disabled");
        }
        *self.target.lock().unwrap() = target;
    }
}
impl Drop for AudioRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn run(
    session: Arc<Session>,
    config: AudioConfig,
    filter:p2pshare_media::audio::AudioExclusions,
    stop: Arc<AtomicBool>,
    requested: Arc<Mutex<Option<CaptureTarget>>>,
) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let _apartment = Apartment;
    let mut encoder = p2pshare_media::audio::encoder(config)?;
    let mut mixer = Mixer::new(if session.playback_delay_ms() > 0 {
        0
    } else {
        config.playback_buffer_ms
    });
    let mut input: Option<CaptureInput> = None;
    let mut capture_target = None;
    let mut output: Option<Device> = None;
    let mut captured = VecDeque::new();
    let mut pts = 0;
    let mut capture_retry = Instant::now();
    let mut playback_retry = Instant::now();
    let mut report = Instant::now();
    let mut membership = Instant::now() - Duration::from_secs(1);
    let mut desired_capture = false;
    let mut desired_playback = false;
    let mut encode_latency = p2pshare_media::diagnostics::LatencyMeter::new("audio_encode");
    let mut encoded = 0u64;
    let mut capture_drops = 0u64;
    let mut underruns = 0u64;
    tracing::info!(
        ?config,
        codec = "Opus low-delay CBR 48kHz stereo",
        "audio_configuration"
    );
    while !stop.load(Ordering::Acquire) {
        let target = *requested.lock().unwrap();
        if target != capture_target {
            input = None;
            captured.clear();
            capture_retry = Instant::now();
            capture_target = target;
            tracing::info!(?target, "audio_capture_target_changed");
        }
        if membership.elapsed() >= Duration::from_millis(20) {
            membership = Instant::now();
            let state = session.snapshot();
            desired_capture = config.enabled
                && session.needs_live_video()
                && state.in_room
                && state
                    .members
                    .iter()
                    .any(|m| m.id == state.local_id && m.sharing);
            let peers: HashSet<_> = state
                .members
                .iter()
                .filter(|m| m.id != state.local_id && m.sharing && session.is_watching(&m.id))
                .map(|m| m.id.clone())
                .collect();
            desired_playback = !peers.is_empty() && state.in_room;
            mixer.retain(&peers);
            for peer in peers {
                mixer.set_muted(&peer, session.audio_muted(&peer));
                mixer.set_volume(&peer, session.audio_volume(&peer));
            }
        }
        if !desired_capture || capture_target.is_none() {
            input = None;
            captured.clear();
        }
        if !desired_playback {
            output = None;
            mixer.clear();
        }
        if desired_capture
            && input.is_none()
            && Instant::now() >= capture_retry
            && let Some(target) = capture_target
        {
            match CaptureInput::open(target,&filter,session.media_micros()) {
                Ok(device) => {
                    input = Some(device);
                    captured.clear();
                    pts = session.media_micros();
                    encoder.reset_state()?;
                }
                Err(error) => {
                    tracing::error!(%error, ?target, "audio_capture_start_failed");
                    capture_retry = Instant::now() + Duration::from_secs(5);
                }
            }
        }
        if let Some(device) = &mut input {
            if let Err(error) = device.capture(&mut captured,&mut pts) {
                tracing::warn!(%error,"audio_capture_failed");
                input = None;
                capture_retry = Instant::now() + Duration::from_secs(1);
            }
            if captured.len() > SAMPLES * 8 {
                let dropped = captured.len() / SAMPLES - 2;
                captured.drain(..dropped * SAMPLES);
                pts += dropped as u64 * PACKET_US;
                capture_drops += dropped as u64;
                tracing::warn!(dropped, "audio_capture_queue_trimmed");
            }
            while captured.len() >= SAMPLES {
                let mut pcm = [0.0; SAMPLES];
                for s in &mut pcm {
                    *s = captured.pop_front().unwrap();
                }
                let started = Instant::now();
                let data = encoder.encode_vec_float(&pcm, 1275)?;
                encode_latency.record(started.elapsed().as_secs_f64() * 1000.);
                session.publish_audio(AudioPacket {
                    data: data.into(),
                    micros: pts,
                });
                pts += PACKET_US;
                encoded += 1;
            }
        }
        for packet in session.take_audio() {
            if let Err(error) = mixer.push_released(
                &packet.peer,
                packet.micros,
                packet.data,
                packet.received_at,
                Instant::now(),
            ) {
                tracing::warn!(%error,"audio_packet_rejected");
            }
        }
        if desired_playback && output.is_none() && Instant::now() >= playback_retry {
            match Device::new(false) {
                Ok(device) => output = Some(device),
                Err(error) => {
                    tracing::error!(%error,"audio_output_start_failed");
                    playback_retry = Instant::now() + Duration::from_secs(5);
                }
            }
        }
        if let Some(device) = &output
            && let Err(error) = device.render(&mut mixer, &mut underruns)
        {
            tracing::warn!(%error,"audio_output_failed");
            output = None;
            playback_retry = Instant::now() + Duration::from_secs(1);
        }
        if report.elapsed() >= Duration::from_secs(1) {
            if p2pshare_media::diagnostics::enabled() {
                tracing::info!(
                    excludes_own_playback = input
                        .as_ref()
                        .is_some_and(|device| device.excludes_own_playback()),
                    capture_active = input.is_some(),
                    playback_active = output.is_some(),
                    remote_viewers = session.snapshot().viewer_count,
                    encoded_packets = encoded,
                    capture_drops,
                    device_underruns = underruns,
                    received = mixer.stats.received,
                    played = mixer.stats.played,
                    concealed = mixer.stats.concealed,
                    late = mixer.stats.late,
                    overflows = mixer.stats.overflows,
                    resets = mixer.stats.resets,
                    "audio_metrics"
                );
            }
            report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(
            if input.is_some() || output.is_some() {
                2
            } else {
                10
            },
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_audio_target_never_falls_back_to_system() {
        let mut source = p2pshare_media::CaptureSource {
            id: 1,
            kind: p2pshare_media::SourceKind::Window,
            title: "Synthetic".into(),
            size: Default::default(),
            process_id: Some(1234),
        };
        assert_eq!(
            CaptureTarget::from_source(&source),
            Some(CaptureTarget::Process(1234))
        );
        source.process_id = None;
        assert_eq!(CaptureTarget::from_source(&source), None);
        source.process_id = Some(0);
        assert_eq!(CaptureTarget::from_source(&source), None);
        source.kind = p2pshare_media::SourceKind::Monitor;
        assert_eq!(
            CaptureTarget::from_source(&source),
            Some(CaptureTarget::System)
        );
    }
    #[test]
    #[ignore = "requires Windows process-loopback and default audio output"]
    fn selected_process_loopback_captures_only_owned_silence() -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        let _apartment = Apartment;
        let capture = Device::for_capture(CaptureTarget::Process(std::process::id()))?;
        let output = Device::new(false)?;
        let mut mixer = Mixer::new(30);
        let mut underruns = 0;
        let mut samples = VecDeque::new();
        let started = Instant::now();
        let mut received = 0;
        while started.elapsed() < Duration::from_millis(250) {
            output.render(&mut mixer, &mut underruns)?;
            capture.capture(&mut samples)?;
            assert!(samples.iter().all(|s| s.is_finite() && s.abs() < 0.000001));
            received += samples.len();
            samples.clear();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            received > 0,
            "Owned silent render did not reach process capture"
        );
        Ok(())
    }
    #[test]
    #[ignore = "requires the Windows default audio device"]
    fn wasapi_loopback_and_silent_output_initialize() -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        let _apartment = Apartment;
        let capture = Device::new(true)?;
        println!(
            "process loopback excludes own playback: {}",
            capture.excludes_own_playback
        );
        let output = Device::new(false)?;
        let mut mixer = Mixer::new(30);
        let mut underruns = 0;
        let mut samples = VecDeque::new();
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(250) {
            output.render(&mut mixer, &mut underruns)?;
            capture.capture(&mut samples)?;
            assert!(samples.iter().all(|s| s.is_finite()));
            samples.clear();
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}

struct AudioEvent(*mut std::ffi::c_void);
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateEventW(
        attributes: *const std::ffi::c_void,
        manual: i32,
        initial: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
}
impl AudioEvent {
    fn new() -> Result<Self> {
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        anyhow::ensure!(
            !handle.is_null(),
            "Could not create the audio capture event"
        );
        Ok(Self(handle))
    }
}
impl Drop for AudioEvent {
    fn drop(&mut self) {
        unsafe {
            let _ =
                windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(self.0));
        }
    }
}
