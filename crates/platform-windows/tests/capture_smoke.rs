//! Explicit desktop tests: run with --ignored --nocapture --test-threads=1.
//! Test windows are temporary and never save captured pixels to disk.
#![cfg(windows)]
use p2pshare_media::{CaptureBackend, CaptureSource, CaptureState, Dimensions, SourceKind};
use p2pshare_platform_windows::{
    GpuFrame, GraphicsDevice, Preview, WindowsCapture, enumerate_sources,
};
use std::{
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{LibraryLoader::GetModuleHandleW, WinRT::*},
        UI::WindowsAndMessaging::*,
    },
    core::{Result, w},
};
struct Desktop {
    source: HWND,
    viewer: HWND,
    source_thread: Option<thread::JoinHandle<()>>,
}
impl Desktop {
    fn new() -> Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(test_window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("P2PShareCaptureTest"),
                hbrBackground: CreateSolidBrush(COLORREF(0x00C04020)),
                ..Default::default()
            };
            RegisterClassW(&class);
            // Keep the source on its own message-pump thread, like an actual external app.
            // Closing a WGC source from the capture thread itself can deadlock Windows teardown.
            let (sender, receiver) = std::sync::mpsc::channel();
            let source_thread = thread::spawn(move || {
                let instance = GetModuleHandleW(None).unwrap();
                let source = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("P2PShareCaptureTest"),
                    w!("P2PShare capture test source"),
                    WS_OVERLAPPEDWINDOW,
                    40,
                    40,
                    640,
                    480,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
                .unwrap();
                let _ = ShowWindow(source, SW_SHOWNOACTIVATE);
                sender.send(source.0 as usize).unwrap();
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            });
            let source = HWND(receiver.recv().unwrap() as *mut _);
            let viewer = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("P2PShareCaptureTest"),
                w!("P2PShare GPU test preview"),
                WS_OVERLAPPEDWINDOW,
                710,
                40,
                600,
                450,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let _ = ShowWindow(viewer, SW_SHOWNOACTIVATE);
            Ok(Self {
                source,
                viewer,
                source_thread: Some(source_thread),
            })
        }
    }
    fn pump(&self) {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.source)).as_bool() {
                let _ = PostMessageW(Some(self.source), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            if let Some(owner) = self.source_thread.take() {
                let _ = owner.join();
            }
            let _ = DestroyWindow(self.viewer);
            RoUninitialize();
        }
    }
}
fn next_frame(capture: &mut WindowsCapture, desktop: &Desktop) -> GpuFrame {
    let timeout = Instant::now();
    loop {
        desktop.pump();
        if let Some(frame) = capture.poll_frame().expect("poll capture") {
            return frame;
        }
        assert!(
            timeout.elapsed() < Duration::from_secs(8),
            "Source did not produce a frame within 8 seconds"
        );
        thread::sleep(Duration::from_millis(16));
    }
}
#[test]
#[ignore = "Requires interactive Windows desktop; opens two temporary test windows"]
fn real_capture_lifecycle() {
    let desktop = Desktop::new().expect("test desktop");
    let graphics = GraphicsDevice::new().expect("D3D11 device");
    let mut capture = WindowsCapture::new(graphics.clone());
    let mut preview = Preview::new(graphics.clone(), desktop.viewer, Dimensions::new(600, 450))
        .expect("GPU preview");
    let sources = enumerate_sources(None).expect("source enumeration");
    let source = sources
        .iter()
        .find(|s| s.kind == SourceKind::Window && s.id == desktop.source.0 as u64)
        .expect("test window enumerated")
        .clone();
    capture.start(&source).expect("window capture start");
    let first = next_frame(&mut capture, &desktop);
    assert_captured_pixels(&graphics, &first);
    let mut previous = first.metadata;
    assert!(!previous.size.is_empty());
    println!(
        "Window first frame: {} x {}",
        previous.size.width, previous.size.height
    );
    for (width, height) in [(900, 620), (420, 700), (1000, 540), (500, 400)] {
        unsafe {
            SetWindowPos(
                desktop.source,
                None,
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .expect("resize source");
        }
        let started = Instant::now();
        loop {
            let frame = next_frame(&mut capture, &desktop);
            preview.present(&frame).expect("render resized frame");
            if frame.metadata.size != previous.size {
                assert!(
                    frame.metadata.timestamp >= previous.timestamp,
                    "capture clock regressed"
                );
                assert!(frame.metadata.sequence > previous.sequence);
                println!(
                    "Resize: {} x {}",
                    frame.metadata.size.width, frame.metadata.size.height
                );
                previous = frame.metadata;
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(8),
                "resize was not observed"
            );
        }
    }
    unsafe {
        let _ = ShowWindow(desktop.source, SW_MINIMIZE);
    }
    desktop.pump();
    assert_eq!(capture.state(), CaptureState::Paused);
    assert!(capture.poll_frame().unwrap().is_none());
    unsafe {
        let _ = ShowWindow(desktop.source, SW_SHOWNOACTIVATE);
    }
    desktop.pump();
    assert_eq!(capture.state(), CaptureState::Capturing);
    preview
        .present(&next_frame(&mut capture, &desktop))
        .unwrap();
    println!("Minimize/restore passed");
    for _ in 0..3 {
        capture.stop().unwrap();
        assert_eq!(capture.state(), CaptureState::Stopped);
        preview.clear().unwrap();
        capture.start(&source).unwrap();
        preview
            .present(&next_frame(&mut capture, &desktop))
            .unwrap();
    }
    println!("Repeated stop/start passed");
    for monitor in sources.iter().filter(|s| s.kind == SourceKind::Monitor) {
        capture.start(monitor).expect("monitor capture start");
        let frame = next_frame(&mut capture, &desktop);
        preview.present(&frame).expect("monitor preview");
        println!(
            "Monitor passed: {} x {}",
            frame.metadata.size.width, frame.metadata.size.height
        );
    }
    println!("Starting source-closure test");
    capture.start(&source).unwrap();
    println!("Closing source window");
    unsafe {
        PostMessageW(Some(desktop.source), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
    }
    println!("Source close posted to owner thread");
    let started = Instant::now();
    while capture.state() != CaptureState::SourceClosed {
        desktop.pump();
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(16));
    }
    capture.stop().unwrap();
    println!("Source closure passed; captured frames remained GPU-backed");
}
#[test]
#[ignore = "30-minute interactive desktop stability test; P2PSHARE_SOAK_SECONDS overrides duration"]
fn capture_soak() {
    let seconds = std::env::var("P2PSHARE_SOAK_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1800);
    let desktop = Desktop::new().expect("test desktop");
    let graphics = GraphicsDevice::new().unwrap();
    let mut preview =
        Preview::new(graphics.clone(), desktop.viewer, Dimensions::new(600, 450)).unwrap();
    let mut capture = WindowsCapture::new(graphics);
    let source = CaptureSource {
        id: desktop.source.0 as u64,
        kind: SourceKind::Window,
        title: "Soak source".into(),
        size: Dimensions::new(640, 480),
        process_id: None,
    };
    capture.start(&source).unwrap();
    let start = Instant::now();
    let mut frames = 0u64;
    let mut report = 0;
    let mut size_phase = 0;
    let mut latest = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        desktop.pump();
        let phase = start.elapsed().as_secs() / 5;
        if phase != size_phase {
            size_phase = phase;
            let (width, height) = if phase.is_multiple_of(2) {
                (640, 480)
            } else {
                (800, 600)
            };
            unsafe {
                SetWindowPos(
                    desktop.source,
                    None,
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .unwrap();
            }
        }
        if let Some(frame) = capture.poll_frame().unwrap() {
            preview.present(&frame).unwrap();
            frames += 1;
            latest = Instant::now();
        }
        assert!(
            latest.elapsed() < Duration::from_secs(15),
            "Capture stalled during soak"
        );
        let minute = start.elapsed().as_secs() / 60;
        if minute > report {
            report = minute;
            println!("Soak: {minute} minutes, {frames} frames, live resize every 5 seconds");
        }
        thread::sleep(Duration::from_millis(32));
    }
    capture.stop().unwrap();
    assert!(frames > 0);
    println!("Soak passed: {seconds} seconds, {frames} frames");
}
unsafe extern "system" fn test_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_DESTROY => {
                PostQuitMessage(0);
            }
            WM_TIMER => {
                let tick = GetWindowLongPtrW(hwnd, GWLP_USERDATA) + 1;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, tick);
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }
            WM_PAINT if GetWindowLongPtrW(hwnd, GWLP_USERDATA) > 0 => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let brush = HBRUSH(GetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND) as *mut _);
                FillRect(dc, &rect, brush);
                let tick = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as i32;
                let x = 10 + tick % (rect.right / 4).max(1);
                FillRect(
                    dc,
                    &RECT {
                        left: x,
                        top: 0,
                        right: x + 16,
                        bottom: rect.bottom,
                    },
                    HBRUSH(GetStockObject(WHITE_BRUSH).0),
                );
                let _ = EndPaint(hwnd, &paint);
                return LRESULT(0);
            }
            _ => (),
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

