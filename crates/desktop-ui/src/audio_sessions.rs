//! Small audited WASAPI session-enumeration ABI; avoids enabling the huge Audio feature set.
use super::*;
use std::collections::BTreeSet;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
windows_core::imp::define_interface!(
    Collection,
    CollectionVtbl,
    0x0bd7a1be_7a1a_44db_8397_cc5392387b5e
);
#[repr(C)]
pub struct CollectionVtbl {
    pub base__: IUnknown_Vtbl,
    pub count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    pub item: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
}
windows_core::imp::define_interface!(Manager, ManagerVtbl, 0x77aa99a0_1bd6_484f_8bc7_2c654c9a9b6f);
#[repr(C)]
pub struct ManagerVtbl {
    pub base__: IUnknown_Vtbl,
    pub control: usize,
    pub volume: usize,
    pub sessions: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub register: usize,
    pub unregister: usize,
    pub duck: usize,
    pub unduck: usize,
}
windows_core::imp::define_interface!(
    Sessions,
    SessionsVtbl,
    0xe2f5bb11_0570_40ca_acdd_3aa01277dee8
);
#[repr(C)]
pub struct SessionsVtbl {
    pub base__: IUnknown_Vtbl,
    pub count: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    pub item: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> HRESULT,
}
windows_core::imp::define_interface!(Control, ControlVtbl, 0xbfb7ff88_7239_4fc9_8fa2_07c950be9c6d);
#[repr(C)]
pub struct ControlVtbl {
    pub base__: IUnknown_Vtbl,
    pub state: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    pub display: usize,
    pub set_display: usize,
    pub icon: usize,
    pub set_icon: usize,
    pub group: usize,
    pub set_group: usize,
    pub register: usize,
    pub unregister: usize,
    pub identifier: usize,
    pub instance: usize,
    pub pid: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    pub system: usize,
    pub duck: usize,
}
/// Active sessions on every active render endpoint. Ambiguous cross-process and PID-zero
/// sessions are omitted: they cannot be safely attributed to an allowed process tree.
pub fn active_audio_pids() -> anyhow::Result<BTreeSet<u32>> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let mut raw = std::ptr::null_mut();
        let enumerate: unsafe extern "system" fn(
            *mut c_void,
            i32,
            u32,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(enumerator.vtable().EnumAudioEndpoints);
        enumerate(enumerator.as_raw(), eRender, 1, &mut raw).ok()?;
        anyhow::ensure!(!raw.is_null(), "Null audio endpoint collection");
        let devices = Collection::from_raw(raw);
        let mut count = 0;
        (devices.vtable().count)(devices.as_raw(), &mut count).ok()?;
        anyhow::ensure!(count <= 64, "Too many audio endpoints");
        let mut pids = BTreeSet::new();
        for index in 0..count {
            let mut raw = std::ptr::null_mut();
            if let Err(error) = (devices.vtable().item)(devices.as_raw(), index, &mut raw).ok() {
                tracing::debug!(index, %error, "audio_endpoint_item_skipped");
                continue;
            }
            if raw.is_null() {
                tracing::debug!(index, "audio_endpoint_null_skipped");
                continue;
            }
            let device = IMMDevice::from_raw(raw);
            let manager: Manager = match device.Activate(CLSCTX_ALL, None) {
                Ok(manager) => manager,
                Err(error) => {
                    tracing::debug!(index, %error, "audio_endpoint_activate_skipped");
                    continue;
                }
            };
            let mut raw = std::ptr::null_mut();
            if let Err(error) = (manager.vtable().sessions)(manager.as_raw(), &mut raw).ok() {
                tracing::debug!(index, %error, "audio_endpoint_sessions_skipped");
                continue;
            }
            if raw.is_null() {
                tracing::debug!(index, "audio_endpoint_sessions_null_skipped");
                continue;
            }
            let sessions = Sessions::from_raw(raw);
            let mut n = 0;
            if let Err(error) = (sessions.vtable().count)(sessions.as_raw(), &mut n).ok() {
                tracing::debug!(index, %error, "audio_endpoint_session_count_skipped");
                continue;
            }
            if !(0..=512).contains(&n) {
                tracing::debug!(index, count = n, "audio_endpoint_session_count_out_of_range");
                continue;
            }
            for i in 0..n {
                let mut raw = std::ptr::null_mut();
                if let Err(error) = (sessions.vtable().item)(sessions.as_raw(), i, &mut raw).ok() {
                    tracing::debug!(index, session = i, %error, "audio_session_item_skipped");
                    continue;
                }
                if raw.is_null() {
                    tracing::debug!(index, session = i, "audio_session_null_skipped");
                    continue;
                }
                let unknown = windows_core::IUnknown::from_raw(raw);
                let control: Control = match unknown.cast() {
                    Ok(control) => control,
                    Err(error) => {
                        tracing::debug!(index, session = i, %error, "audio_session_cast_skipped");
                        continue;
                    }
                };
                let mut state = 0;
                if let Err(error) = (control.vtable().state)(control.as_raw(), &mut state).ok() {
                    tracing::debug!(index, session = i, %error, "audio_session_state_skipped");
                    continue;
                }
                if state != 1 {
                    continue;
                }
                let mut pid = 0;
                let hr = (control.vtable().pid)(control.as_raw(), &mut pid);
                if let Err(error) = hr.ok() {
                    tracing::debug!(index, session = i, %error, "audio_session_pid_skipped");
                    continue;
                }
                if hr.0 == 0 && pid != 0 {
                    pids.insert(pid);
                }
            }
        }
        Ok(pids)
    }
}
