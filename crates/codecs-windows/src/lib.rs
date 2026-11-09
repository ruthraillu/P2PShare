//! Bounded FFmpeg AVC/HEVC workers and GPU conversion for portable Windows.
#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::*;

#[cfg(windows)]
mod gpu;
#[cfg(windows)]
pub use gpu::GpuEncoder;
#[cfg(windows)]
pub mod capabilities;
#[cfg(windows)]
pub mod ffmpeg;

#[cfg(windows)]
mod adapter_policy;

#[cfg(windows)]
mod profiles;
