use p2pshare_media::{
    CaptureBackend, CaptureSource, CaptureState, Dimensions, FrameMetadata, SourceKind,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{Direct3D::*, Direct3D11::*, Dxgi::IDXGIDevice, Gdi::HMONITOR},
        System::WinRT::{Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop},
        UI::WindowsAndMessaging::{IsIconic, IsWindow},
    },
    core::{Interface, Result},
};

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
struct Session {
    source: CaptureSource,
    item: GraphicsCaptureItem,
    closed_token: i64,
    closed: Arc<AtomicBool>,
    pool: Direct3D11CaptureFramePool,
    frame_token: i64,
    frame_ready: Arc<AtomicBool>,
    capture: GraphicsCaptureSession,
    pool_size: SizeInt32,
    sequence: u64,
    copy: Option<(Dimensions, ID3D11Texture2D)>,
    suspended:bool,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.pool.RemoveFrameArrived(self.frame_token);
        let _ = self.capture.Close();
        let _ = self.pool.Close();
    }
}
pub struct WindowsCapture {
    pub graphics: GraphicsDevice,
    session: Option<Session>,
    show_cursor:bool,
}
impl WindowsCapture {
    pub fn new(graphics: GraphicsDevice) -> Self {
        Self {
            graphics,
            session: None,
            show_cursor:true,
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
    pub fn state(&self) -> CaptureState {
        let Some(session) = &self.session else {
            return CaptureState::Stopped;
        };
        if session.closed.load(Ordering::Relaxed) {
            return CaptureState::SourceClosed;
        }
        if session.source.kind == SourceKind::Window {
            // SAFETY: native handles are checked before use. Stale handles stop capture.
            unsafe {
                let hwnd = HWND(session.source.id as *mut _);
                if !IsWindow(Some(hwnd)).as_bool() {
                    return CaptureState::SourceClosed;
                }
                if IsIconic(hwnd).as_bool() {
                    return CaptureState::Paused;
                }
            }
        }
        CaptureState::Capturing
    }
}
/// WGC can report 1x1 placeholders during fullscreen/minimize transitions.
/// These are not usable stream resolutions and must never replace a live frame pool.
pub fn usable_capture_size(size:Dimensions)->bool{size.width>=64&&size.height>=64}
#[cfg(test)]mod transition_tests{
 use super::*;
 #[test]fn fullscreen_placeholders_do_not_replace_valid_capture_sizes(){for (w,h) in [(0,0),(1,1),(2,2),(146,21),(1920,1)]{assert!(!usable_capture_size(Dimensions::new(w,h)));}for (w,h) in [(64,64),(1366,768),(1920,1080)]{assert!(usable_capture_size(Dimensions::new(w,h)));}}
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
        let interop: IGraphicsCaptureItemInterop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        // SAFETY: source handles originate from Win32 enumeration; interop checks their validity.
        let item: GraphicsCaptureItem = unsafe {
            match source.kind {
                SourceKind::Monitor => interop.CreateForMonitor(HMONITOR(source.id as *mut _))?,
                SourceKind::Window => interop.CreateForWindow(HWND(source.id as *mut _))?,
            }
        };
        let size = item.Size()?;
        if !usable_capture_size(Dimensions::new(size.Width.max(0)as u32,size.Height.max(0)as u32)) {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80070057u32 as i32),
                "Restore the selected window before starting capture",
            ));
        }
        let closed = Arc::new(AtomicBool::new(false));
        let signal = closed.clone();
        let token = item.Closed(&TypedEventHandler::new(move |_, _| {
            signal.store(true, Ordering::Relaxed);
            Ok(())
        }))?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.graphics.winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let frame_ready = Arc::new(AtomicBool::new(false));
        let arrived = frame_ready.clone();
        let frame_token = pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
            arrived.store(true, Ordering::Release);
            Ok(())
        }))?;
        let capture = pool.CreateCaptureSession(&item)?;
        // Full-trust desktop capture, as used by Sunshine. Older Windows versions
        // may not expose the border toggle; capture must still work there.
        if let Err(error) = capture.SetIsBorderRequired(false) {
            tracing::warn!(%error, "capture_border_toggle_unavailable");
        }
        if !self.show_cursor{capture.SetIsCursorCaptureEnabled(false)?;}
        capture.StartCapture()?;
        self.session = Some(Session {
            source: source.clone(),
            item,
            closed_token: token,
            closed,
            pool,
            frame_token,
            frame_ready,
            capture,
            pool_size: size,
            sequence: 0,
            suspended:false,
            copy: None,
        });
        tracing::info!(kind=?source.kind,width=size.Width,height=size.Height,"Capture started");
        Ok(())
    }
    fn poll_frame(&mut self) -> Result<Option<GpuFrame>> {
        if self.state() != CaptureState::Capturing {
            return Ok(None);
        }
        let Some(session) = &mut self.session else {
            return Ok(None);
        };
        if session.source.kind==SourceKind::Window&&unsafe{windows::Win32::UI::WindowsAndMessaging::IsIconic(HWND(session.source.id as *mut _)).as_bool()}{if !session.suspended{tracing::info!(reason="minimized","capture_transition_paused");session.suspended=true;}return Ok(None);}
        if !session.frame_ready.swap(false, Ordering::Acquire) {
            return Ok(None);
        }
        // Free-threaded WGC produces frames independently; poll rather than queue callbacks.
        let mut latest = None;
        for _ in 0..2 {
            let frame = match session.pool.TryGetNextFrame() {
                Ok(frame) => frame,
                Err(error) if error.code() == windows::Win32::Foundation::S_OK => break,
                Err(error) => return Err(error),
            };
            // windows-core 0.62 maps a successful null frame to Error::empty (S_OK).
            if let Some(old) = latest.replace(frame) {
                let old: Direct3D11CaptureFrame = old;
                old.Close()?;
            }
        }
        let Some(frame) = latest else {
            return Ok(None);
        };
        let size = frame.ContentSize()?;
        if !usable_capture_size(Dimensions::new(size.Width.max(0)as u32,size.Height.max(0)as u32)){
            frame.Close()?;
            if !session.suspended{tracing::info!(width=size.Width,height=size.Height,reason="placeholder_frame","capture_transition_paused");session.suspended=true;}
            return Ok(None);
        }
        if session.suspended{tracing::info!(width=size.Width,height=size.Height,"capture_transition_resumed");session.suspended=false;}
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
            session.pool.Recreate(
                &self.graphics.winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )?;
            session.pool_size = size;
            tracing::info!(
                width = size.Width,
                height = size.Height,
                "Capture source resized"
            );
        }
        result
    }
    fn stop(&mut self) -> Result<()> {
        if self.session.take().is_some() {
            tracing::info!("Capture stopped");
        }
        Ok(())
    }
}
