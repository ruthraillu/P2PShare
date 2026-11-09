//! Minimal WASAPI COM ABI from the Windows SDK. Keeps audio independent of the
//! large generated graphics binding feature set, reducing compile memory.
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]
use std::ffi::c_void;
use windows::Win32::System::Com::CLSCTX;
use windows::core as windows_core;
use windows::core::{GUID, HRESULT, IUnknown_Vtbl, Interface, Result};
pub const MMDeviceEnumerator: GUID = GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);
pub const eRender: i32 = 0;
pub const eConsole: i32 = 0;
pub const AUDCLNT_SHAREMODE_SHARED: i32 = 0;
pub const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x20000;
pub const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x80000000;
pub const AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY: u32 = 0x08000000;
pub struct Flag(pub i32);
pub const AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY: Flag = Flag(1);
pub const AUDCLNT_BUFFERFLAGS_SILENT: Flag = Flag(2);
#[repr(C, packed(1))]
pub struct WaveFormatEx {
    pub wFormatTag: u16,
    pub nChannels: u16,
    pub nSamplesPerSec: u32,
    pub nAvgBytesPerSec: u32,
    pub nBlockAlign: u16,
    pub wBitsPerSample: u16,
    pub cbSize: u16,
}
windows_core::imp::define_interface!(
    IMMDeviceEnumerator,
    IMMDeviceEnumerator_Vtbl,
    0xa95664d2_9614_4f35_a746_de8db63617e6
);
#[repr(C)]
pub struct IMMDeviceEnumerator_Vtbl {
    pub base__: IUnknown_Vtbl,
    pub EnumAudioEndpoints: usize,
    pub GetDefaultAudioEndpoint:
        unsafe extern "system" fn(*mut c_void, i32, i32, *mut *mut c_void) -> HRESULT,
    pub GetDevice: usize,
    pub RegisterEndpointNotificationCallback: usize,
    pub UnregisterEndpointNotificationCallback: usize,
}
impl IMMDeviceEnumerator {
    pub unsafe fn GetDefaultAudioEndpoint(&self, flow: i32, role: i32) -> Result<IMMDevice> {
        unsafe {
            let mut out = std::ptr::null_mut();
            (self.vtable().GetDefaultAudioEndpoint)(self.as_raw(), flow, role, &mut out).ok()?;
            Ok(IMMDevice::from_raw(out))
        }
    }
}
windows_core::imp::define_interface!(
    IMMDevice,
    IMMDevice_Vtbl,
    0xd666063f_1587_4e43_81f1_b948e807363f
);
#[repr(C)]
pub struct IMMDevice_Vtbl {
    pub base__: IUnknown_Vtbl,
    pub Activate: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        u32,
        *const c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    pub OpenPropertyStore: usize,
    pub GetId: usize,
    pub GetState: usize,
}
impl IMMDevice {
    pub unsafe fn Activate<T: Interface>(&self, context: CLSCTX, _params: Option<()>) -> Result<T> {
        unsafe {
            let mut out = std::ptr::null_mut();
            (self.vtable().Activate)(
                self.as_raw(),
                &T::IID,
                context.0,
                std::ptr::null(),
                &mut out,
            )
            .ok()?;
            Ok(T::from_raw(out))
        }
    }
}
windows_core::imp::define_interface!(
    IAudioClient,
    IAudioClient_Vtbl,
    0x1cb9ad4c_dbfa_4c32_b178_c2f568a703b2
);
#[repr(C)]
pub struct IAudioClient_Vtbl {
    pub base__: IUnknown_Vtbl,
    pub Initialize: unsafe extern "system" fn(
        *mut c_void,
        i32,
        u32,
        i64,
        i64,
        *const WaveFormatEx,
        *const GUID,
    ) -> HRESULT,
    pub GetBufferSize: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    pub GetStreamLatency: usize,
    pub GetCurrentPadding: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    pub IsFormatSupported: usize,
    pub GetMixFormat: usize,
    pub GetDevicePeriod: usize,
    pub Start: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub Stop: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub Reset: usize,
    pub SetEventHandle: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub GetService:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
}
impl IAudioClient {
    pub unsafe fn SetEventHandle(&self, handle: *mut c_void) -> Result<()> {
        unsafe { (self.vtable().SetEventHandle)(self.as_raw(), handle).ok() }
    }
    pub unsafe fn Initialize(
        &self,
        mode: i32,
        flags: u32,
        duration: i64,
        period: i64,
        format: *const WaveFormatEx,
        guid: Option<*const GUID>,
    ) -> Result<()> {
        unsafe {
            (self.vtable().Initialize)(
                self.as_raw(),
                mode,
                flags,
                duration,
                period,
                format,
                guid.unwrap_or(std::ptr::null()),
            )
            .ok()
        }
    }
    pub unsafe fn GetBufferSize(&self) -> Result<u32> {
        unsafe {
            let mut out = 0;
            (self.vtable().GetBufferSize)(self.as_raw(), &mut out).ok()?;
            Ok(out)
        }
    }
    pub unsafe fn GetCurrentPadding(&self) -> Result<u32> {
        unsafe {
            let mut out = 0;
            (self.vtable().GetCurrentPadding)(self.as_raw(), &mut out).ok()?;
            Ok(out)
        }
    }
    pub unsafe fn Start(&self) -> Result<()> {
        unsafe { (self.vtable().Start)(self.as_raw()).ok() }
    }
    pub unsafe fn Stop(&self) -> Result<()> {
        unsafe { (self.vtable().Stop)(self.as_raw()).ok() }
    }
    pub unsafe fn GetService<T: Interface>(&self) -> Result<T> {
        unsafe {
            let mut out = std::ptr::null_mut();
            (self.vtable().GetService)(self.as_raw(), &T::IID, &mut out).ok()?;
            Ok(T::from_raw(out))
        }
    }
}
windows_core::imp::define_interface!(
    IAudioCaptureClient,
    IAudioCaptureClient_Vtbl,
    0xc8adbd64_e71e_48a0_a4de_185c395cd317
);
#[repr(C)]
pub struct IAudioCaptureClient_Vtbl {
    pub base__: IUnknown_Vtbl,
    pub GetBuffer: unsafe extern "system" fn(
        *mut c_void,
        *mut *mut u8,
        *mut u32,
        *mut u32,
        *mut u64,
        *mut u64,
    ) -> HRESULT,
    pub ReleaseBuffer: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    pub GetNextPacketSize: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
}
impl IAudioCaptureClient {
    pub unsafe fn GetBuffer(
        &self,
        data: *mut *mut u8,
        frames: *mut u32,
        flags: *mut u32,
        device: Option<*mut u64>,
        qpc: Option<*mut u64>,
    ) -> Result<()> {
        unsafe {
            (self.vtable().GetBuffer)(
                self.as_raw(),
                data,
                frames,
                flags,
                device.unwrap_or(std::ptr::null_mut()),
                qpc.unwrap_or(std::ptr::null_mut()),
            )
            .ok()
        }
    }
    pub unsafe fn ReleaseBuffer(&self, frames: u32) -> Result<()> {
        unsafe { (self.vtable().ReleaseBuffer)(self.as_raw(), frames).ok() }
    }
    pub unsafe fn GetNextPacketSize(&self) -> Result<u32> {
        unsafe {
            let mut out = 0;
            (self.vtable().GetNextPacketSize)(self.as_raw(), &mut out).ok()?;
            Ok(out)
        }
    }
}
windows_core::imp::define_interface!(
    IAudioRenderClient,
    IAudioRenderClient_Vtbl,
    0xf294acfc_3146_4483_a7bf_addca7c260e2
);
#[repr(C)]
pub struct IAudioRenderClient_Vtbl {
    pub base__: IUnknown_Vtbl,
    pub GetBuffer: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u8) -> HRESULT,
    pub ReleaseBuffer: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
}
impl IAudioRenderClient {
    pub unsafe fn GetBuffer(&self, frames: u32) -> Result<*mut u8> {
        unsafe {
            let mut out = std::ptr::null_mut();
            (self.vtable().GetBuffer)(self.as_raw(), frames, &mut out).ok()?;
            Ok(out)
        }
    }
    pub unsafe fn ReleaseBuffer(&self, frames: u32, flags: u32) -> Result<()> {
        unsafe { (self.vtable().ReleaseBuffer)(self.as_raw(), frames, flags).ok() }
    }
}

