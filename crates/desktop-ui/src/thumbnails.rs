//! Static picker snapshots: one WGC session at a time, GPU downscale, bounded readback.
//! No preview work occurs after the picker closes; full-resolution frames never reach the UI.
use p2pshare_media::{CaptureBackend, CaptureSource};
use p2pshare_platform_windows::{GraphicsDevice, WindowsCapture};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use windows::Win32::{
    Graphics::{Direct3D11::*, Dxgi::Common::*},
    System::WinRT::*,
};
const W: u32 = 360;
const H: u32 = 202;
enum Request {
    Sources(u64, Vec<CaptureSource>),
    Quit,
}
pub struct Thumbnails {
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<(u64, u64, egui::ColorImage)>,
    generation: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Thumbnails {
    pub fn new() -> std::io::Result<Self> {
        let (tx, requests) = mpsc::channel();
        let (output, rx) = mpsc::sync_channel(2);
        let generation = Arc::new(AtomicU64::new(0));
        let state = generation.clone();
        let thread = std::thread::Builder::new()
            .name("source-thumbnails".into())
            .spawn(move || {
                if let Err(e) = worker(requests, output, state) {
                    tracing::warn!(%e,"source_previews_unavailable");
                }
            })?;
        Ok(Self {
            tx,
            rx,
            generation,
            thread: Some(thread),
        })
    }
    pub fn set_sources(&self, sources: Vec<CaptureSource>) {
        let g = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.tx.send(Request::Sources(g, sources));
    }
    pub fn take(&self) -> Option<(u64, egui::ColorImage)> {
        while let Ok((g, id, img)) = self.rx.try_recv() {
            if g == self.generation.load(Ordering::Acquire) {
                return Some((id, img));
            }
        }
        None
    }
}
impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let _ = self.tx.send(Request::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}
fn worker(
    requests: mpsc::Receiver<Request>,
    output: mpsc::SyncSender<(u64, u64, egui::ColorImage)>,
    generation: Arc<AtomicU64>,
) -> anyhow::Result<()> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let _apartment = Apartment;
    // Create the GPU device lazily on the first picker request.
    let mut state: Option<(GraphicsDevice, egui_directx11::Renderer)> = None;
    let mut sources = Vec::new();
    let mut g = 0;
    loop {
        let request = if sources.is_empty() {
            state = None;
            match requests.recv() {
                Ok(r) => Some(r),
                Err(_) => break,
            }
        } else {
            match requests.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => None,
            }
        };
        if let Some(r) = request {
            match r {
                Request::Quit => break,
                Request::Sources(gg, ss) => {
                    g = gg;
                    sources = ss;
                    sources.reverse();
                }
            }
        }
        if sources.is_empty() {
            state = None;
            continue;
        }
        if state.is_none() {
            let graphics = GraphicsDevice::new()?;
            let renderer = egui_directx11::Renderer::new(&graphics.device)?;
            state = Some((graphics, renderer));
        }
        // Consume each source exactly once, then sleep until the picker reopens.
        let source = sources.pop().unwrap();
        let (graphics, renderer) = state.as_mut().unwrap();
        let mut capture = WindowsCapture::new(graphics.clone());
        let result = (|| -> anyhow::Result<Option<egui::ColorImage>> {
            capture.start(&source)?;
            let deadline = Instant::now() + Duration::from_millis(350);
            while Instant::now() < deadline && generation.load(Ordering::Acquire) == g {
                if let Some(frame) = capture.poll_frame()? {
                    return Ok(Some(downscale(graphics, renderer, &frame.texture)?));
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            Ok(None)
        })();
        let _ = capture.stop();
        match result {
            Ok(Some(image)) => {
                send_snapshot(&output, &generation, (g, source.id, image));
            }
            Err(e) => tracing::debug!(%e,"source_preview_skipped"),
            _ => {}
        }
    }
    Ok(())
}
fn send_snapshot(
    output: &mpsc::SyncSender<(u64, u64, egui::ColorImage)>,
    generation: &AtomicU64,
    mut snapshot: (u64, u64, egui::ColorImage),
) {
    // A static snapshot has no next animated frame to replace it. Preserve it
    // across bounded-queue backpressure, but cancel immediately on close/reopen.
    while generation.load(Ordering::Acquire) == snapshot.0 {
        match output.try_send(snapshot) {
            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => return,
            Err(mpsc::TrySendError::Full(pending)) => snapshot = pending,
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}
fn downscale(
    graphics: &GraphicsDevice,
    renderer: &mut egui_directx11::Renderer,
    source: &ID3D11Texture2D,
) -> anyhow::Result<egui::ColorImage> {
    render_thumbnail(graphics, renderer, source, W, H)
}
pub(crate) fn jpeg_snapshot(
    graphics: &GraphicsDevice,
    renderer: &mut egui_directx11::Renderer,
    source: &ID3D11Texture2D,
) -> anyhow::Result<Vec<u8>> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe {
        source.GetDesc(&mut desc);
    }
    let max = p2pshare_media::preview::MAX_EDGE;
    let size = if desc.Width <= max && desc.Height <= max {
        p2pshare_media::Dimensions::new(desc.Width, desc.Height)
    } else {
        p2pshare_media::Dimensions::new(desc.Width, desc.Height)
            .fit(p2pshare_media::Dimensions::new(max, max))
    };
    let image = render_thumbnail(graphics, renderer, source, size.width, size.height)?;
    let rgba: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    p2pshare_media::preview::encode(size.width, size.height, &rgba)
}
fn render_thumbnail(
    graphics: &GraphicsDevice,
    renderer: &mut egui_directx11::Renderer,
    source: &ID3D11Texture2D,
    width: u32,
    height: u32,
) -> anyhow::Result<egui::ColorImage> {
    unsafe {
        let mut srv = None;
        graphics
            .device
            .CreateShaderResourceView(source, None, Some(&mut srv))?;
        let id = renderer.register_user_texture(srv.unwrap());
        let result = (|| -> anyhow::Result<egui::ColorImage> {
            let mut texture = None;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                ..Default::default()
            };
            graphics
                .device
                .CreateTexture2D(&desc, None, Some(&mut texture))?;
            let texture = texture.unwrap();
            let mut target = None;
            graphics
                .device
                .CreateRenderTargetView(&texture, None, Some(&mut target))?;
            let target = target.unwrap();
            let mut original = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut original);
            let scale =
                (width as f32 / original.Width as f32).min(height as f32 / original.Height as f32);
            let ctx = egui::Context::default();
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width as f32, height as f32),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let r = egui::Rect::from_center_size(
                        egui::pos2(width as f32 / 2., height as f32 / 2.),
                        egui::vec2(
                            original.Width as f32 * scale,
                            original.Height as f32 * scale,
                        ),
                    );
                    ui.painter().image(
                        id,
                        r,
                        egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                        egui::Color32::WHITE,
                    );
                },
            );
            graphics
                .context
                .ClearRenderTargetView(&target, &[0.025, 0.025, 0.035, 1.]);
            let (render, _, _) = egui_directx11::split_output(output);
            renderer.render(&graphics.context, &target, &ctx, render)?;
            graphics.context.OMSetRenderTargets(None, None);
            let mut staging = None;
            let cpu = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..desc
            };
            graphics
                .device
                .CreateTexture2D(&cpu, None, Some(&mut staging))?;
            let staging = staging.unwrap();
            graphics.context.CopyResource(&staging, &texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            graphics
                .context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let mut rgba = vec![0u8; (width * height * 4) as usize];
            for y in 0..height as usize {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    (width * 4) as usize,
                );
                rgba[y * (width * 4) as usize..(y + 1) * (width * 4) as usize].copy_from_slice(row);
            }
            graphics.context.Unmap(&staging, 0);
            Ok(egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                &rgba,
            ))
        })();
        renderer.unregister_user_texture(id);
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_snapshot_survives_queue_backpressure_and_cancels_on_close() {
        let (tx, rx) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(1));
        let image = || egui::ColorImage::new([1, 1], vec![egui::Color32::RED]);
        tx.send((1, 1, image())).unwrap();
        let state = generation.clone();
        let sender = tx.clone();
        let snapshot = (1, 2, image());
        let thread = std::thread::spawn(move || send_snapshot(&sender, &state, snapshot));
        assert_eq!(rx.recv().unwrap().1, 1);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().1, 2);
        thread.join().unwrap();
        tx.send((1, 3, image())).unwrap();
        let state = generation.clone();
        let snapshot = (1, 4, image());
        let thread = std::thread::spawn(move || send_snapshot(&tx, &state, snapshot));
        generation.store(2, Ordering::Release);
        thread.join().unwrap();
        assert_eq!(rx.recv().unwrap().1, 3);
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn thumbnail_downscale_keeps_color_and_aspect_without_a_window() {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED).unwrap();
        }
        let _a = Apartment;
        let g = GraphicsDevice::new().unwrap();
        let mut renderer = egui_directx11::Renderer::new(&g.device).unwrap();
        unsafe {
            let mut texture = None;
            g.device
                .CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Width: 640,
                        Height: 480,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Usage: D3D11_USAGE_DEFAULT,
                        BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0 as u32,
                        ..Default::default()
                    },
                    None,
                    Some(&mut texture),
                )
                .unwrap();
            let texture = texture.unwrap();
            let mut target = None;
            g.device
                .CreateRenderTargetView(&texture, None, Some(&mut target))
                .unwrap();
            g.context
                .ClearRenderTargetView(&target.unwrap(), &[0.9, 0.1, 0.2, 1.]);
            let jpeg = jpeg_snapshot(&g, &mut renderer, &texture).unwrap();
            let preview = p2pshare_media::preview::decode(&jpeg).unwrap();
            assert_eq!((preview.width, preview.height), (512, 384));
            let image = downscale(&g, &mut renderer, &texture).unwrap();
            let center = image.pixels[(H / 2 * W + W / 2) as usize];
            assert!(center.r() > 210 && center.g() < 40 && center.b() > 40);
            let edge = image.pixels[(H / 2 * W) as usize];
            assert!(edge.r() < 20 && edge.g() < 20);
        }
    }
}
