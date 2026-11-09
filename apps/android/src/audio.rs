//! Opus decoding/jitter mixing and AudioTrack playback independent of UI and video.
use super::{Bridge, FOREGROUND};
use jni::objects::JValue;
use p2pshare_media::audio::{AudioConfig, FRAMES, Mixer, SAMPLES};
use p2pshare_session::Session;
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub(super) struct AudioReceiver {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl AudioReceiver {
    pub(super) fn new(
        session: Arc<Session>,
        bridge: Arc<Bridge>,
        config: AudioConfig,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::Builder::new()
            .name("p2pshare-audio-playback".into())
            .spawn(move || {
                let _attachment = match bridge.vm.attach_current_thread() {
                    Ok(a) => a,
                    Err(error) => {
                        tracing::error!(%error,"audio_jni_attach_failed");
                        return;
                    }
                };
                let mut mixer = Mixer::new(if session.playback_delay_ms() > 0 {
                    0
                } else {
                    config.playback_buffer_ms
                });
                let mut report = Instant::now();
                let mut membership = Instant::now() - Duration::from_secs(1);
                let mut peers = HashSet::new();
                let mut open = false;
                let mut failures = 0u64;
                let mut retry = Instant::now();
                let mut pending = Vec::<i16>::new();
                let mut cursor = 0;
                while !stopped.load(Ordering::Acquire) {
                    if membership.elapsed() >= Duration::from_millis(20) {
                        membership = Instant::now();
                        let state = session.snapshot();
                        peers = state
                            .members
                            .iter()
                            .filter(|m| state.in_room && m.id != state.local_id && m.sharing)
                            .map(|m| m.id.clone())
                            .collect();
                        mixer.retain(&peers);
                        for peer in &peers {
                            mixer.set_muted(peer, session.audio_muted(peer));
                            mixer.set_volume(peer, session.audio_volume(peer));
                        }
                    }
                    if !FOREGROUND.load(Ordering::Acquire) || peers.is_empty() {
                        let _ = session.take_audio();
                        mixer.clear();
                        pending.clear();
                        cursor = 0;
                        if open {
                            bridge.close_audio();
                            open = false;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
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
                    if Instant::now() >= retry {
                        let free = bridge.audio_available();
                        if free < 0 {
                            failures += 1;
                            bridge.close_audio();
                            open = false;
                            pending.clear();
                            cursor = 0;
                            retry = Instant::now() + Duration::from_secs(2);
                        } else {
                            open = true;
                            if pending.is_empty() && free >= FRAMES as i32 {
                                let mut pcm = [0.0; SAMPLES];
                                mixer.render(Instant::now(), &mut pcm);
                                pending
                                    .extend(pcm.into_iter().map(|s| (s * 32767.0).round() as i16));
                                cursor = 0;
                            }
                            if !pending.is_empty() {
                                let written = bridge.write_audio(&pending[cursor..]);
                                if written < 0 {
                                    failures += 1;
                                    bridge.close_audio();
                                    open = false;
                                    pending.clear();
                                    cursor = 0;
                                    retry = Instant::now() + Duration::from_secs(2);
                                } else {
                                    cursor += written as usize;
                                    if cursor >= pending.len() {
                                        pending.clear();
                                        cursor = 0;
                                    }
                                }
                            }
                        }
                    }
                    if report.elapsed() >= Duration::from_secs(1) {
                        if p2pshare_media::diagnostics::enabled() {
                            tracing::info!(
                                received = mixer.stats.received,
                                played = mixer.stats.played,
                                concealed = mixer.stats.concealed,
                                late = mixer.stats.late,
                                overflows = mixer.stats.overflows,
                                resets = mixer.stats.resets,
                                device_failures = failures,
                                "audio_metrics"
                            );
                            p2pshare_media::diagnostics::append(&bridge.audio_diagnostics());
                        }
                        report = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                bridge.close_audio();
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for AudioReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
impl Bridge {
    fn audio_available(&self) -> i32 {
        self.env(|env| {
            env.call_method(self.activity.as_obj(), "audioAvailable", "()I", &[])?
                .i()
        })
        .unwrap_or(-1)
    }
    fn write_audio(&self, pcm: &[i16]) -> i32 {
        self.env(|env| {
            let array = env.new_short_array(pcm.len() as i32)?;
            env.set_short_array_region(&array, 0, pcm)?;
            env.call_method(
                self.activity.as_obj(),
                "writeAudio",
                "([S)I",
                &[JValue::Object(array.as_ref())],
            )?
            .i()
        })
        .unwrap_or(-1)
    }
    fn close_audio(&self) {
        let _ = self.env(|env| {
            env.call_method(self.activity.as_obj(), "closeAudio", "()V", &[])?;
            Ok(())
        });
    }
    fn audio_diagnostics(&self) -> String {
        self.env(|env| {
            let value = env
                .call_method(
                    self.activity.as_obj(),
                    "audioDiagnostics",
                    "()Ljava/lang/String;",
                    &[],
                )?
                .l()?;
            Ok(env.get_string((&value).into())?.into())
        })
        .unwrap_or_default()
    }
}
