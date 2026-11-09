//! GPU-resident BGRA -> NV12 -> FFmpeg NVENC/AMF.
use crate::{Encoded, ffmpeg::Encoder};
use anyhow::{Context, Result};
use p2pshare_media::{Dimensions, config::StreamConfig, diagnostics::LatencyMeter};
use p2pshare_platform_windows::GraphicsDevice;
use std::mem::ManuallyDrop;
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{Direct3D11::*, Dxgi::Common::*},
    },
    core::Interface,
};
pub struct GpuEncoder {
    encoder: Encoder,
    graphics: GraphicsDevice,
    video: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    target: ID3D11Texture2D,
    output: ID3D11VideoProcessorOutputView,
    size: Dimensions,
    settings: StreamConfig,
    adapter: u32,
    latency: LatencyMeter,
}
impl GpuEncoder {
    pub fn new(
        graphics: GraphicsDevice,
        source: Dimensions,
        settings: StreamConfig,
    ) -> Result<Self> {
        let candidates = crate::capabilities::candidates(settings, Some(&graphics.device), true)?;
        let preferred = crate::capabilities::candidates(settings, Some(&graphics.device), false)?;
        if settings.codec == p2pshare_media::config::Codec::Auto
            && preferred.first().is_some_and(|p| {
                p.codec == p2pshare_media::config::VideoFormat::Hevc && !p.name.starts_with("lib")
            })
            && candidates
                .first()
                .is_none_or(|p| p.codec != p2pshare_media::config::VideoFormat::Hevc)
        {
            anyhow::bail!("Hardware HEVC is on another adapter; use the readback/upload path");
        }
        let mut failures = Vec::new();
        for candidate in candidates {
            if let Err(error) = crate::profiles::gpu(
                &candidate.name,
                candidate.codec,
                source,
                settings,
                candidate.adapter,
                &graphics,
            ) {
                tracing::warn!(backend=%candidate.name,adapter=candidate.adapter,%error,"gpu_stream_profile_rejected");
                failures.push(error.to_string());
                continue;
            }
            match Self::new_backend(
                graphics.clone(),
                source,
                settings,
                &candidate.name,
                candidate.codec,
                candidate.adapter,
            ) {
                Ok(encoder) => return Ok(encoder),
                Err(error) => {
                    tracing::warn!(backend=candidate.name, %error, "gpu_encoder_candidate_failed");
                    failures.push(error.to_string());
                }
            }
        }
        anyhow::bail!("No usable GPU encoder: {}", failures.join("; "))
    }
    pub(crate) fn output_size(source: Dimensions, settings: StreamConfig) -> Dimensions {
        let bounds = settings.bounds();
        let ratio = (bounds.width.max(bounds.height) as f64
            / source.width.max(source.height).max(1) as f64)
            .min(
                bounds.width.min(bounds.height) as f64
                    / source.width.min(source.height).max(1) as f64,
            )
            .min(1.);
        Dimensions::new(
            ((source.width as f64 * ratio) as u32 & !1).max(2),
            ((source.height as f64 * ratio) as u32 & !1).max(2),
        )
    }
    pub(crate) fn new_backend(
        graphics: GraphicsDevice,
        source: Dimensions,
        settings: StreamConfig,
        name: &str,
        format: p2pshare_media::config::VideoFormat,
        adapter: u32,
    ) -> Result<Self> {
        let size = Self::output_size(source, settings);
        unsafe {
            let video: ID3D11VideoDevice = graphics.device.cast()?;
            let context: ID3D11VideoContext = graphics.context.cast()?;
            let enumerator =
                video.CreateVideoProcessorEnumerator(&D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                    InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                    InputFrameRate: DXGI_RATIONAL {
                        Numerator: settings.fps,
                        Denominator: 1,
                    },
                    InputWidth: source.width,
                    InputHeight: source.height,
                    OutputFrameRate: DXGI_RATIONAL {
                        Numerator: settings.fps,
                        Denominator: 1,
                    },
                    OutputWidth: size.width,
                    OutputHeight: size.height,
                    Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                })?;
            let processor = video.CreateVideoProcessor(&enumerator, 0)?;
            let mut target = None;
            graphics.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size.width,
                    Height: size.height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_NV12,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut target),
            )?;
            let target = target.unwrap();
            let mut output = None;
            video.CreateVideoProcessorOutputView(
                &target,
                &enumerator,
                &D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                    ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                    ..Default::default()
                },
                Some(&mut output),
            )?;
            let input_rect = RECT {
                left: 0,
                top: 0,
                right: source.width as i32,
                bottom: source.height as i32,
            };
            let output_rect = RECT {
                left: 0,
                top: 0,
                right: size.width as i32,
                bottom: size.height as i32,
            };
            context.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            context.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&input_rect));
            context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&output_rect));
            context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&output_rect));
            context.VideoProcessorSetStreamColorSpace(
                &processor,
                0,
                &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 },
            );
            // BT.601 matrix and studio-range NV12 match the existing AVC software decoder path.
            context.VideoProcessorSetOutputColorSpace(
                &processor,
                &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 1 << 4 },
            );
            let encoder = Encoder::new(
                name,
                format,
                size,
                settings,
                Some(&graphics.device),
                adapter,
            )?;
            tracing::info!(
                width = size.width,
                height = size.height,
                bitrate = settings.bitrate_bps(),
                codec = format.label(),
                "gpu_resident_encoder_ready"
            );
            Ok(Self {
                encoder,
                settings,
                adapter,
                graphics,
                video,
                context,
                enumerator,
                processor,
                target,
                output: output.unwrap(),
                size,
                latency: LatencyMeter::new("gpu_convert_encode"),
            })
        }
    }
    pub fn backend(&self) -> String {
        format!("FFmpeg {} (D3D11 NV12)", self.encoder.name)
    }
    pub fn size(&self) -> Dimensions {
        self.size
    }
    pub fn runtime_failure(&self, error: &anyhow::Error) {
        crate::profiles::failed(
            &self.encoder.name,
            self.encoder.format,
            self.size,
            self.settings,
            self.adapter,
            true,
            error,
        );
    }
    pub fn encode(
        &mut self,
        input: &ID3D11Texture2D,
        micros: u64,
        keyframe: bool,
    ) -> Result<Vec<Encoded>> {
        let started = std::time::Instant::now();
        unsafe {
            let mut view = None;
            self.video
                .CreateVideoProcessorInputView(
                    input,
                    &self.enumerator,
                    &D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                        ..Default::default()
                    },
                    Some(&mut view),
                )
                .context("Create GPU processor input view")?;
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(view),
                ..Default::default()
            };
            let result = self.context.VideoProcessorBlt(
                &self.processor,
                &self.output,
                0,
                std::slice::from_ref(&stream),
            );
            drop(ManuallyDrop::take(&mut stream.pInputSurface));
            result.context("BGRA to NV12 processor blit")?;
            self.graphics.context.Flush();
        }
        let frames = self
            .encoder
            .submit(None, Some(&self.target), micros, keyframe)
            .context("Encode GPU texture")?;
        self.latency.record(started.elapsed().as_secs_f64() * 1000.);
        Ok(frames)
    }
}
