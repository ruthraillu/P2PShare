//! Windows-only capture implementation; unsafe Win32/D3D calls stay in this crate.
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod sources;
#[cfg(windows)]
pub use capture::*;
#[cfg(windows)]
pub use preview::*;
#[cfg(windows)]
pub use sources::*;
