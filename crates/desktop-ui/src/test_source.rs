//! Temporary synthetic source for opt-in GPU UI integration tests.
use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::{Result, w},
};
pub struct TestSource {
    pub hwnd: HWND,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl TestSource {
    pub fn new() -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || unsafe {
            let result = (|| -> Result<HWND> {
                let instance = GetModuleHandleW(None)?;
                let class = WNDCLASSW {
                    lpfnWndProc: Some(proc),
                    hInstance: instance.into(),
                    lpszClassName: w!("P2PShareEguiTestSource"),
                    ..Default::default()
                };
                RegisterClassW(&class);
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("P2PShareEguiTestSource"),
                    w!("P2PShare egui test source"),
                    WS_OVERLAPPEDWINDOW,
                    20,
                    20,
                    640,
                    400,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )?;
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                // A deterministic changing source supplies frames even if capture starts before room setup.
                SetTimer(Some(hwnd), 1, 100, None);
                Ok(hwnd)
            })();
            match result {
                Ok(hwnd) => {
                    let _ = tx.send(Ok(hwnd.0 as usize));
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                }
            }
        });
        let hwnd = rx.recv().map_err(|e| {
            windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), e.to_string())
        })??;
        Ok(Self {
            hwnd: HWND(hwnd as *mut _),
            thread: Some(thread),
        })
    }
    pub fn prepare_hd(&self) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                20,
                20,
                1920,
                1080,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SetTimer(Some(self.hwnd), 1, 10, None);
        }
    }
    pub fn resize(&self) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                20,
                20,
                920,
                560,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}
impl Drop for TestSource {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let brush = CreateSolidBrush(COLORREF(0x009F7C26));
                FillRect(dc, &ps.rcPaint, brush);
                let _ = DeleteObject(brush.into());
                // Actual changing content for capture cadence tests; a flat repaint
                // can be coalesced by the compositor even when WM_TIMER fires.
                let tick = GetWindowLongPtrW(hwnd, GWLP_USERDATA).wrapping_add(1);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, tick);
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let x = (tick as i32 * 19).rem_euclid((client.right - 120).max(1));
                let marker = RECT {
                    left: x,
                    top: 40,
                    right: x + 100,
                    bottom: 140,
                };
                let white = CreateSolidBrush(COLORREF(0x00ffffff));
                FillRect(dc, &marker, white);
                let _ = DeleteObject(white.into());
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
