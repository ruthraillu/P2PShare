//! Portable normal/maximized placement. Fullscreen and minimized bounds are never persisted.
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::*,
    },
    core::Result,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    maximized: bool,
}
impl Placement {
    fn valid(&self) -> bool {
        (-1_000_000..=1_000_000).contains(&self.x)
            && (-1_000_000..=1_000_000).contains(&self.y)
            && (100..=32768).contains(&self.width)
            && (100..=32768).contains(&self.height)
    }
    fn native(self) -> WINDOWPLACEMENT {
        WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            showCmd: if self.maximized {
                SW_SHOWMAXIMIZED.0 as u32
            } else {
                SW_SHOWNORMAL.0 as u32
            },
            rcNormalPosition: RECT {
                left: self.x,
                top: self.y,
                right: self.x + self.width,
                bottom: self.y + self.height,
            },
            ..Default::default()
        }
    }
    fn document(self) -> String {
        format!(
            "# P2PShare window state. Saved automatically beside the executable.\n# Normal outer left position in Windows workspace coordinates.\nx = {}\n# Normal outer top position in Windows workspace coordinates.\ny = {}\n# Normal outer window width in pixels.\nwidth = {}\n# Normal outer window height in pixels.\nheight = {}\n# Reopen maximized while retaining the normal restore bounds.\nmaximized = {}\n",
            self.x, self.y, self.width, self.height, self.maximized
        )
    }
}
pub struct WindowMemory {
    path: PathBuf,
    saved: Option<Placement>,
    enabled: bool,
    dirty: bool,
    changed: Instant,
}
impl WindowMemory {
    pub fn new(path: PathBuf, enabled: bool) -> Self {
        let saved = if enabled {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let parsed = if text.len() <= 4096 {
                        toml::from_str::<Placement>(&text)
                            .ok()
                            .filter(Placement::valid)
                    } else {
                        None
                    };
                    if parsed.is_none() {
                        tracing::warn!("invalid_window_state_ignored");
                    }
                    parsed
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(%e,"window_state_read_failed");
                    }
                    None
                }
            }
        } else {
            None
        };
        Self {
            path,
            saved,
            enabled,
            dirty: false,
            changed: Instant::now(),
        }
    }
    /// Restore normal bounds without showing the startup window until painting is ready.
    pub fn restore(&self, hwnd: HWND, show: bool) -> Result<()> {
        if let Some(saved) = self.saved {
            // Matching Get/SetWindowPlacement avoids taskbar-coordinate drift. Windows
            // relocates completely offscreen bounds when the monitor arrangement changes.
            unsafe {
                let mut placement = WINDOWPLACEMENT {
                    length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                    ..Default::default()
                };
                GetWindowPlacement(hwnd, &mut placement)?;
                let target = saved.native();
                placement.rcNormalPosition = target.rcNormalPosition;
                placement.showCmd = if show {
                    target.showCmd
                } else {
                    SW_HIDE.0 as u32
                };
                placement.flags = WINDOWPLACEMENT_FLAGS(0);
                SetWindowPlacement(hwnd, &placement)?;
            }
        }
        Ok(())
    }
    pub fn capture(&mut self, hwnd: HWND, fullscreen: bool) {
        if !self.enabled || fullscreen {
            return;
        }
        unsafe {
            if IsIconic(hwnd).as_bool() {
                return;
            }
            let mut native = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            if let Err(e) = GetWindowPlacement(hwnd, &mut native) {
                tracing::debug!(%e,"window_state_capture_failed");
                return;
            }
            let r = native.rcNormalPosition;
            let next = Placement {
                x: r.left,
                y: r.top,
                width: r.right.saturating_sub(r.left),
                height: r.bottom.saturating_sub(r.top),
                maximized: native.showCmd == SW_SHOWMAXIMIZED.0 as u32,
            };
            if next.valid() && self.saved != Some(next) {
                self.saved = Some(next);
                self.dirty = true;
                self.changed = Instant::now();
            }
        }
    }
    pub fn flush(&mut self, force: bool) {
        if !self.dirty || (!force && self.changed.elapsed() < Duration::from_millis(600)) {
            return;
        }
        if let Some(saved) = self.saved {
            let pending = self.path.with_extension("toml.pending");
            match std::fs::write(&pending, saved.document())
                .and_then(|()| std::fs::rename(&pending, &self.path))
            {
                Ok(()) => self.dirty = false,
                Err(e) => {
                    tracing::warn!(%e,"window_state_save_failed");
                    self.changed = Instant::now();
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placement_roundtrip_keeps_negative_monitor_coordinates_and_normal_bounds_when_maximized() {
        let p = Placement {
            x: -1800,
            y: 45,
            width: 1300,
            height: 850,
            maximized: true,
        };
        let loaded: Placement = toml::from_str(&p.document()).unwrap();
        assert_eq!(loaded, p);
        assert!(loaded.valid());
        let native = p.native();
        assert_eq!(native.showCmd, SW_SHOWMAXIMIZED.0 as u32);
        assert_eq!(native.rcNormalPosition.right, -500);
        assert_eq!(native.rcNormalPosition.bottom, 895);
        assert!(!Placement { width: 0, ..p }.valid());
        assert!(!Placement { x: i32::MAX, ..p }.valid());
    }
    #[test]
    fn restoring_startup_bounds_does_not_reveal_a_normal_or_maximized_window() -> Result<()> {
        use windows::core::w;
        // A hidden owned HWND only: never shown, never captures the user's desktop.
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Hidden startup placement test"),
                WS_OVERLAPPEDWINDOW,
                40,
                70,
                900,
                652,
                None,
                None,
                None,
                None,
            )?;
            let result = (|| -> Result<()> {
                let mut memory = WindowMemory::new(PathBuf::new(), false);
                for maximized in [false, true] {
                    let saved = Placement {
                        x: 40,
                        y: 70,
                        width: 1000,
                        height: 700,
                        maximized,
                    };
                    memory.saved = Some(saved);
                    memory.restore(hwnd, false)?;
                    assert!(
                        !IsWindowVisible(hwnd).as_bool(),
                        "Placement restore exposed an unpainted window"
                    );
                    assert_eq!(
                        memory.saved,
                        Some(saved),
                        "Startup lost the requested maximized state"
                    );
                }
                Ok(())
            })();
            DestroyWindow(hwnd)?;
            result
        }
    }
    #[test]
    fn portable_state_file_persists_and_invalid_file_is_ignored() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/window-state-tests")
            .join(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
                    .to_string(),
            )
            .join("window-state.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut memory = WindowMemory::new(path.clone(), true);
        let state = Placement {
            x: 40,
            y: 70,
            width: 1280,
            height: 800,
            maximized: false,
        };
        memory.saved = Some(state);
        memory.dirty = true;
        memory.flush(true);
        assert_eq!(WindowMemory::new(path.clone(), true).saved, Some(state));
        assert!(WindowMemory::new(path.clone(), false).saved.is_none());
        std::fs::write(&path, "bad file").unwrap();
        assert!(WindowMemory::new(path, true).saved.is_none());
    }
}
