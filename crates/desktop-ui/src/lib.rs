#[cfg(windows)]
mod firewall;
#[cfg(windows)]
mod updater;
#[cfg(windows)]
pub use updater::command_mode as update_command_mode;
#[cfg(windows)]
pub use firewall::command_mode as firewall_command_mode;
#[cfg(windows)]
mod cpu_profile;
#[cfg(windows)]
mod local_preview;
#[cfg(windows)]
mod thumbnails;
#[cfg(windows)]
mod window_memory;
// Native desktop capture preview shell.
#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod publisher;
#[cfg(windows)]
mod test_source;
#[cfg(windows)]
mod wasapi;
#[cfg(windows)]
pub use native::run;
#[cfg(not(windows))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    Err("P2PShare capture preview requires Windows 10/11".into())
}
/// Report startup failures even in the release GUI subsystem without a console.
pub fn show_error(message: &str) {
    #[cfg(windows)]
    unsafe {
        use windows::{
            Win32::UI::WindowsAndMessaging::*,
            core::{PCWSTR, w},
        };
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("P2PShare"),
            MB_OK | MB_ICONERROR,
        );
    }
    #[cfg(not(windows))]
    eprintln!("P2PShare: {message}");
}