#[test]
#[ignore = "Five-second animated-window capture check; requires an interactive desktop"]
fn capture_motion() {
    let desktop = Desktop::new().unwrap();
    let graphics = GraphicsDevice::new().unwrap();
    let mut capture = WindowsCapture::new(graphics.clone());
    let mut preview = Preview::new(graphics, desktop.viewer, Dimensions::new(600, 450)).unwrap();
    let source = enumerate_sources(None)
        .unwrap()
        .into_iter()
        .find(|s| s.id == desktop.source.0 as u64 && s.kind == SourceKind::Window)
        .unwrap();
    capture.start(&source).unwrap();
    unsafe {
        SetTimer(Some(desktop.source), 99, 16, None);
    }
    let start = Instant::now();
    let mut count = 0;
    let mut previous = Duration::ZERO;
    while start.elapsed() < Duration::from_secs(5) {
        desktop.pump();
        if let Some(frame) = capture.poll_frame().unwrap() {
            assert!(frame.metadata.timestamp >= previous);
            previous = frame.metadata.timestamp;
            preview.present(&frame).unwrap();
            count += 1;
        }
        thread::sleep(Duration::from_millis(8));
    }
    unsafe {
        let _ = KillTimer(Some(desktop.source), 99);
    }
    capture.stop().unwrap();
    assert!(
        count >= 30,
        "Animated source delivered only {count} frames in five seconds"
    );
    println!(
        "Animated source passed: {count} frames in {:.2}s",
        start.elapsed().as_secs_f64()
    );
}
// Validation-only readback proves WGC and the owned GPU copy contain actual source pixels.
// The application preview has no readback path.
fn assert_captured_pixels(graphics: &GraphicsDevice, frame: &GpuFrame) {
    use windows::Win32::Graphics::Direct3D11::*;
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        frame.texture.GetDesc(&mut desc);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut texture = None;
        graphics
            .device
            .CreateTexture2D(&desc, None, Some(&mut texture))
            .unwrap();
        let texture = texture.unwrap();
        graphics.context.CopyResource(&texture, &frame.texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        graphics
            .context
            .Map(&texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .unwrap();
        let offset = (desc.Height / 2 * mapped.RowPitch + desc.Width / 2 * 4) as usize;
        let pixel = std::slice::from_raw_parts((mapped.pData as *const u8).add(offset), 4);
        let value = [pixel[0], pixel[1], pixel[2]];
        graphics.context.Unmap(&texture, 0);
        assert_eq!(
            value,
            [192, 64, 32],
            "Captured center should match the test window's blue background"
        );
        println!("GPU capture pixels verified: BGRA {value:?}");
    }
}
