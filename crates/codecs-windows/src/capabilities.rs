//! First-start real codec probes, persisted next to the executable, with driver invalidation.
use crate::ffmpeg;
use anyhow::{Context, Result};
use p2pshare_media::{
    Dimensions,
    config::{Codec, EncoderBackend, StreamConfig, VideoFormat},
};
use p2pshare_platform_windows::GraphicsDevice;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};
#[derive(Clone, Serialize, Deserialize)]
pub struct Adapter {
    pub index: u32,
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub sub_system: u32,
    pub revision: u32,
    pub luid: i64,
    pub driver: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Probe {
    pub adapter: u32,
    pub name: String,
    pub codec: VideoFormat,
    pub supported: bool,
    pub gpu_input: bool,
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Capabilities {
    schema: u32,
    app_version: String,
    ffmpeg_version: u32,
    driver_fingerprint: String,
    runtime_identity: String,
    cpu: String,
    pub adapters: Vec<Adapter>,
    pub encoders: Vec<Probe>,
    pub decoders: Vec<Probe>,
    probe_width: u32,
    probe_height: u32,
    probe_fps: u32,
}
type State = Arc<(Mutex<Option<Result<Capabilities, String>>>, Condvar)>;
static STATE: OnceLock<State> = OnceLock::new();
fn path() -> Result<PathBuf> {
    Ok(p2pshare_media::portable::data_dir()?.join("hardware-capabilities.json"))
}
pub fn start(rescan: bool) {
    STATE.get_or_init(|| {
        let state = Arc::new((Mutex::new(None), Condvar::new()));
        let output = state.clone();
        match std::thread::Builder::new()
            .name("p2pshare-codec-probes".into())
            .spawn(move || {
                let _apartment = crate::native::CodecApartment::new();
                let result = load_or_probe(rescan).map_err(|e| format!("{e:#}"));
                if let Err(error) = &result {
                    tracing::error!(%error,"hardware_probe_failed");
                }
                *output.0.lock().unwrap() = Some(result);
                output.1.notify_all();
            }) {
            Ok(_) => {}
            Err(error) => {
                *state.0.lock().unwrap() = Some(Err(error.to_string()));
            }
        }
        state
    });
}
fn get() -> Result<Capabilities> {
    start(false);
    let state = STATE.get().unwrap();
    let result = state
        .1
        .wait_timeout_while(state.0.lock().unwrap(), Duration::from_secs(60), |s| {
            s.is_none()
        })
        .unwrap()
        .0;
    result
        .as_ref()
        .context("Codec probe timed out")?
        .clone()
        .map_err(|e| anyhow::anyhow!(e))
}
fn inventory() -> Vec<Adapter> {
    (0..16)
        .map_while(|index| {
            ffmpeg::gpu(index).map(|g| Adapter {
                index,
                name: String::from_utf16_lossy(
                    &g.name[..g.name.iter().position(|c| *c == 0).unwrap_or(128)],
                ),
                vendor: g.vendor,
                device: g.device,
                sub_system: g.sub_system,
                revision: g.revision,
                luid: g.luid,
                driver: g.driver,
            })
        })
        .collect()
}
fn load_or_probe(rescan: bool) -> Result<Capabilities> {
    if rescan {
        crate::profiles::invalidate();
    }
    let runtime = ffmpeg::runtime_version()?;
    let adapters = inventory();
    let drivers = ffmpeg::driver_fingerprint();
    let runtime_identity = ffmpeg::runtime_identity()?;
    let cpu = ffmpeg::cpu_description();
    tracing::info!(cpu=%cpu,adapters=adapters.len(),"hardware_inventory");
    for a in &adapters {
        tracing::info!(adapter=a.index,name=%a.name,vendor=a.vendor,device=a.device,luid=a.luid,driver=a.driver,"hardware_adapter");
    }
    let file = path()?;
    if !rescan
        && std::fs::metadata(&file).is_ok_and(|m| m.len() < 128 * 1024)
        && let Ok(bytes) = std::fs::read(&file)
        && bytes.len() < 128 * 1024
        && let Ok(mut saved) = serde_json::from_slice::<Capabilities>(&bytes)
        && saved.schema == 4
        && saved.runtime_identity == runtime_identity
        && saved.cpu == cpu
        && cache_consistent(&saved)
        && saved.app_version == env!("CARGO_PKG_VERSION")
        && saved.ffmpeg_version == runtime
        && saved.driver_fingerprint == drivers
        && saved.adapters.len() == adapters.len()
        && saved.adapters.iter().zip(&adapters).all(|(old, new)| {
            (
                old.index,
                &old.name,
                old.vendor,
                old.device,
                old.sub_system,
                old.revision,
                old.driver,
            ) == (
                new.index,
                &new.name,
                new.vendor,
                new.device,
                new.sub_system,
                new.revision,
                new.driver,
            )
        })
    {
        tracing::info!(path=%file.display(),"hardware_capabilities_cache_hit");
        // DXGI LUID changes across Windows boots; refresh it in memory without re-probing.
        saved.adapters = adapters;
        return Ok(saved);
    }
    tracing::info!(
        adapters = adapters.len(),
        "hardware_capabilities_probe_started"
    );
    let started = Instant::now();
    let size = Dimensions::new(640, 360);
    let settings = StreamConfig {
        fps: 60,
        bitrate_kbps: 2000,
        ..Default::default()
    };
    let bgra = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
    let mut encoders = Vec::new();
    let mut decoders = Vec::new();
    let mut fixtures = Vec::new();
    for format in [VideoFormat::H264, VideoFormat::Hevc] {
        let name = if format == VideoFormat::H264 {
            "libx264"
        } else {
            "libx265"
        };
        let result = (|| -> Result<Vec<Vec<u8>>> {
            let mut encoder = ffmpeg::Encoder::new(name, format, size, settings, None, 0)?;
            let mut packets = Vec::new();
            for i in 0..4 {
                packets.extend(
                    encoder
                        .submit(Some(&bgra), None, i * 16667, i == 0)?
                        .into_iter()
                        .map(|f| f.bytes),
                );
            }
            anyhow::ensure!(packets.len() == 4, "Encoder did not emit all probe frames");
            Ok(packets)
        })();
        encoders.push(Probe {
            adapter: 0,
            name: name.into(),
            codec: format,
            supported: result.is_ok(),
            gpu_input: false,
            error: result.as_ref().err().map(|e| format!("{e:#}")),
        });
        if let Ok(bytes) = result {
            fixtures.push((format, bytes));
        }
    }
    for adapter in &adapters {
        let family = match adapter.vendor {
            0x10de => Some("nvenc"),
            0x1002 => Some("amf"),
            0x8086 => Some("qsv"),
            _ => None,
        };
        let Some(family) = family else {
            continue;
        };
        for format in [VideoFormat::H264, VideoFormat::Hevc] {
            let name = format!(
                "{}_{}",
                if format == VideoFormat::H264 {
                    "h264"
                } else {
                    "hevc"
                },
                family
            );
            let mut gpu_input = family != "qsv";
            let mut result = (|| -> Result<()> {
                if gpu_input {
                    let graphics = GraphicsDevice::from_device(ffmpeg::device(adapter.index)?)?;
                    let mut texture = None;
                    unsafe {
                        graphics.device.CreateTexture2D(
                            &D3D11_TEXTURE2D_DESC {
                                Width: size.width,
                                Height: size.height,
                                MipLevels: 1,
                                ArraySize: 1,
                                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                SampleDesc: DXGI_SAMPLE_DESC {
                                    Count: 1,
                                    Quality: 0,
                                },
                                Usage: D3D11_USAGE_DEFAULT,
                                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0
                                    | D3D11_BIND_RENDER_TARGET.0)
                                    as u32,
                                ..Default::default()
                            },
                            Some(&D3D11_SUBRESOURCE_DATA {
                                pSysMem: bgra.as_ptr().cast(),
                                SysMemPitch: size.width * 4,
                                SysMemSlicePitch: 0,
                            }),
                            Some(&mut texture),
                        )?;
                    }
                    let mut encoder = crate::GpuEncoder::new_backend(
                        graphics,
                        size,
                        settings,
                        &name,
                        format,
                        adapter.index,
                    )?;
                    let mut packets = Vec::new();
                    for i in 0..4 {
                        packets.extend(encoder.encode(
                            texture.as_ref().unwrap(),
                            i * 16667,
                            i == 0 || i == 3,
                        )?);
                    }
                    verify_recovery_probe(&packets, format, size)?;
                } else {
                    let mut encoder =
                        ffmpeg::Encoder::new(&name, format, size, settings, None, adapter.index)?;
                    let mut packets = Vec::new();
                    for i in 0..4 {
                        packets.extend(encoder.submit(
                            Some(&bgra),
                            None,
                            i * 16667,
                            i == 0 || i == 3,
                        )?);
                    }
                    verify_recovery_probe(&packets, format, size)?;
                }
                Ok(())
            })();
            if result.is_err() && gpu_input {
                let direct_error = format!("{:#}", result.as_ref().unwrap_err());
                gpu_input = false;
                result = (|| -> Result<()> {
                    let mut encoder =
                        ffmpeg::Encoder::new(&name, format, size, settings, None, adapter.index)?;
                    let mut packets = Vec::new();
                    for i in 0..4 {
                        packets.extend(encoder.submit(
                            Some(&bgra),
                            None,
                            i * 16667,
                            i == 0 || i == 3,
                        )?);
                    }
                    verify_recovery_probe(&packets, format, size)
                })()
                .with_context(|| {
                    format!("GPU input unavailable: {direct_error}; CPU upload failed")
                });
            }
            tracing::info!(
                backend = name,
                adapter=adapter.index,
                adapter_name=%adapter.name,
                gpu_input,
                supported = result.is_ok(),
                "encoder_probe_result"
            );
            encoders.push(Probe {
                adapter: adapter.index,
                name,
                codec: format,
                supported: result.is_ok(),
                gpu_input,
                error: result.err().map(|e| format!("{e:#}")),
            });
        }
        for (format, packets) in &fixtures {
            let result = (|| -> Result<()> {
                let mut decoder = ffmpeg::Decoder::new(*format, size, 2, true, adapter.index)?;
                let mut decoded = 0;
                for packet in packets {
                    if decoder.decode(packet)?.is_some() {
                        decoded += 1;
                    }
                }
                anyhow::ensure!(
                    decoded == packets.len() && decoder.hardware,
                    "Hardware decoding was not actually used"
                );
                Ok(())
            })();
            decoders.push(Probe {
                adapter: adapter.index,
                name: "d3d11va".into(),
                codec: *format,
                supported: result.is_ok(),
                gpu_input: true,
                error: result.err().map(|e| format!("{e:#}")),
            });
        }
    }
    let saved = Capabilities {
        schema: 4,
        app_version: env!("CARGO_PKG_VERSION").into(),
        ffmpeg_version: runtime,
        driver_fingerprint: drivers,
        runtime_identity,
        cpu,
        adapters,
        encoders,
        decoders,
        probe_width: size.width,
        probe_height: size.height,
        probe_fps: 60,
    };
    let temp = file.with_extension(format!("{}.tmp", std::process::id()));
    if let Err(error) = (|| -> Result<()> {
        std::fs::write(&temp, serde_json::to_vec_pretty(&saved)?)?;
        std::fs::rename(&temp, &file)?;
        Ok(())
    })() {
        tracing::warn!(%error,"hardware_cache_write_failed_using_memory_results");
        let _ = std::fs::remove_file(&temp);
    }
    tracing::info!(elapsed_ms=started.elapsed().as_millis(),path=%file.display(),"hardware_capabilities_probe_finished");
    Ok(saved)
}
#[derive(Clone)]
pub(crate) struct Candidate {
    pub name: String,
    pub codec: VideoFormat,
    pub adapter: u32,
}
pub(crate) fn candidates(
    settings: StreamConfig,
    device: Option<&ID3D11Device>,
    gpu_only: bool,
) -> Result<Vec<Candidate>> {
    let cache = match get() {
        Ok(cache) => cache,
        Err(error) => {
            if matches!(
                settings.encoder,
                EncoderBackend::Auto | EncoderBackend::Software
            ) && !gpu_only
            {
                tracing::warn!(%error,"hardware_inventory_unavailable_using_software");
                let codec = if settings.codec == Codec::Hevc {
                    VideoFormat::Hevc
                } else {
                    VideoFormat::H264
                };
                return Ok(vec![Candidate {
                    name: if codec == VideoFormat::Hevc {
                        "libx265"
                    } else {
                        "libx264"
                    }
                    .into(),
                    codec,
                    adapter: 0,
                }]);
            }
            return Err(error);
        }
    };
    let luid = device.map(ffmpeg::luid);
    let preferred = cache
        .adapters
        .iter()
        .find(|g| Some(g.luid) == luid)
        .map(|g| g.index);
    let formats = match settings.codec {
        Codec::Auto => vec![VideoFormat::Hevc, VideoFormat::H264],
        Codec::Avc => vec![VideoFormat::H264],
        Codec::Hevc => vec![VideoFormat::Hevc],
    };
    let mut output = Vec::new();
    if settings.encoder != EncoderBackend::Software {
        for format in formats {
            let mut probes: Vec<_> = cache
                .encoders
                .iter()
                .filter(|p| {
                    (p.supported
                        || p.error.as_ref().is_some_and(|e| {
                            let e = e.to_ascii_lowercase();
                            ["memory", "session", "busy", "temporar", "timeout"]
                                .iter()
                                .any(|word| e.contains(word))
                        }))
                        && p.codec == format
                        && cache.adapters.iter().any(|a| {
                            a.index == p.adapter
                                && crate::adapter_policy::encoder_matches(
                                    &p.name, p.codec, a.vendor,
                                )
                        })
                        && (!gpu_only || (p.gpu_input && Some(p.adapter) == preferred))
                })
                .collect();
            probes.sort_by_key(|p| Some(p.adapter) != preferred);
            for p in probes {
                let allowed = match settings.encoder {
                    EncoderBackend::Auto => true,
                    EncoderBackend::Nvenc => p.name.ends_with("nvenc"),
                    EncoderBackend::Amf => p.name.ends_with("amf"),
                    EncoderBackend::Qsv => p.name.ends_with("qsv"),
                    EncoderBackend::Software => false,
                };
                if allowed {
                    output.push(Candidate {
                        name: p.name.clone(),
                        codec: p.codec,
                        adapter: p.adapter,
                    });
                }
            }
        }
    }
    if !gpu_only
        && matches!(
            settings.encoder,
            EncoderBackend::Auto | EncoderBackend::Software
        )
    {
        let codec = if settings.codec == Codec::Hevc {
            VideoFormat::Hevc
        } else {
            VideoFormat::H264
        };
        output.push(Candidate {
            name: if codec == VideoFormat::Hevc {
                "libx265"
            } else {
                "libx264"
            }
            .into(),
            codec,
            adapter: 0,
        });
    }
    Ok(output)
}
pub(crate) fn open_cpu_encoder(
    settings: StreamConfig,
    size: Dimensions,
    failed: &mut std::collections::HashSet<(String, u32)>,
) -> Result<ffmpeg::Encoder> {
    open_cpu_candidates(settings, size, failed, candidates(settings, None, false)?)
}
fn open_cpu_candidates(
    settings: StreamConfig,
    size: Dimensions,
    failed: &mut std::collections::HashSet<(String, u32)>,
    choices: Vec<Candidate>,
) -> Result<ffmpeg::Encoder> {
    let mut errors = Vec::new();
    for c in choices {
        if failed.contains(&(c.name.clone(), c.adapter)) {
            continue;
        }
        let result = (|| -> Result<ffmpeg::Encoder> {
            crate::profiles::cpu(&c.name, c.codec, size, settings, c.adapter)?;
            ffmpeg::Encoder::new(&c.name, c.codec, size, settings, None, c.adapter)
        })();
        match result {
            Ok(encoder) => return Ok(encoder),
            Err(error) => {
                tracing::warn!(backend=%c.name,adapter=c.adapter,%error,"encoder_candidate_rejected");
                failed.insert((c.name, c.adapter));
                errors.push(format!("{error:#}"));
            }
        }
    }
    anyhow::bail!("No usable encoder: {}", errors.join("; "))
}
pub(crate) fn decoder(
    format: VideoFormat,
    size: Dimensions,
    threads: u32,
    display: Option<&ID3D11Device>,
) -> Result<ffmpeg::Decoder> {
    if let Ok(cache) = get() {
        for p in cache.decoders.iter().filter(|p| {
            p.codec == format
                && p.supported
                && p.name == "d3d11va"
                && cache.adapters.iter().any(|a| a.index == p.adapter)
        }) {
            let display = display.filter(|d| {
                cache
                    .adapters
                    .iter()
                    .any(|a| a.index == p.adapter && a.luid == ffmpeg::luid(d))
            });
            match ffmpeg::Decoder::with_device(format, size, threads, true, p.adapter, display) {
                Ok(decoder) => {
                    tracing::info!(
                        codec = format.label(),
                        adapter = p.adapter,
                        "hardware_decoder_selected"
                    );
                    return Ok(decoder);
                }
                Err(error) => {
                    tracing::warn!(%error,"hardware_decoder_initialization_failed");
                    if display.is_some()
                        && let Ok(decoder) =
                            ffmpeg::Decoder::new(format, size, threads, true, p.adapter)
                    {
                        tracing::info!(
                            codec = format.label(),
                            adapter = p.adapter,
                            "hardware_decoder_using_cpu_output"
                        );
                        return Ok(decoder);
                    }
                }
            }
        }
    }
    tracing::info!(codec = format.label(), "software_decoder_selected");
    ffmpeg::Decoder::new(format, size, threads, false, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poisoned_vendor_entries_and_missing_adapters_are_rejected() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let original = load_or_probe(false)?;
        assert!(cache_consistent(&original));
        if let Some(adapter) = original.adapters.first() {
            let mut poisoned = original.clone();
            poisoned.encoders.push(Probe {
                adapter: adapter.index,
                name: if adapter.vendor == 0x1002 {
                    "h264_nvenc"
                } else {
                    "h264_amf"
                }
                .into(),
                codec: VideoFormat::H264,
                supported: true,
                gpu_input: true,
                error: None,
            });
            assert!(!cache_consistent(&poisoned));
            let mut missing = original.clone();
            missing.decoders.push(Probe {
                adapter: u32::MAX,
                name: "d3d11va".into(),
                codec: VideoFormat::H264,
                supported: true,
                gpu_input: true,
                error: None,
            });
            assert!(!cache_consistent(&missing));
        }
        Ok(())
    }

    #[test]
    fn failed_hardware_candidate_falls_back_to_real_software_encoder() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let settings = StreamConfig::default();
        let size = Dimensions::new(320, 180);
        let mut failed = std::collections::HashSet::new();
        let encoder = open_cpu_candidates(
            settings,
            size,
            &mut failed,
            vec![
                Candidate {
                    name: "h264_amf".into(),
                    codec: VideoFormat::H264,
                    adapter: u32::MAX,
                },
                Candidate {
                    name: "libx264".into(),
                    codec: VideoFormat::H264,
                    adapter: 0,
                },
            ],
        )?;
        assert_eq!(encoder.name, "libx264");
        assert!(failed.contains(&("h264_amf".into(), u32::MAX)));
        assert!(!failed.contains(&("libx264".into(), 0)));
        Ok(())
    }

