//! Portable capture/codec contracts and geometry. Platform handles stay in the backend.
pub mod audio;
pub mod avatar;
pub mod i18n;
pub mod portable;
pub mod profile_history;
pub mod playback;
use p2pshare_protocol::VideoCodec;
use std::time::Duration;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}
impl Dimensions {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
    /// Fits inside a bounding box without stretching, using overflow-safe integer arithmetic.
    pub fn fit(self, bounds: Self) -> Self {
        if self.is_empty() || bounds.is_empty() {
            return Self::default();
        }
        if u64::from(self.width) * u64::from(bounds.height)
            > u64::from(bounds.width) * u64::from(self.height)
        {
            Self::new(
                bounds.width,
                ((u64::from(self.height) * u64::from(bounds.width)) / u64::from(self.width)).max(1)
                    as u32,
            )
        } else {
            Self::new(
                ((u64::from(self.width) * u64::from(bounds.height)) / u64::from(self.height)).max(1)
                    as u32,
                bounds.height,
            )
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub size: Dimensions,
}
pub fn letterbox(source: Dimensions, target: Dimensions) -> Viewport {
    let size = source.fit(target);
    Viewport {
        x: target.width.saturating_sub(size.width) / 2,
        y: target.height.saturating_sub(size.height) / 2,
        size,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Monitor,
    Window,
}
#[derive(Clone, Debug)]
pub struct CaptureSource {
    pub id: u64,
    pub kind: SourceKind,
    pub title: String,
    pub size: Dimensions,
    pub process_id: Option<u32>,
}
#[derive(Clone, Copy, Debug)]
pub struct FrameMetadata {
    pub size: Dimensions,
    pub timestamp: Duration,
    pub sequence: u64,
    pub resized: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureState {
    Stopped,
    Capturing,
    Paused,
    SourceClosed,
}
pub trait CaptureBackend {
    type Frame;
    type Error: std::error::Error;
    fn sources(&self) -> Result<Vec<CaptureSource>, Self::Error>;
    fn start(&mut self, source: &CaptureSource) -> Result<(), Self::Error>;
    /// Non-blocking; returns at most the latest available frame. Never accumulate stale frames.
    fn poll_frame(&mut self) -> Result<Option<Self::Frame>, Self::Error>;
    fn stop(&mut self) -> Result<(), Self::Error>;
}
#[derive(Clone, Debug)]
pub struct EncodedFrame {
    pub codec: VideoCodec,
    pub timestamp: Duration,
    pub keyframe: bool,
    pub size: Dimensions,
    pub data: Vec<u8>,
}
#[derive(Clone, Copy, Debug)]
pub struct EncoderSettings {
    pub codec: VideoCodec,
    pub size: Dimensions,
    pub fps: u32,
    pub bitrate_bps: u32,
}
pub trait VideoEncoder {
    type Input;
    type Error: std::error::Error;
    fn configure(&mut self, settings: EncoderSettings) -> Result<(), Self::Error>;
    fn encode(&mut self, frame: &Self::Input) -> Result<Option<EncodedFrame>, Self::Error>;
    fn request_keyframe(&mut self);
}
pub trait VideoDecoder {
    type Output;
    type Error: std::error::Error;
    fn decode(&mut self, frame: &EncodedFrame) -> Result<Option<Self::Output>, Self::Error>;
}
/// A capacity-one handoff used by future threaded pipelines.
#[derive(Debug)]
pub struct LatestFrame<T>(Option<T>);
impl<T> Default for LatestFrame<T> {
    fn default() -> Self {
        Self(None)
    }
}
impl<T> LatestFrame<T> {
    pub fn replace(&mut self, frame: T) {
        self.0 = Some(frame);
    }
    pub fn take(&mut self) -> Option<T> {
        self.0.take()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ultrawide_is_not_stretched() {
        assert_eq!(
            Dimensions::new(3440, 1440).fit(Dimensions::new(1920, 1080)),
            Dimensions::new(1920, 803)
        );
    }
    #[test]
    fn portrait_and_resize_are_centered() {
        let v = letterbox(Dimensions::new(900, 1280), Dimensions::new(1920, 1080));
        assert_eq!(v.size, Dimensions::new(759, 1080));
        assert_eq!(v.x, 580);
        assert_eq!(v.y, 0);
        assert_eq!(
            Dimensions::new(1280, 900).fit(Dimensions::new(1920, 1080)),
            Dimensions::new(1536, 1080)
        );
    }
    #[test]
    fn empty_and_extreme_dimensions_are_safe() {
        assert_eq!(
            Dimensions::new(0, 900).fit(Dimensions::new(1920, 1080)),
            Dimensions::default()
        );
        assert_eq!(
            Dimensions::new(u32::MAX, u32::MAX).fit(Dimensions::new(1920, 1080)),
            Dimensions::new(1080, 1080)
        );
        assert_eq!(
            Dimensions::new(u32::MAX, 1).fit(Dimensions::new(1, 1)),
            Dimensions::new(1, 1)
        );
    }
    #[test]
    fn fit_never_exceeds_bounds() {
        for source in [
            Dimensions::new(1920, 1080),
            Dimensions::new(1280, 900),
            Dimensions::new(3840, 2160),
            Dimensions::new(1, 10000),
        ] {
            for bounds in [
                Dimensions::new(320, 200),
                Dimensions::new(1, 1),
                Dimensions::new(1600, 900),
            ] {
                let fit = source.fit(bounds);
                assert!(fit.width <= bounds.width && fit.height <= bounds.height);
                let error = (i64::from(fit.width) * i64::from(source.height)
                    - i64::from(fit.height) * i64::from(source.width))
                .abs();
                assert!(error <= i64::from(source.width.max(source.height)));
            }
        }
    }
    #[test]
    fn stale_frames_are_replaced() {
        let mut queue = LatestFrame::default();
        queue.replace(1);
        queue.replace(2);
        assert_eq!(queue.take(), Some(2));
        assert_eq!(queue.take(), None);
    }
}

/// Shared capture presets. Decoder/display code does not depend on the chosen preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum VideoQuality {
    Balanced = 0,
    #[default]
    High = 1,
    Best = 2,
}
impl VideoQuality {
    pub const ALL: [Self; 3] = [Self::Balanced, Self::High, Self::Best];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced • 720p / 4 Mbps",
            Self::High => "High • 1080p / 8 Mbps",
            Self::Best => "Best • 1080p / 16 Mbps",
        }
    }
    pub const fn bitrate(self) -> u32 {
        match self {
            Self::Balanced => 4_000_000,
            Self::High => 8_000_000,
            Self::Best => 16_000_000,
        }
    }
    pub const fn bounds(self) -> Dimensions {
        match self {
            Self::Balanced => Dimensions::new(1280, 720),
            _ => Dimensions::new(1920, 1080),
        }
    }
}

pub mod config;
pub mod diagnostics;

pub mod theme;

pub mod preview;
