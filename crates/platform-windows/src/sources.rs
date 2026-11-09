use p2pshare_media::{CaptureSource, Dimensions, SourceKind};
use windows::{
    Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*},
    core::{BOOL, Result},
};
/// Enumerates connected monitors and visible, titled top-level application windows.
/// Handles are opaque IDs in the portable layer and never cross a network boundary.
pub fn enumerate_sources(exclude: Option<HWND>) -> Result<Vec<CaptureSource>> {
    let mut sources = Vec::new();
    // SAFETY: callbacks are synchronous; the Vec pointer stays valid throughout enumeration.
    unsafe {
        if !EnumDisplayMonitors(
            None,
            None,
            Some(monitor_callback),
            LPARAM(&mut sources as *mut _ as isize),
        )
        .as_bool()
        {
            return Err(windows::core::Error::from_thread());
        }
        let mut context = WindowEnumeration {
            sources: &mut sources,
            exclude,
        };
        EnumWindows(
            Some(window_callback),
            LPARAM(&mut context as *mut _ as isize),
        )?;
    }
    Ok(sources)
}
struct WindowEnumeration<'a> {
    sources: &'a mut Vec<CaptureSource>,
    exclude: Option<HWND>,
}
unsafe extern "system" fn monitor_callback(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: data was provided by enumerate_sources for this synchronous callback.
    unsafe {
        let sources = &mut *(data.0 as *mut Vec<CaptureSource>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
            let rect = info.monitorInfo.rcMonitor;
            let name = String::from_utf16_lossy(
                &info.szDevice[..info
                    .szDevice
                    .iter()
                    .position(|&v| v == 0)
                    .unwrap_or(info.szDevice.len())],
            );
            let primary = if info.monitorInfo.dwFlags & 1 != 0 {
                " (primary)"
            } else {
                ""
            };
            sources.push(CaptureSource {
                id: monitor.0 as u64,
                kind: SourceKind::Monitor,
                title: format!("Monitor {name}{primary}"),
                size: Dimensions::new(
                    (rect.right - rect.left) as u32,
                    (rect.bottom - rect.top) as u32,
                ),
                process_id: None,
            });
        }
        TRUE
    }
}
unsafe extern "system" fn window_callback(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: Win32 validates window handles; callback data is live during EnumWindows.
    unsafe {
        let context = &mut *(data.0 as *mut WindowEnumeration<'_>);
        if context.exclude == Some(hwnd)
            || !IsWindowVisible(hwnd).as_bool()
            || GetWindowTextLengthW(hwnd) == 0
        {
            return TRUE;
        }
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return TRUE;
        }
        let mut text = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut text);
        let mut rect = RECT::default();
        if len == 0
            || GetWindowRect(hwnd, &mut rect).is_err()
            || rect.right <= rect.left
            || rect.bottom <= rect.top
        {
            return TRUE;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        context.sources.push(CaptureSource {
            id: hwnd.0 as u64,
            kind: SourceKind::Window,
            title: String::from_utf16_lossy(&text[..len as usize]),
            size: Dimensions::new(
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
            ),
            process_id: Some(pid),
        });
        TRUE
    }
}
