//! GPU-resident video display and a single-upload path for software output.
use p2pshare_codecs_windows::DecodedFrame;
use p2pshare_media::Dimensions;
use p2pshare_platform_windows::GraphicsDevice;
use windows::{
    Win32::Graphics::{Direct3D11::*, Dxgi::Common::*},
    core::Result,
};

pub(super) struct VideoTexture {
    texture: ID3D11Texture2D,
    pub id: egui::TextureId,
    size: Dimensions,
    format: DXGI_FORMAT,
}
impl VideoTexture {
    pub fn upload(
        slot: &mut Option<Self>,
        graphics: &GraphicsDevice,
        renderer: &mut egui_directx11::Renderer,
        frame: &DecodedFrame,
    ) -> Result<egui::TextureId> {
        let format = if frame.gpu.is_some() {
            DXGI_FORMAT_B8G8R8A8_UNORM
        } else {
            DXGI_FORMAT_R8G8B8A8_UNORM
        };
        // SAFETY: texture dimensions/pixel lengths are validated by the codec shim;
        // D3D11 owns its allocations and the immediate context is multithread protected.
        unsafe {
            if slot
                .as_ref()
                .is_none_or(|s| s.size != frame.size || s.format != format)
            {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: frame.size.width,
                    Height: frame.size.height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    ..Default::default()
                };
                let mut texture = None;
                let mut srv = None;
                graphics
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))?;
                let texture = texture.unwrap();
                graphics
                    .device
                    .CreateShaderResourceView(&texture, None, Some(&mut srv))?;
                let id = renderer.register_user_texture(srv.unwrap());
                if let Some(old) = slot.replace(Self {
                    texture,
                    id,
                    size: frame.size,
                    format,
                }) {
                    renderer.unregister_user_texture(old.id);
                }
            }
            let target = slot.as_ref().unwrap();
            if let Some(gpu) = &frame.gpu {
                // Same-device GPU copy: no map/download, CPU conversion or upload.
                // The source lease survives until this copy has been submitted.
                graphics
                    .context
                    .CopyResource(&target.texture, &gpu.texture());
            } else {
                // FFmpeg produces opaque RGBA. Upload directly without egui's
                // premultiplication/ColorImage allocation and additional pixel copies.
                graphics.context.UpdateSubresource(
                    &target.texture,
                    0,
                    None,
                    frame.rgba.as_ptr().cast(),
                    frame.size.width * 4,
                    0,
                );
            }
            Ok(target.id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2pshare_codecs_windows::ffmpeg::{Decoder, Encoder};
    use p2pshare_media::config::{StreamConfig, VideoFormat};
    use windows::Win32::{Foundation::HMODULE, Graphics::Direct3D::D3D_DRIVER_TYPE_WARP};

    fn graphics() -> GraphicsDevice {
        unsafe {
            let mut device = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
            .unwrap();
            GraphicsDevice::from_device(device.unwrap()).unwrap()
        }
    }
    fn target(g: &GraphicsDevice, w: u32, h: u32) -> (ID3D11Texture2D, ID3D11RenderTargetView) {
        unsafe {
            let mut texture = None;
            let mut rtv = None;
            g.device
                .CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Width: w,
                        Height: h,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                        ..Default::default()
                    },
                    None,
                    Some(&mut texture),
                )
                .unwrap();
            let texture = texture.unwrap();
            g.device
                .CreateRenderTargetView(&texture, None, Some(&mut rtv))
                .unwrap();
            (texture, rtv.unwrap())
        }
    }
    fn pixel(g: &GraphicsDevice, texture: &ID3D11Texture2D) -> [u8; 4] {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            desc.BindFlags = 0;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            g.device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            g.context.CopyResource(&staging, texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            g.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let p = (mapped.pData as *const u8).add(
                (desc.Height / 2) as usize * mapped.RowPitch as usize
                    + (desc.Width / 2 * 4) as usize,
            );
            let result = [*p, *p.add(1), *p.add(2), *p.add(3)];
            g.context.Unmap(&staging, 0);
            result
        }
    }
    fn output(id: egui::TextureId, ppp: f32) -> egui_directx11::RendererOutput {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64., 64.));
        let mut mesh = egui::epaint::Mesh::with_texture(id);
        mesh.add_rect_with_uv(
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
            egui::Color32::WHITE,
        );
        egui_directx11::RendererOutput {
            textures_delta: Default::default(),
            shapes: vec![egui::epaint::ClippedShape {
                clip_rect: rect,
                shape: egui::Shape::mesh(mesh),
            }],
            pixels_per_point: ppp,
        }
    }
    #[test]
    fn software_video_upload_and_geometry_cache_update_pixels_and_invalidate_on_resize() {
        let g = graphics();
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |_| {});
        let mut renderer = egui_directx11::Renderer::new(&g.device).unwrap();
        let (texture, rtv) = target(&g, 64, 64);
        let mut slot = None;
        let size = Dimensions::new(64, 64);
        let mut encoder = Encoder::new(
            "libx264",
            VideoFormat::H264,
            size,
            StreamConfig {
                software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
                ..Default::default()
            },
            None,
            0,
        )
        .unwrap();
        let mut decoder = Decoder::new(VideoFormat::H264, size, 1, false, 0).unwrap();
        let mut first_id = None;
        for (n, bgra) in [[31u8, 63, 127, 255], [137, 81, 43, 255]]
            .into_iter()
            .enumerate()
        {
            let pixels = bgra.repeat((size.width * size.height) as usize);
            let packet = encoder
                .submit(Some(&pixels), None, n as u64 * 16667, n == 0)
                .unwrap()
                .remove(0);
            let frame = decoder.decode(&packet.bytes).unwrap().unwrap();
            let id = VideoTexture::upload(&mut slot, &g, &mut renderer, &frame).unwrap();
            if let Some(first) = first_id {
                assert_eq!(id, first, "Video ID changed for every frame");
            } else {
                first_id = Some(id);
            }
            if n == 0 {
                assert!(renderer.needs_render(&rtv, &ctx, &output(id, 1.)).unwrap());
            }
            renderer
                .render(&g.context, &rtv, &ctx, output(id, 1.))
                .unwrap();
            assert!(
                !renderer.needs_render(&rtv, &ctx, &output(id, 1.)).unwrap(),
                "Unchanged geometry requested a repaint"
            );
            // Native live playback bypasses this static-window gate, because the
            // texture can change even though egui geometry and IDs are stable.

            let actual = pixel(&g, &texture);
            assert!(
                actual.iter().zip(bgra).all(|(a, b)| a.abs_diff(b) < 10),
                "Channel order or cached video froze: {actual:?}"
            );
        }
        assert_eq!(renderer.geometry_cache_stats(), (1, 1));
        let id = first_id.unwrap();
        assert!(
            renderer.needs_render(&rtv, &ctx, &output(id, 2.)).unwrap(),
            "DPI change was skipped"
        );
        let mut texture_delta = output(id, 1.);
        texture_delta
            .textures_delta
            .free
            .push(egui::TextureId::Managed(999999));
        assert!(
            renderer.needs_render(&rtv, &ctx, &texture_delta).unwrap(),
            "Texture delta was skipped"
        );

        renderer
            .render(&g.context, &rtv, &ctx, output(id, 2.))
            .unwrap();
        assert_eq!(
            renderer.geometry_cache_stats(),
            (1, 2),
            "DPI did not invalidate geometry"
        );
        let (_, resized) = target(&g, 128, 128);
        assert!(
            renderer
                .needs_render(&resized, &ctx, &output(id, 2.))
                .unwrap(),
            "Resized target was skipped"
        );

        renderer
            .render(&g.context, &resized, &ctx, output(id, 2.))
            .unwrap();
        assert_eq!(
            renderer.geometry_cache_stats(),
            (1, 3),
            "Target resize did not invalidate geometry"
        );
        let empty = egui_directx11::RendererOutput {
            textures_delta: Default::default(),
            shapes: Vec::new(),
            pixels_per_point: 2.,
        };
        renderer.render(&g.context, &resized, &ctx, empty).unwrap();
        renderer
            .render(&g.context, &resized, &ctx, output(id, 2.))
            .unwrap();
        assert_eq!(
            renderer.geometry_cache_stats(),
            (1, 4),
            "Empty UI retained stale geometry"
        );
        let (_, fresh_rtv) = target(&g, 128, 128);
        let mut uncached = egui_directx11::Renderer::new(&g.device).unwrap();
        // A new renderer has no user textures; register the same surface and verify
        // the optimized cache does not alter its registration/lifetime contract.
        let mut srv = None;
        unsafe {
            g.device
                .CreateShaderResourceView(&slot.as_ref().unwrap().texture, None, Some(&mut srv))
                .unwrap();
        }
        let new_id = uncached.register_user_texture(srv.unwrap());
        uncached
            .render(&g.context, &fresh_rtv, &ctx, output(new_id, 2.))
            .unwrap();
        assert!(renderer.unregister_user_texture(id));
        assert!(!renderer.unregister_user_texture(id));
    }
    #[test]
    #[ignore = "Requires D3D11 hardware decoding; synthetic frames and offscreen output only"]
    fn gpu_worker_to_display_handles_resize_and_retained_old_frames() {
        use p2pshare_codecs_windows::CodecWorker;
        let g = GraphicsDevice::new().unwrap();
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |_| {});
        let mut renderer = egui_directx11::Renderer::new(&g.device).unwrap();
        let worker = CodecWorker::with_display_device(Some(g.device.clone())).unwrap();
        let (texture, rtv) = target(&g, 64, 64);
        let mut slot = None;
        let mut held = None;
        let mut old_id = None;
        for (n, size) in [Dimensions::new(64, 64), Dimensions::new(128, 64)]
            .into_iter()
            .enumerate()
        {
            let bgra = if n == 0 {
                [31u8, 63, 127, 255]
            } else {
                [137, 81, 43, 255]
            };
            let pixels = bgra.repeat((size.width * size.height) as usize);
            let mut encoder = Encoder::new(
                "libx264",
                VideoFormat::H264,
                size,
                StreamConfig {
                    software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
                    ..Default::default()
                },
                None,
                0,
            )
            .unwrap();
            let packet = encoder
                .submit(Some(&pixels), None, 0, true)
                .unwrap()
                .remove(0);
            assert!(worker.decode_codec(
                "synthetic".into(),
                packet.bytes.into(),
                VideoFormat::H264,
                size
            ));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let frame = loop {
                if let Some(frame) = worker.take_decoded().remove("synthetic") {
                    break frame;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Worker failed: {:?}",
                    worker.take_error()
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            };
            assert!(
                frame.gpu.is_some() && frame.rgba.is_empty(),
                "Worker did not select GPU-resident output"
            );
            let id = VideoTexture::upload(&mut slot, &g, &mut renderer, &frame).unwrap();
            renderer
                .render(&g.context, &rtv, &ctx, output(id, 1.))
                .unwrap();
            let actual = pixel(&g, &texture);
            assert!(
                actual.iter().zip(bgra).all(|(a, b)| a.abs_diff(b) < 10),
                "GPU display/crop/resize colors changed: {actual:?}"
            );
            if let Some(old) = old_id {
                assert_ne!(old, id);
                assert!(
                    !renderer.unregister_user_texture(old),
                    "Old size leaked a registration"
                );
            }
            old_id = Some(id);
            // Keep the old lease alive across decoder replacement and verify it
            // remains readable after that decoder's output pool was destroyed.
            if let Some(old) = held.take() {
                let mut previous = None;
                VideoTexture::upload(&mut previous, &g, &mut renderer, &old).unwrap();
                let previous = previous.unwrap();
                renderer
                    .render(&g.context, &rtv, &ctx, output(previous.id, 1.))
                    .unwrap();
                let actual = pixel(&g, &texture);
                assert!(
                    actual[0].abs_diff(31) < 10
                        && actual[1].abs_diff(63) < 10
                        && actual[2].abs_diff(127) < 10,
                    "Retired decoder frame was overwritten"
                );
                renderer.unregister_user_texture(previous.id);
            }
            held = Some(frame);
        }
        worker.retain(Vec::new());
    }
}
