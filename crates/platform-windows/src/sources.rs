use p2pshare_media::{CaptureSource, Dimensions, SourceKind};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::Threading::*,
        UI::WindowsAndMessaging::*,
    },
    core::{BOOL, PWSTR, Result},
};

/// Resolves the executable filename (e.g. `League of Legends.exe`) for a process ID.
pub fn process_executable_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: `OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION` is read-only and
    // the returned handle is always closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok || len == 0 {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        let name = full
            .rsplit(['\\', '/'])
            .next()
            .filter(|s| !s.is_empty())?;
        Some(name.to_owned())
    }
}

/// Returns true when the given window handle is currently minimized (`IsIconic`).
pub fn is_window_minimized(id: u64) -> bool {
    let hwnd = HWND(id as *mut _);
    // SAFETY: Win32 validates the HWND before querying its iconic state.
    unsafe { IsWindow(Some(hwnd)).as_bool() && IsIconic(hwnd).as_bool() }
}

/// Applies the configured Windows scheduling priority class to the current P2PShare process.
pub fn apply_process_priority(priority: p2pshare_media::config::ProcessPriority) -> Result<()> {
    let class = match priority {
        p2pshare_media::config::ProcessPriority::Normal => NORMAL_PRIORITY_CLASS,
        p2pshare_media::config::ProcessPriority::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
    };
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle valid for the lifetime of this process.
    unsafe { SetPriorityClass(GetCurrentProcess(), class) }
}
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
const DWMWA_CLOAKED: u32 = 14;

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmGetWindowAttribute(
        hwnd: *mut std::ffi::c_void,
        dw_attribute: u32,
        pv_attribute: *mut std::ffi::c_void,
        cb_attribute: u32,
    ) -> i32;
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
        let mut cloaked: u32 = 0;
        let hr = DwmGetWindowAttribute(
            hwnd.0,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
        if hr >= 0 && cloaked != 0 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_executable_name_resolves_current_process() {
        let name = process_executable_name(std::process::id()).expect("current process exe name");
        assert!(name.to_ascii_lowercase().ends_with(".exe"), "unexpected exe name: {name}");
        assert!(!name.contains('\\') && !name.contains('/'));
        assert_eq!(process_executable_name(0), None);
    }

    #[test]
    fn apply_process_priority_updates_current_process_class() {
        apply_process_priority(p2pshare_media::config::ProcessPriority::AboveNormal).unwrap();
        let class = unsafe { GetPriorityClass(GetCurrentProcess()) };
        assert_eq!(class, ABOVE_NORMAL_PRIORITY_CLASS.0);
        apply_process_priority(p2pshare_media::config::ProcessPriority::Normal).unwrap();
        let restored = unsafe { GetPriorityClass(GetCurrentProcess()) };
        assert_eq!(restored, NORMAL_PRIORITY_CLASS.0);
    }
}