    #[test]
    fn cached_probe_results_do_not_rewrite_file() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let first = load_or_probe(false)?;
        let modified = std::fs::metadata(path()?)?.modified()?;
        let second = load_or_probe(false)?;
        anyhow::ensure!(
            std::fs::metadata(path()?)?.modified()? == modified,
            "A matching cache must avoid rescanning"
        );
        anyhow::ensure!(
            first.encoders.len() == second.encoders.len(),
            "Probe cache changed"
        );
        if !second.adapters.is_empty() {
            let original = std::fs::read(path()?)?;
            let mut simulated = second.clone();
            for adapter in &mut simulated.adapters {
                adapter.luid = -1;
            }
            std::fs::write(path()?, serde_json::to_vec_pretty(&simulated)?)?;
            let before = std::fs::metadata(path()?)?.modified()?;
            let restored = load_or_probe(false)?;
            anyhow::ensure!(
                restored.adapters[0].luid == second.adapters[0].luid,
                "Refresh boot-local adapter IDs"
            );
            anyhow::ensure!(
                std::fs::metadata(path()?)?.modified()? == before,
                "A reboot alone must not run expensive probes"
            );
            std::fs::write(path()?, original)?;
        }
        Ok(())
    }
}

/// A byte-producing encoder is not sufficient: the joining viewer has no earlier
/// references or codec headers. Reject it before Auto selects it if recovery fails.
fn verify_recovery_probe(
    packets: &[crate::Encoded],
    format: VideoFormat,
    size: Dimensions,
) -> Result<()> {
    anyhow::ensure!(
        packets.len() == 4,
        "Hardware encoder did not emit all probe frames"
    );
    for index in [0usize, 3] {
        anyhow::ensure!(
            packets[index].keyframe,
            "Requested IDR was not emitted at probe frame {index}"
        );
        let mut fresh = ffmpeg::Decoder::new(format, size, 1, false, 0)?;
        let decoded = fresh
            .decode(&packets[index].bytes)?
            .context("Requested IDR cannot start a fresh decoder with its own headers")?;
        anyhow::ensure!(decoded.size == size, "Recovery frame dimensions changed");
    }
    Ok(())
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn probes_require_real_restart_frames_with_headers_not_just_a_keyframe_flag() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let size = Dimensions::new(320, 180);
        let pixels = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        for (format, name) in [
            (VideoFormat::H264, "libx264"),
            (VideoFormat::Hevc, "libx265"),
        ] {
            for restart in [false, true] {
                let mut encoder = ffmpeg::Encoder::new(
                    name,
                    format,
                    size,
                    StreamConfig {
                        software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
                        ..Default::default()
                    },
                    None,
                    0,
                )?;
                let mut packets = Vec::new();
                for i in 0..4 {
                    packets.extend(encoder.submit(
                        Some(&pixels),
                        None,
                        i * 16667,
                        i == 0 || (restart && i == 3),
                    )?);
                }
                if restart {
                    verify_recovery_probe(&packets, format, size)?;
                } else {
                    anyhow::ensure!(
                        verify_recovery_probe(&packets, format, size).is_err(),
                        "Probe accepted an inter frame at a recovery boundary"
                    );
                    packets[3].keyframe = true;
                    anyhow::ensure!(
                        verify_recovery_probe(&packets, format, size).is_err(),
                        "Probe trusted a flag without independently decodable headers/data"
                    );
                }
            }
        }
        Ok(())
    }
}

fn cache_consistent(cache: &Capabilities) -> bool {
    let mut indices = std::collections::HashSet::new();
    if !cache.adapters.iter().all(|a| indices.insert(a.index)) {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    if !cache.encoders.iter().all(|p| {
        seen.insert((p.adapter, p.name.clone(), p.codec.label()))
            && ((crate::adapter_policy::software_matches(&p.name, p.codec) && !p.gpu_input)
                || cache.adapters.iter().any(|a| {
                    a.index == p.adapter
                        && crate::adapter_policy::encoder_matches(&p.name, p.codec, a.vendor)
                        && !(p.gpu_input && p.name.ends_with("qsv"))
                }))
    }) {
        return false;
    }
    seen.clear();
    cache.decoders.iter().all(|p| {
        seen.insert((p.adapter, p.name.clone(), p.codec.label()))
            && p.name == "d3d11va"
            && cache.adapters.iter().any(|a| a.index == p.adapter)
    })
}
