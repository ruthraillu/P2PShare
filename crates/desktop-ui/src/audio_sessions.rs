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
        let devices = Collection::from_raw(raw);
        let mut count = 0;
        (devices.vtable().count)(devices.as_raw(), &mut count).ok()?;
        anyhow::ensure!(count <= 64, "Too many audio endpoints");
        let mut pids = BTreeSet::new();
        for index in 0..count {
            let mut raw = std::ptr::null_mut();
            (devices.vtable().item)(devices.as_raw(), index, &mut raw).ok()?;
            let device = IMMDevice::from_raw(raw);
            let manager: Manager = device.Activate(CLSCTX_ALL, None)?;
            let mut raw = std::ptr::null_mut();
            (manager.vtable().sessions)(manager.as_raw(), &mut raw).ok()?;
            let sessions = Sessions::from_raw(raw);
            let mut n = 0;
            (sessions.vtable().count)(sessions.as_raw(), &mut n).ok()?;
            anyhow::ensure!((0..=512).contains(&n), "Too many audio sessions");
            for i in 0..n {
                let mut raw = std::ptr::null_mut();
                (sessions.vtable().item)(sessions.as_raw(), i, &mut raw).ok()?;
                let unknown = windows_core::IUnknown::from_raw(raw);
                let control: Control = unknown.cast()?;
                let mut state = 0;
                (control.vtable().state)(control.as_raw(), &mut state).ok()?;
                if state != 1 {
                    continue;
                }
                let mut pid = 0;
                let hr = (control.vtable().pid)(control.as_raw(), &mut pid);
                hr.ok()?;
                if hr.0 == 0 && pid != 0 {
                    pids.insert(pid);
                }
            }
        }
        Ok(pids)
    }
}
