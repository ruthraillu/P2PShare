//! Explicit, local-only MusicBee static-frame quality diagnostic. Never injects OS input.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use p2pshare_codecs_windows::{
        Readback,
        ffmpeg::{Decoder, Encoder},
    };
    use p2pshare_media::{
        CaptureBackend,
        config::{Codec, StreamConfig, VideoFormat},
    };
    use p2pshare_platform_windows::{GraphicsDevice, WindowsCapture, enumerate_sources};
    use std::{
        io::Write,
        time::{Duration, Instant},
    };
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    let args: Vec<_> = std::env::args().collect();
    let hwnd: u64 = args
        .get(1)
        .context("Supply the explicitly authorized MusicBee HWND")?
        .parse()?;
    let out = std::path::PathBuf::from(args.get(2).context("Supply an F: output directory")?);
    std::fs::create_dir_all(&out)?;
    let source = enumerate_sources(None)?
        .into_iter()
        .find(|s| s.id == hwnd && s.title.to_lowercase().contains("musicbee"))
        .context("Authorized MusicBee window is not visible; no other window will be captured")?;
    let settings = StreamConfig {
        codec: Codec::Hevc,
        bitrate_kbps: 6000,
        fps: 60,
        width: 1920,
        height: 1080,
        software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
        ..Default::default()
    };
    let graphics = GraphicsDevice::new()?;
    let mut capture = WindowsCapture::new(graphics.clone());
    capture.start(&source)?;
    let mut readback = Readback::new(graphics)?;
    readback.set_config(settings);
    let deadline = Instant::now() + Duration::from_secs(5);
    let raw = loop {
        if let Some(frame) = capture.poll_frame()? {
            readback.submit(&frame.texture, frame.metadata.size)?;
        }
        if let Some(raw) = readback.poll()? {
            break raw;
        }
        ensure!(
            Instant::now() < deadline,
            "MusicBee produced no frame; it may be minimized"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    let size = raw.size;
    let base = raw.bytes;
    println!(
        "MusicBee snapshot: {}x{}, 6 Mbps HEVC/60fps model; synthetic cursor only",
        size.width, size.height
    );
    let ppm = |path: &std::path::Path, bytes: &[u8], bgra: bool| -> anyhow::Result<()> {
        let mut file = std::fs::File::create(path)?;
        write!(file, "P6\n{} {}\n255\n", size.width, size.height)?;
        let mut rgb = Vec::with_capacity(bytes.len() / 4 * 3);
        for p in bytes.chunks_exact(4) {
            if bgra {
                rgb.extend_from_slice(&[p[2], p[1], p[0]]);
            } else {
                rgb.extend_from_slice(&p[..3]);
            }
        }
        file.write_all(&rgb)?;
        Ok(())
    };
    ppm(&out.join("reference.ppm"), &base, true)?;
    let cursor = |frame: &mut [u8], x: usize, y: usize| {
        for dy in 0..24 {
            for dx in 0..=(dy / 2).min(10) {
                let px = (x + dx).min(size.width as usize - 1);
                let py = (y + dy).min(size.height as usize - 1);
                let at = (py * size.width as usize + px) * 4;
                let white = dx == 0 || dx == dy / 2 || dy == 23;
                let c = if white { 255 } else { 0 };
                frame[at..at + 4].copy_from_slice(&[c, c, c, 255]);
            }
        }
    };
    let mut reports = Vec::new();
    for refinement in [false, true] {
        let mut encoder = Encoder::new("hevc_nvenc", VideoFormat::Hevc, size, settings, None, 0)?;
        let mut packets = Vec::new();
        let mut frame = base.clone();
        let mut x = 100usize;
        let mut last_activity = 0;
        let start = Instant::now();
        for i in 0..600usize {
            let moving = i < 120 || (240..360).contains(&i) || (480..540).contains(&i);
            let key = i % 120 == 0;
            if moving {
                x = 100 + (i % 120) * 4;
            }
            if moving || key {
                last_activity = i;
            }
            if moving || key || (refinement && i < last_activity + 16) {
                frame.copy_from_slice(&base);
                cursor(&mut frame, x, 100);
                packets.extend(encoder.submit(
                    Some(&frame),
                    None,
                    i as u64 * 1_000_000 / 60,
                    key,
                )?);
            }
            if !refinement {
                let _ = capture.poll_frame()?;
                if let Some(wait) = (start + Duration::from_micros((i as u64 + 1) * 1_000_000 / 60))
                    .checked_duration_since(Instant::now())
                {
                    std::thread::sleep(wait);
                }
            }
        }
        if !refinement {
            capture.stop()?;
        }
        drop(encoder);
        let label = if refinement { "refinement" } else { "current" };
        let bytes: usize = packets.iter().map(|p| p.bytes.len()).sum();
        let mut decoder = Decoder::new(VideoFormat::Hevc, size, 1, false, 0)?;
        let mut samples = Vec::new();
        for (n, packet) in packets.iter().enumerate() {
            if let Some(decoded) = decoder.decode(&packet.bytes)? {
                let next = packets.get(n + 1).map_or(u64::MAX, |p| p.micros);
                for sample_us in [3_900_000, 7_900_000] {
                    if packet.micros <= sample_us && next > sample_us {
                        let mut error = 0f64;
                        let mut count = 0;
                        // Ignore the simulated cursor; measure MusicBee text/image fidelity.
                        for (i, (a, b)) in base
                            .chunks_exact(4)
                            .zip(decoded.rgba.chunks_exact(4))
                            .enumerate()
                        {
                            if i / (size.width as usize) < 150 {
                                continue;
                            }
                            let original =
                                (54. * a[2] as f64 + 183. * a[1] as f64 + 19. * a[0] as f64) / 256.;
                            let actual =
                                (54. * b[0] as f64 + 183. * b[1] as f64 + 19. * b[2] as f64) / 256.;
                            error += (original - actual).powi(2);
                            count += 1;
                        }
                        let psnr =
                            10. * (255f64.powi(2) / (error / count as f64).max(0.000001)).log10();
                        samples.push(serde_json::json!({"at_seconds":sample_us as f64/1e6,"last_encoded_frame_seconds":packet.micros as f64/1e6,"luma_psnr_db":psnr}));
                        ppm(
                            &out.join(format!("{label}-{sample_us}.ppm")),
                            &decoded.rgba,
                            false,
                        )?;
                    }
                }
            }
        }
        let report = serde_json::json!({"mode":label,"frames":packets.len(),"bytes":bytes,"samples":samples});
        println!("{report}");
        reports.push(report);
    }
    std::fs::write(
        out.join("results.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows-only diagnostic");
}