// Async activation interfaces use the SDK ABI; the completion object is agile.
windows_core::imp::define_interface!(
    IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceAsyncOperation_Vtbl,
    0x72a22d78_cde4_431d_b8cc_843a71199b6d
);
#[repr(C)]
pub struct IActivateAudioInterfaceAsyncOperation_Vtbl {
    pub base__: IUnknown_Vtbl,
    pub GetActivateResult:
        unsafe extern "system" fn(*mut c_void, *mut HRESULT, *mut *mut c_void) -> HRESULT,
}
windows_core::imp::define_interface!(
    IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Vtbl,
    0x41d949ab_9862_444a_80f6_c261334da5eb
);
#[repr(C)]
pub struct IActivateAudioInterfaceCompletionHandler_Vtbl {
    pub base__: IUnknown_Vtbl,
    pub ActivateCompleted: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}
pub trait IActivateAudioInterfaceCompletionHandler_Impl: windows_core::IUnknownImpl {
    fn ActivateCompleted(
        &self,
        operation: windows_core::Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> Result<()>;
}
impl IActivateAudioInterfaceCompletionHandler_Vtbl {
    pub const fn new<
        Identity: IActivateAudioInterfaceCompletionHandler_Impl,
        const OFFSET: isize,
    >() -> Self {
        unsafe extern "system" fn complete<
            Identity: IActivateAudioInterfaceCompletionHandler_Impl,
            const OFFSET: isize,
        >(
            this: *mut c_void,
            operation: *mut c_void,
        ) -> HRESULT {
            unsafe {
                let owner: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                owner
                    .ActivateCompleted(core::mem::transmute_copy(&operation))
                    .into()
            }
        }
        Self {
            base__: IUnknown_Vtbl::new::<Identity, OFFSET>(),
            ActivateCompleted: complete::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &GUID) -> bool {
        iid == &IActivateAudioInterfaceCompletionHandler::IID
    }
}
impl windows_core::RuntimeName for IActivateAudioInterfaceCompletionHandler {}
#[windows_core::implement(IActivateAudioInterfaceCompletionHandler)]
struct Activation {
    // Keep the pointed-to activation blob alive until the async callback finishes.
    params: Box<[u32; 3]>,
    sender: std::sync::mpsc::SyncSender<ActivationResult>,
}
impl IActivateAudioInterfaceCompletionHandler_Impl for Activation_Impl {
    fn ActivateCompleted(
        &self,
        operation: windows_core::Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> Result<()> {
        let result = (|| -> Result<IAudioClient> {
            let operation = operation.ok()?;
            let mut status = HRESULT(0);
            let mut raw = std::ptr::null_mut();
            unsafe {
                (operation.vtable().GetActivateResult)(operation.as_raw(), &mut status, &mut raw)
                    .ok()?;
            }
            if raw.is_null() {
                status.ok()?;
                return Err(windows_core::Error::from_hresult(HRESULT(
                    0x80004003u32 as i32,
                )));
            }
            let interface = unsafe { windows_core::IUnknown::from_raw(raw) };
            status.ok()?;
            interface.cast()
        })();
        let value = match result {
            Ok(client) => ActivationResult {
                status: 0,
                raw: client.into_raw() as usize,
            },
            Err(error) => ActivationResult {
                status: error.code().0,
                raw: 0,
            },
        };
        let _ = self.sender.send(value);
        Ok(())
    }
}
#[repr(C)]
struct Blob {
    size: u32,
    data: *const u32,
}
#[repr(C)]
struct BlobVariant {
    vt: u16,
    reserved: [u16; 3],
    blob: Blob,
}
#[link(name = "Mmdevapi")]
unsafe extern "system" {
    fn ActivateAudioInterfaceAsync(
        device: *const u16,
        iid: *const GUID,
        params: *const BlobVariant,
        callback: *mut c_void,
        operation: *mut *mut c_void,
    ) -> HRESULT;
}
/// Capture a window owner's process tree, excluding unrelated applications.
pub fn including_process(process_id: u32) -> anyhow::Result<IAudioClient> {
    anyhow::ensure!(process_id != 0, "Window has no audio process");
    process_loopback(process_id, 0)
}
/// Capture all processes except this application's playback for display shares.
pub fn excluding_own_process() -> anyhow::Result<IAudioClient> {
    process_loopback(std::process::id(), 1)
}
fn process_loopback(process_id: u32, mode: u32) -> anyhow::Result<IAudioClient> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let completion = Activation {
        params: Box::new([1, process_id, mode]),
        sender,
    };
    let variant = BlobVariant {
        vt: 65,
        reserved: [0; 3],
        blob: Blob {
            size: 12,
            data: completion.params.as_ptr(),
        },
    };
    let callback: IActivateAudioInterfaceCompletionHandler = completion.into();
    let device: Vec<u16> = "VAD\\Process_Loopback"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut operation = std::ptr::null_mut();
    unsafe {
        ActivateAudioInterfaceAsync(
            device.as_ptr(),
            &IAudioClient::IID,
            &variant,
            callback.as_raw(),
            &mut operation,
        )
        .ok()?;
    }
    anyhow::ensure!(
        !operation.is_null(),
        "No process-loopback activation operation"
    );
    let _operation = unsafe { IActivateAudioInterfaceAsyncOperation::from_raw(operation) };
    let mut result = receiver.recv_timeout(std::time::Duration::from_secs(1))?;
    HRESULT(result.status).ok()?;
    let raw = std::mem::replace(&mut result.raw, 0);
    anyhow::ensure!(raw != 0, "No process-loopback audio client");
    // Completion and caller are both in the shared MTA COM apartment.
    Ok(unsafe { IAudioClient::from_raw(raw as *mut c_void) })
}

// Own the returned COM reference even if completion races the bounded timeout.
// Every sender/receiver/drop site remains in the COM MTA apartment.
struct ActivationResult {
    status: i32,
    raw: usize,
}
impl Drop for ActivationResult {
    fn drop(&mut self) {
        if self.raw != 0 {
            unsafe {
                drop(IAudioClient::from_raw(self.raw as *mut c_void));
            }
        }
    }
}

#[path="audio_sessions.rs"]
mod sessions;
pub use sessions::active_audio_pids;
