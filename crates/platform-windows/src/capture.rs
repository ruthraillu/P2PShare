use p2pshare_media::{
    CaptureBackend, CaptureSource, CaptureState, Dimensions, FrameMetadata, SourceKind,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Foundation::{E_ACCESSDENIED, HMODULE, HWND, RECT},
        Graphics::{
            Direct3D::*,
            Direct3D11::*,
            Dxgi::{Common::*, IDXGIDevice},
            Gdi::{GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow},
        },
        System::WinRT::{Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop},
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, IsIconic, IsWindow},
    },
    core::{Interface, Result},
};

const POOL_BUFFERS: i32 = 3;
const RESTORE_SETTLE_DELAY: Duration = Duration::from_millis(200);
const PENDING_PROBE_INTERVAL: Duration = Duration::from_millis(200);
const BLACK_PROBE_INTERVAL: Duration = Duration::from_millis(250);
const NO_FRAME_HINT_DELAY: Duration = Duration::from_secs(3);

/// 4x4 grid of normalized percentage coordinates used to detect all-black DWM window surfaces
/// (which still show a mouse cursor over at most one point) during fullscreen capture.
const BLACK_PROBE_SAMPLES: [(u32, u32); 16] = [
    (8, 8),
    (36, 8),
    (64, 8),
    (92, 8),
    (8, 36),
    (36, 36),
    (64, 36),
    (92, 36),
    (8, 64),
    (36, 64),
    (64, 64),
    (92, 64),
    (8, 92),
    (36, 92),
    (64, 92),
    (92, 92),
];

fn count_non_black_bgra_samples(pixels: &[[u8; 4]]) -> usize {
    pixels
        .iter()
        .filter(|&&[b, g, r, _]| b > 4 || g > 4 || r > 4)
        .count()
}

fn probe_non_black_samples(
    graphics: &GraphicsDevice,
    staging: &mut Option<ID3D11Texture2D>,
    texture: &ID3D11Texture2D,
    size: Dimensions,
) -> Option<usize> {
    if !usable_capture_size(size) {
        return None;
    }
    // SAFETY: D3D11 resources belong to `graphics.device` and are accessed on the capture thread.
    unsafe {
        if staging.is_none() {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: BLACK_PROBE_SAMPLES.len() as u32,
                Height: 1,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut created = None;
            graphics
                .device
                .CreateTexture2D(&desc, None, Some(&mut created))
                .ok()?;
            *staging = created;
        }
        let staging_tex = staging.as_ref()?;
        for (idx, &(px, py)) in BLACK_PROBE_SAMPLES.iter().enumerate() {
            let x = ((u64::from(size.width) * u64::from(px)) / 100)
                .min(u64::from(size.width - 1)) as u32;
            let y = ((u64::from(size.height) * u64::from(py)) / 100)
                .min(u64::from(size.height - 1)) as u32;
            graphics.context.CopySubresourceRegion(
                staging_tex,
                0,
                idx as u32,
                0,
                0,
                texture,
                0,
                Some(&D3D11_BOX {
                    left: x,
                    top: y,
                    front: 0,
                    right: x + 1,
                    bottom: y + 1,
                    back: 1,
                }),
            );
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        graphics
            .context
            .Map(staging_tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .ok()?;
        let bytes = std::slice::from_raw_parts(
            mapped.pData as *const u8,
            BLACK_PROBE_SAMPLES.len() * 4,
        );
        let mut pixels = [[0u8; 4]; BLACK_PROBE_SAMPLES.len()];
        for (slot, chunk) in pixels.iter_mut().zip(bytes.chunks_exact(4)) {
            slot.copy_from_slice(chunk);
        }
        graphics.context.Unmap(staging_tex, 0);
        Some(count_non_black_bgra_samples(&pixels))
    }
}

/// Convert an optional update interval into WinRT `TimeSpan` 100 ns ticks.
pub fn min_update_ticks(interval: Option<Duration>) -> Option<i64> {
    interval.map(|d| (d.as_nanos() / 100).min(i64::MAX as u128) as i64)
}

fn apply_min_update_interval(capture: &GraphicsCaptureSession, interval: Option<Duration>) {
    if let Some(ticks) = min_update_ticks(interval)
        && let Err(error) = capture.SetMinUpdateInterval(TimeSpan { Duration: ticks })
    {
        tracing::debug!(%error, ticks, "capture_min_update_interval_unavailable");
    }
}

fn rect_covers_monitor(window: RECT, monitor: RECT) -> bool {
    const TOLERANCE: i32 = 2;
    let mon_w = monitor.right - monitor.left;
    let mon_h = monitor.bottom - monitor.top;
    if mon_w < 64 || mon_h < 64 {
        return false;
    }
    window.left <= monitor.left + TOLERANCE
        && window.top <= monitor.top + TOLERANCE
        && window.right >= monitor.right - TOLERANCE
        && window.bottom >= monitor.bottom - TOLERANCE
}

/// Returns the monitor covered by `hwnd` strictly when `hwnd` is alive, not minimized,
/// the active foreground window, and covers the entire monitor (fullscreen).
fn foreground_fullscreen_monitor(hwnd: HWND) -> Option<HMONITOR> {
    // SAFETY: Win32 validates the HWND; MONITORINFO.cbSize is initialized before query.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        if GetForegroundWindow() != hwnd {
            return None;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        if monitor.0.is_null() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        rect_covers_monitor(rect, info.rcMonitor).then_some(monitor)
    }
}

fn window_usable_for_capture(hwnd: HWND) -> bool {
    // SAFETY: Win32 validates the HWND; non-window / minimized / sub-64px bounds return false.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;
        usable_capture_size(Dimensions::new(width, height))
    }
}

#[derive(Clone)]
pub struct GraphicsDevice {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub winrt: IDirect3DDevice,
    pub hardware: bool,
}
impl GraphicsDevice {
    pub fn from_device(device: ID3D11Device) -> Result<Self> {
        unsafe {
            let context = device.GetImmediateContext()?;
            let multithread: ID3D11Multithread = context.cast()?;
            let _ = multithread.SetMultithreadProtected(true);
            let dxgi: IDXGIDevice = device.cast()?;
            let winrt = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            Ok(Self {
                device,
                context,
                winrt,
                hardware: true,
            })
        }
    }
    pub fn new() -> Result<Self> {
        // SAFETY: COM is initialized by the caller. Out parameters are valid owned Options.
        unsafe {
            let mut device = None;
            let mut context = None;
            let create = |driver, video: bool, device: &mut _, context: &mut _| {
                D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT
                        | if video {
                            D3D11_CREATE_DEVICE_VIDEO_SUPPORT
                        } else {
                            D3D11_CREATE_DEVICE_FLAG(0)
                        },
                    None,
                    D3D11_SDK_VERSION,
                    Some(device),
                    None,
                    Some(context),
                )
            };
            let hardware = match create(D3D_DRIVER_TYPE_HARDWARE, true, &mut device, &mut context)
                .or_else(|_| create(D3D_DRIVER_TYPE_HARDWARE, false, &mut device, &mut context))
            {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error,"Hardware D3D11 unavailable; trying WARP");
                    create(D3D_DRIVER_TYPE_WARP, false, &mut device, &mut context)?;
                    false
                }
            };
            let device = device.ok_or_else(|| {
                windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    "D3D11 returned no device",
                )
            })?;
            let context = context.ok_or_else(|| {
                windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    "D3D11 returned no context",
                )
            })?;
            let multithread: ID3D11Multithread = context.cast()?;
            let _ = multithread.SetMultithreadProtected(true);
            let dxgi: IDXGIDevice = device.cast()?;
            let winrt = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            Ok(Self {
                device,
                context,
                winrt,
                hardware,
            })
        }
    }
}
/// Owned GPU copy, safe after releasing the WGC frame. No pixel readback.
/// Contents are reused on the next poll; consume on the owning thread before polling again.
pub struct GpuFrame {
    pub texture: ID3D11Texture2D,
    pub metadata: FrameMetadata,
}
struct PendingWindow {
    source: CaptureSource,
    restored_at: Option<Instant>,
    last_probe: Option<Instant>,
    was_foreground: bool,
    waiting_for_size: bool,
}
impl PendingWindow {
    fn new(source: CaptureSource, waiting_for_size: bool) -> Self {
        Self {
            source,
            restored_at: None,
            last_probe: None,
            was_foreground: false,
            waiting_for_size,
        }
    }
}
struct Session {
    source: CaptureSource,
    black_probe: Option<ID3D11Texture2D>,
    last_black_probe: Option<Instant>,
    black_fullscreen_since: Option<Instant>,
    black_fullscreen_logged: bool,
    saw_non_black_frame: bool,
    item: GraphicsCaptureItem,
    closed_token: i64,
    closed: Arc<AtomicBool>,
    pool: Direct3D11CaptureFramePool,
    frame_token: i64,
    pool_lock: Arc<Mutex<()>>,
    latest_frame: Arc<Mutex<Option<Direct3D11CaptureFrame>>>,
    capture: GraphicsCaptureSession,
    pool_size: SizeInt32,
    sequence: u64,
    started_at: Instant,
    copy: Option<(Dimensions, ID3D11Texture2D)>,
    suspended: bool,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let _pool_guard = self.pool_lock.lock().unwrap();
        if let Some(frame) = self.latest_frame.lock().unwrap().take() {
            let _ = frame.Close();
        }
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.pool.RemoveFrameArrived(self.frame_token);
        let _ = self.capture.Close();
        let _ = self.pool.Close();
    }
}
pub struct WindowsCapture {
    pub graphics: GraphicsDevice,
    session: Option<Session>,
    pending: Option<PendingWindow>,
    show_cursor: bool,
    min_update_interval: Option<Duration>,
}
impl WindowsCapture {
    pub fn new(graphics: GraphicsDevice) -> Self {
        Self {
            graphics,
            session: None,
            pending: None,
            show_cursor: true,
            min_update_interval: None,
        }
    }
    /// Applies to the existing WGC session without replacing capture or the encoder.
    pub fn set_cursor_capture(&mut self,enabled:bool)->Result<()> {
        if self.show_cursor!=enabled {
            if let Some(session)=&self.session{session.capture.SetIsCursorCaptureEnabled(enabled)?;}
            self.show_cursor=enabled;
        }
        Ok(())
    }
    /// Hints the minimum frame delivery interval to Windows Graphics Capture when supported.
    pub fn set_min_update_interval(&mut self, interval: Option<Duration>) {
        if self.min_update_interval == interval {
            return;
        }
        self.min_update_interval = interval;
        if let Some(session) = &self.session {
            apply_min_update_interval(&session.capture, interval);
        }
    }
    pub fn waiting_for_restore(&self) -> bool {
        self.pending.is_some()
    }
    pub fn status_hint(&self) -> Option<&'static str> {
        if self.pending.is_some() {
            return Some("Waiting for window — switch back to the shared window to start streaming.");
        }
        if let Some(session) = &self.session
            && session.source.kind == SourceKind::Window
            && ((session.sequence == 0 && session.started_at.elapsed() >= NO_FRAME_HINT_DELAY)
                || (!session.saw_non_black_frame
                    && session
                        .black_fullscreen_since
                        .is_some_and(|since| since.elapsed() >= NO_FRAME_HINT_DELAY)))
        {
            return Some(
                "No video frames received from fullscreen game. Uncheck 'Disable fullscreen optimizations' on the game .exe or switch the game to Borderless.",
            );
        }
        None
    }
    pub fn state(&self) -> CaptureState {
        if let Some(pending) = &self.pending {
            let hwnd = HWND(pending.source.id as *mut _);
            // SAFETY: native handle checked with Win32 IsWindow before waiting for restore.
            return if unsafe { IsWindow(Some(hwnd)).as_bool() } {
                CaptureState::Paused
            } else {
                CaptureState::SourceClosed
            };
        }
        let Some(session) = &self.session else {
            return CaptureState::Stopped;
        };
        if session.source.kind == SourceKind::Window {
            // SAFETY: native handles are checked before use. Stale handles stop capture,
            // while minimized or transient WGC item closes pause and reattach cleanly.
            unsafe {
                let hwnd = HWND(session.source.id as *mut _);
                if !IsWindow(Some(hwnd)).as_bool() {
                    return CaptureState::SourceClosed;
                }
                if IsIconic(hwnd).as_bool() || session.closed.load(Ordering::Acquire) {
                    return CaptureState::Paused;
                }
            }
            return CaptureState::Capturing;
        }
        if session.closed.load(Ordering::Acquire) {
            return CaptureState::SourceClosed;
        }
        CaptureState::Capturing
    }
    fn open_session(&self, source: &CaptureSource) -> Result<Option<Session>> {
        let interop: IGraphicsCaptureItemInterop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        // SAFETY: source handles originate from Win32 enumeration; interop checks their validity.
        let item_result: Result<GraphicsCaptureItem> = unsafe {
            match source.kind {
                SourceKind::Monitor => interop.CreateForMonitor(HMONITOR(source.id as *mut _)),
                SourceKind::Window => interop.CreateForWindow(HWND(source.id as *mut _)),
            }
        };
        let item = match item_result {
            Ok(item) => item,
            Err(error) if error.code() == E_ACCESSDENIED => {
                return Err(windows::core::Error::new(
                    E_ACCESSDENIED,
                    "Cannot capture an elevated (Administrator) window unless P2PShare is also run as Administrator",
                ));
            }
            Err(error) => return Err(error),
        };
        let size = item.Size()?;
        if !usable_capture_size(Dimensions::new(
            size.Width.max(0) as u32,
            size.Height.max(0) as u32,
        )) {
            if source.kind == SourceKind::Window {
                tracing::debug!(
                    title = %source.title,
                    width = size.Width,
                    height = size.Height,
                    "capture_window_item_size_unusable"
                );
                return Ok(None);
            }
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80070057u32 as i32),
                "Selected monitor reported an unusable capture size",
            ));
        }
        struct SetupGuard<'a> {
            item: &'a GraphicsCaptureItem,
            closed_token: Option<i64>,
            pool: Option<Direct3D11CaptureFramePool>,
            frame_token: Option<i64>,
            pool_lock: &'a Arc<Mutex<()>>,
            latest_frame: &'a Arc<Mutex<Option<Direct3D11CaptureFrame>>>,
        }
        impl Drop for SetupGuard<'_> {
            fn drop(&mut self) {
                let _pool_guard = self.pool_lock.lock().unwrap();
                if let Some(frame) = self.latest_frame.lock().unwrap().take() {
                    let _ = frame.Close();
                }
                if let (Some(pool), Some(token)) = (&self.pool, self.frame_token.take()) {
                    let _ = pool.RemoveFrameArrived(token);
                }
                if let Some(pool) = self.pool.take() {
                    let _ = pool.Close();
                }
                if let Some(token) = self.closed_token.take() {
                    let _ = self.item.RemoveClosed(token);
                }
            }
        }
        let closed = Arc::new(AtomicBool::new(false));
        let pool_lock = Arc::new(Mutex::new(()));
        let latest_frame = Arc::new(Mutex::new(None));
        let mut guard = SetupGuard {
            item: &item,
            closed_token: None,
            pool: None,
            frame_token: None,
            pool_lock: &pool_lock,
            latest_frame: &latest_frame,
        };
        let signal = closed.clone();
        guard.closed_token = Some(item.Closed(&TypedEventHandler::new(move |_, _| {
            signal.store(true, Ordering::Release);
            Ok(())
        }))?);
        guard.pool = Some(Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.graphics.winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            POOL_BUFFERS,
            size,
        )?);
        let handler_closed = closed.clone();
        let handler_lock = pool_lock.clone();
        let handler_latest = latest_frame.clone();
        // Drain extra WGC buffers in FrameArrived so the pool never stalls DWM or the game.
        guard.frame_token = Some(guard.pool.as_ref().unwrap().FrameArrived(
            &TypedEventHandler::<Direct3D11CaptureFramePool, windows::core::IInspectable>::new(
                move |sender, _| {
                if handler_closed.load(Ordering::Acquire) {
                    return Ok(());
                }
                let Some(pool) = sender.as_ref() else {
                    return Ok(());
                };
                let Ok(_pool_guard) = handler_lock.try_lock() else {
                    return Ok(());
                };
                let mut newest: Option<Direct3D11CaptureFrame> = None;
                for _ in 0..8 {
                    let frame = match pool.TryGetNextFrame() {
                        Ok(frame) => frame,
                        Err(error) if error.code() == windows::Win32::Foundation::S_OK => break,
                        Err(_) => break,
                    };
                    if let Some(old) = newest.replace(frame) {
                        let _ = old.Close();
                    }
                }
                if let Some(frame) = newest
                    && let Some(old) = handler_latest.lock().unwrap().replace(frame)
                {
                    let _ = old.Close();
                }
                Ok(())
            }),
        )?);
        let capture = guard.pool.as_ref().unwrap().CreateCaptureSession(&item)?;
        let session = Session {
            source: source.clone(),
            black_probe: None,
            last_black_probe: None,
            black_fullscreen_since: None,
            black_fullscreen_logged: false,
            saw_non_black_frame: false,
            item: item.clone(),
            closed_token: guard.closed_token.take().unwrap(),
            closed,
            pool: guard.pool.take().unwrap(),
            frame_token: guard.frame_token.take().unwrap(),
            pool_lock: pool_lock.clone(),
            latest_frame: latest_frame.clone(),
            capture,
            pool_size: size,
            sequence: 0,
            started_at: Instant::now(),
            suspended: false,
            copy: None,
        };
        drop(guard);
        // Full-trust desktop capture, as used by Sunshine. Older Windows versions
        // may not expose border/cursor/interval toggles; capture must still work there.
        if let Err(error) = session.capture.SetIsBorderRequired(false) {
            tracing::warn!(%error, "capture_border_toggle_unavailable");
        }
        if !self.show_cursor
            && let Err(error) = session.capture.SetIsCursorCaptureEnabled(false)
        {
            tracing::warn!(%error, "capture_cursor_toggle_unavailable");
        }
        apply_min_update_interval(&session.capture, self.min_update_interval);
        session.capture.StartCapture()?;
        tracing::info!(
            kind = ?source.kind,
            width = size.Width,
            height = size.Height,
            "Capture started"
        );
        Ok(Some(session))
    }
}
/// WGC can report 1x1 placeholders during fullscreen/minimize transitions.
/// These are not usable stream resolutions and must never replace a live frame pool.
pub fn usable_capture_size(size:Dimensions)->bool{size.width>=64&&size.height>=64}
#[cfg(test)]mod transition_tests{
 use super::*;
 #[test]fn fullscreen_placeholders_do_not_replace_valid_capture_sizes(){for (w,h) in [(0,0),(1,1),(2,2),(146,21),(1920,1)]{assert!(!usable_capture_size(Dimensions::new(w,h)));}for (w,h) in [(64,64),(1366,768),(1920,1080)]{assert!(usable_capture_size(Dimensions::new(w,h)));}}
 #[test]fn min_update_interval_converts_duration_to_100ns_ticks(){assert_eq!(min_update_ticks(None),None);assert_eq!(min_update_ticks(Some(Duration::from_millis(1))),Some(10_000));assert_eq!(min_update_ticks(Some(Duration::from_millis(16))),Some(160_000));assert_eq!(min_update_ticks(Some(Duration::from_millis(250))),Some(2_500_000));}
 #[test]fn rect_covers_monitor_detects_fullscreen_windows_only(){
  let mon=RECT{left:0,top:0,right:1920,bottom:1080};
  assert!(rect_covers_monitor(mon,mon));
  assert!(rect_covers_monitor(RECT{left:1,top:1,right:1919,bottom:1079},mon));
  assert!(rect_covers_monitor(RECT{left:-8,top:-8,right:1928,bottom:1088},mon));
  assert!(!rect_covers_monitor(RECT{left:0,top:0,right:1920,bottom:1032},mon));
  assert!(!rect_covers_monitor(RECT{left:100,top:100,right:1700,bottom:1000},mon));
  let mon2=RECT{left:1920,top:0,right:3840,bottom:1080};
  assert!(rect_covers_monitor(mon2,mon2));
  assert!(!rect_covers_monitor(mon,mon2));
 }
 #[test]fn black_frame_probe_ignores_single_cursor_sample_and_detects_real_content(){
  let all_black=[[0,0,0,255];16];
  assert_eq!(count_non_black_bgra_samples(&all_black),0);
  let mut black_with_cursor=all_black;
  black_with_cursor[5]=[255,255,255,255];
  assert_eq!(count_non_black_bgra_samples(&black_with_cursor),1);
  let mut real_game=all_black;
  real_game[0]=[18,45,32,255];
  real_game[15]=[120,90,40,255];
  assert_eq!(count_non_black_bgra_samples(&real_game),2);
 }
}
impl CaptureBackend for WindowsCapture {
    type Frame = GpuFrame;
    type Error = windows::core::Error;
    fn sources(&self) -> Result<Vec<CaptureSource>> {
        crate::enumerate_sources(None)
    }
    fn start(&mut self, source: &CaptureSource) -> Result<()> {
        self.stop()?;
        if !GraphicsCaptureSession::IsSupported()? {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80004001u32 as i32),
                "Windows Graphics Capture requires a supported Windows 10/11 desktop",
            ));
        }
        if source.kind == SourceKind::Window {
            let hwnd = HWND(source.id as *mut _);
            // SAFETY: Win32 checks window validity before inspecting iconic state.
            unsafe {
                if !IsWindow(Some(hwnd)).as_bool() {
                    return Err(windows::core::Error::new(
                        windows::core::HRESULT(0x80070057u32 as i32),
                        "The selected window is no longer open",
                    ));
                }
                if IsIconic(hwnd).as_bool() {
                    self.pending = Some(PendingWindow::new(source.clone(), false));
                    tracing::info!(title = %source.title, "capture_waiting_for_window_restore");
                    return Ok(());
                }
            }
        }
        match self.open_session(source)? {
            Some(session) => {
                self.session = Some(session);
            }
            None => {
                self.pending = Some(PendingWindow::new(source.clone(), true));
                tracing::info!(title = %source.title, "capture_waiting_for_window_size");
            }
        }
        Ok(())
    }
    fn poll_frame(&mut self) -> Result<Option<GpuFrame>> {
        if self.session.is_some() && self.state() == CaptureState::Paused {
            if let Some(session) = self.session.take() {
                let reason = if session.closed.load(Ordering::Acquire) {
                    "item_closed"
                } else {
                    "minimized"
                };
                tracing::info!(
                    reason,
                    title = %session.source.title,
                    "capture_released_until_window_restored"
                );
                self.pending = Some(PendingWindow::new(
                    session.source.clone(),
                    reason == "item_closed",
                ));
            }
            return Ok(None);
        }
        if let Some(mut pending) = self.pending.take() {
            let hwnd = HWND(pending.source.id as *mut _);
            // SAFETY: Win32 validates the HWND; drop stops cleanly once the window closes.
            if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
                return Ok(None);
            }
            if !window_usable_for_capture(hwnd) {
                pending.restored_at = None;
                pending.last_probe = None;
                pending.was_foreground = false;
                self.pending = Some(pending);
                return Ok(None);
            }
            let now = Instant::now();
            // SAFETY: Win32 GetForegroundWindow has no preconditions.
            let is_fg = unsafe { GetForegroundWindow() == hwnd };
            if pending.waiting_for_size && is_fg && !pending.was_foreground {
                // Give a newly refocused fullscreen window time to finish switching display modes.
                pending.restored_at = Some(now);
            }
            pending.was_foreground = is_fg;
            let ready_since = *pending.restored_at.get_or_insert(now);
            if now.duration_since(ready_since) < RESTORE_SETTLE_DELAY {
                self.pending = Some(pending);
                return Ok(None);
            }
            if pending
                .last_probe
                .is_some_and(|last| now.duration_since(last) < PENDING_PROBE_INTERVAL)
            {
                self.pending = Some(pending);
                return Ok(None);
            }
            pending.last_probe = Some(now);
            match self.open_session(&pending.source) {
                Ok(Some(session)) => {
                    tracing::info!(
                        title = %pending.source.title,
                        width = session.pool_size.Width,
                        height = session.pool_size.Height,
                        "capture_attached_after_window_restore"
                    );
                    self.session = Some(session);
                }
                Ok(None) => {
                    pending.waiting_for_size = true;
                    self.pending = Some(pending);
                }
                Err(error) => {
                    if error.code() == E_ACCESSDENIED || !unsafe { IsWindow(Some(hwnd)).as_bool() } {
                        return Err(error);
                    }
                    tracing::debug!(%error, title = %pending.source.title, "capture_reattach_retry");
                    pending.waiting_for_size = true;
                    self.pending = Some(pending);
                }
            }
            return Ok(None);
        }
        if self.state() != CaptureState::Capturing {
            return Ok(None);
        }
        let latest = {
            let Some(session) = &mut self.session else {
                return Ok(None);
            };
            let mut latest = session.latest_frame.lock().unwrap().take();
            if latest.is_none() {
                let _pool_guard = session.pool_lock.lock().unwrap();
                for _ in 0..2 {
                    let frame = match session.pool.TryGetNextFrame() {
                        Ok(frame) => frame,
                        Err(error) if error.code() == windows::Win32::Foundation::S_OK => break,
                        Err(error) => return Err(error),
                    };
                    // windows-core 0.62 maps a successful null frame to Error::empty (S_OK).
                    if let Some(old) = latest.replace(frame) {
                        old.Close()?;
                    }
                }
            }
            latest
        };
        let Some(frame) = latest else {
            return Ok(None);
        };
        let size = frame.ContentSize()?;
        if !usable_capture_size(Dimensions::new(size.Width.max(0) as u32, size.Height.max(0) as u32))
        {
            frame.Close()?;
            let release_window = self
                .session
                .as_ref()
                .is_some_and(|s| s.source.kind == SourceKind::Window);
            if release_window {
                let source = self.session.take().unwrap().source.clone();
                tracing::info!(
                    width = size.Width,
                    height = size.Height,
                    reason = "placeholder_frame",
                    title = %source.title,
                    "capture_released_until_window_restored"
                );
                self.pending = Some(PendingWindow::new(source, true));
            } else if let Some(session) = &mut self.session
                && !session.suspended
            {
                tracing::info!(
                    width = size.Width,
                    height = size.Height,
                    reason = "placeholder_frame",
                    "capture_transition_paused"
                );
                session.suspended = true;
            }
            return Ok(None);
        }
        let Some(session) = &mut self.session else {
            frame.Close()?;
            return Ok(None);
        };
        if session.suspended {
            tracing::info!(
                width = size.Width,
                height = size.Height,
                "capture_transition_resumed"
            );
            session.suspended = false;
        }
        let resized = size != session.pool_size;
        let dimensions = Dimensions::new(size.Width.max(0) as u32, size.Height.max(0) as u32);
        let result = (|| -> Result<Option<GpuFrame>> {
            if dimensions.is_empty() {
                return Ok(None);
            }
            let surface = frame.Surface()?;
            let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
            // SAFETY: D3D textures belong to our device, and copies are serialized on the UI thread.
            unsafe {
                let texture: ID3D11Texture2D = access.GetInterface()?;
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                // On a growing window WGC can report a size larger than the old pool's texture.
                if desc.Width < dimensions.width || desc.Height < dimensions.height {
                    return Ok(None);
                }
                if session.copy.as_ref().map(|(s, _)| *s) != Some(dimensions) {
                    desc.Width = dimensions.width;
                    desc.Height = dimensions.height;
                    desc.MipLevels = 1;
                    desc.ArraySize = 1;
                    desc.Usage = D3D11_USAGE_DEFAULT;
                    // Video-processor input views require a video/render bind flag as well.
                    desc.BindFlags =
                        (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32;
                    desc.CPUAccessFlags = 0;
                    desc.MiscFlags = 0;
                    let mut owned = None;
                    self.graphics
                        .device
                        .CreateTexture2D(&desc, None, Some(&mut owned))?;
                    session.copy = Some((dimensions, owned.unwrap()));
                }
                let owned = &session.copy.as_ref().unwrap().1;
                self.graphics.context.CopySubresourceRegion(
                    owned,
                    0,
                    0,
                    0,
                    0,
                    &texture,
                    0,
                    Some(&D3D11_BOX {
                        left: 0,
                        top: 0,
                        front: 0,
                        right: dimensions.width,
                        bottom: dimensions.height,
                        back: 1,
                    }),
                );
                session.sequence += 1;
                let metadata = FrameMetadata {
                    size: dimensions,
                    timestamp: Duration::from_nanos(
                        frame.SystemRelativeTime()?.Duration.max(0) as u64 * 100,
                    ),
                    sequence: session.sequence,
                    resized,
                };
                Ok(Some(GpuFrame {
                    texture: owned.clone(),
                    metadata,
                }))
            }
        })();
        frame.Close()?;
        drop(frame);
        if resized && !dimensions.is_empty() {
            let _pool_guard = session.pool_lock.lock().unwrap();
            if let Some(stale) = session.latest_frame.lock().unwrap().take() {
                let _ = stale.Close();
            }
            session.pool.Recreate(
                &self.graphics.winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                POOL_BUFFERS,
                size,
            )?;
            session.pool_size = size;
            tracing::info!(
                width = size.Width,
                height = size.Height,
                "Capture source resized"
            );
        }
        if let Ok(Some(ref gpu_frame)) = result
            && session.source.kind == SourceKind::Window
            && !session.saw_non_black_frame
        {
            let now = Instant::now();
            if session
                .last_black_probe
                .is_none_or(|last| now.duration_since(last) >= BLACK_PROBE_INTERVAL)
            {
                session.last_black_probe = Some(now);
                let hwnd = HWND(session.source.id as *mut _);
                if foreground_fullscreen_monitor(hwnd).is_some() {
                    if let Some(non_black) = probe_non_black_samples(
                        &self.graphics,
                        &mut session.black_probe,
                        &gpu_frame.texture,
                        dimensions,
                    ) {
                        if non_black >= 2 {
                            session.saw_non_black_frame = true;
                            session.black_probe = None;
                            session.black_fullscreen_since = None;
                        } else {
                            let since = *session.black_fullscreen_since.get_or_insert(now);
                            if now.duration_since(since) >= NO_FRAME_HINT_DELAY
                                && !session.black_fullscreen_logged
                            {
                                session.black_fullscreen_logged = true;
                                tracing::warn!(
                                    title = %session.source.title,
                                    non_black,
                                    "capture_fullscreen_window_black_frames"
                                );
                            }
                        }
                    }
                } else {
                    session.black_fullscreen_since = None;
                }
            }
        }
        result
    }
    fn stop(&mut self) -> Result<()> {
        let stopped_pending = self.pending.take().is_some();
        let stopped_session = self.session.take().is_some();
        if stopped_pending || stopped_session {
            tracing::info!("Capture stopped");
        }
        Ok(())
    }
}
