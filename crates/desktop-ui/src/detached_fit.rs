//! Aspect-constrained native resizing for detached players. No rendering or media work.
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    UI::WindowsAndMessaging::WM_SIZING,
};
use winit::{dpi::PhysicalSize, window::Window};

type SubclassProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, callback: Option<SubclassProc>, id: usize, data: usize)
    -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, callback: Option<SubclassProc>, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}
const SUBCLASS_ID: usize = 0x50325046;
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    video: [u32; 2],
    footer: u32,
    extra: PhysicalSize<u32>,
    minimum: PhysicalSize<u32>,
}
impl Fit {
    fn by_width(self, width: u32) -> PhysicalSize<u32> {
        let ratio = self.video[0].max(1) as f64 / self.video[1].max(1) as f64;
        let width = width.max(self.minimum.width);
        let height = (width as f64 / ratio).round().max(1.) as u32 + self.footer;
        if height < self.minimum.height {
            return self.by_height(self.minimum.height);
        }
        PhysicalSize::new(width, height)
    }
    fn by_height(self, height: u32) -> PhysicalSize<u32> {
        let ratio = self.video[0].max(1) as f64 / self.video[1].max(1) as f64;
        let height = height.max(self.minimum.height).max(self.footer + 1);
        let width = ((height - self.footer) as f64 * ratio).round().max(1.) as u32;
        if width < self.minimum.width {
            // Avoid mutual recursion around rounding at the minimum boundary.
            return PhysicalSize::new(
                self.minimum.width,
                (self.minimum.width as f64 / ratio).round().max(1.) as u32 + self.footer,
            );
        }
        PhysicalSize::new(width, height)
    }
    fn constrain(self, edge: usize, bounds: &mut RECT) {
        let w = (bounds.right - bounds.left).max(1) as u32;
        let h = (bounds.bottom - bounds.top).max(1) as u32;
        let client = if edge == 3 || edge == 6 {
            self.by_height(h.saturating_sub(self.extra.height))
        } else {
            self.by_width(w.saturating_sub(self.extra.width))
        };
        let w = client.width.saturating_add(self.extra.width) as i32;
        let h = client.height.saturating_add(self.extra.height) as i32;
        if matches!(edge, 1 | 4 | 7) {
            bounds.left = bounds.right - w;
        } else {
            bounds.right = bounds.left + w;
        }
        if matches!(edge, 3..=5) {
            bounds.top = bounds.bottom - h;
        } else {
            bounds.bottom = bounds.top + h;
        }
    }
}
struct State {
    fit: Cell<Fit>,
    enabled: Cell<bool>,
}
// SAFETY: installed and removed on the HWND's UI thread. State has a stable Box
// allocation owned by Guard; removal precedes freeing it. WM_SIZING supplies a
// writable RECT for the duration of this synchronous callback.
unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_SIZING && data != 0 && lp.0 != 0 {
        let state = unsafe { &*(data as *const State) };
        if state.enabled.get() {
            state
                .fit
                .get()
                .constrain(wp.0, unsafe { &mut *(lp.0 as *mut RECT) });
            return LRESULT(1);
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}
pub(super) struct Guard {
    hwnd: HWND,
    state: Box<State>,
    was_suspended: bool,
}
impl Guard {
    pub fn install(window: &Window, video: [u32; 2]) -> anyhow::Result<Self> {
        let RawWindowHandle::Win32(h) = window.window_handle()?.as_raw() else {
            anyhow::bail!("Expected Win32 window");
        };
        let video = if video[0] > 0 && video[1] > 0 {
            video
        } else {
            [16, 9]
        };
        let scale = window.scale_factor();
        let footer = (44. * scale).round() as u32;
        let monitor = window
            .current_monitor()
            .or_else(|| window.primary_monitor());
        let max = monitor
            .map(|m| m.size())
            .unwrap_or(PhysicalSize::new(1280, 832));
        let margin = (80. * scale).round() as u32;
        let maximum = PhysicalSize::new(
            max.width.saturating_sub(margin).max(200),
            max.height.saturating_sub(margin).max(footer + 100),
        );
        let mut fit = Fit {
            video,
            footer,
            extra: PhysicalSize::new(0, 0),
            minimum: PhysicalSize::new(1, footer + 1),
        };
        let mut size = fit.by_width(window.inner_size().width.min(maximum.width));
        if size.height > maximum.height {
            size = fit.by_height(maximum.height);
        }
        fit.minimum = fit.by_width((500. * scale).round() as u32);
        if fit.minimum.width > size.width || fit.minimum.height > size.height {
            fit.minimum = size;
        }
        window.set_min_inner_size(Some(fit.minimum));
        let _ = window.request_inner_size(size);
        let inner = window.inner_size();
        let outer = window.outer_size();
        fit.extra = PhysicalSize::new(
            outer.width.saturating_sub(inner.width),
            outer.height.saturating_sub(inner.height),
        );
        let mut guard = Self {
            hwnd: HWND(h.hwnd.get() as *mut _),
            state: Box::new(State {
                fit: Cell::new(fit),
                enabled: Cell::new(true),
            }),
            was_suspended: false,
        };
        let data = (&mut *guard.state) as *mut State as usize;
        if unsafe { SetWindowSubclass(guard.hwnd, Some(subclass), SUBCLASS_ID, data) } == 0 {
            anyhow::bail!("Could not install detached window aspect constraint");
        }
        Ok(guard)
    }
    pub fn refresh(
        &mut self,
        window: &Window,
        video: [u32; 2],
        pixels_per_point: f32,
        fullscreen: bool,
    ) {
        if video[0] == 0 || video[1] == 0 {
            return;
        }
        let suspended = fullscreen
            || window.fullscreen().is_some()
            || window.is_maximized()
            || window.is_minimized() == Some(true);
        let mut fit = self.state.fit.get();
        let footer = (44. * pixels_per_point).round().max(1.) as u32;
        let changed = fit.video != video || fit.footer != footer;
        if changed {
            fit.video = video;
            fit.footer = footer;
            let monitor = window
                .current_monitor()
                .or_else(|| window.primary_monitor());
            let max = monitor
                .map(|m| m.size())
                .unwrap_or(PhysicalSize::new(1280, 832));
            let margin = (80. * window.scale_factor()).round() as u32;
            let maximum = PhysicalSize::new(
                max.width.saturating_sub(margin).max(200),
                max.height.saturating_sub(margin).max(footer + 100),
            );
            fit.minimum = PhysicalSize::new(1, footer + 1);
            let mut limit = fit.by_width(maximum.width);
            if limit.height > maximum.height {
                limit = fit.by_height(maximum.height);
            }
            fit.minimum = fit.by_width((500. * window.scale_factor()).round() as u32);
            if fit.minimum.width > limit.width || fit.minimum.height > limit.height {
                fit.minimum = limit;
            }
            window.set_min_inner_size(Some(fit.minimum));
        }
        let inner = window.inner_size();
        let outer = window.outer_size();
        fit.extra = PhysicalSize::new(
            outer.width.saturating_sub(inner.width),
            outer.height.saturating_sub(inner.height),
        );
        self.state.fit.set(fit);
        self.state.enabled.set(!suspended);
        if !suspended && (changed || self.was_suspended) && inner.width > 0 {
            let mut desired = fit.by_width(inner.width);
            if changed && let Some(monitor) = window.current_monitor() {
                let maximum = monitor.size();
                let margin = (80. * window.scale_factor()).round() as u32;
                desired = fit.by_width(desired.width.min(maximum.width.saturating_sub(margin)));
                if desired.height > maximum.height.saturating_sub(margin) {
                    desired = fit.by_height(maximum.height.saturating_sub(margin));
                }
            }
            if desired != inner {
                let _ = window.request_inner_size(desired);
            }
        }
        self.was_suspended = suspended;
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        // The window can already have been destroyed; failed removal is harmless.
        unsafe {
            RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS_ID);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_accounts_for_controls_dpi_minimum_and_all_resize_anchors() {
        for scale in [1., 1.5, 2.] {
            let fit = Fit {
                video: [1920, 1032],
                footer: (44. * scale) as u32,
                extra: PhysicalSize::new(0, 0),
                minimum: PhysicalSize::new(1, 1),
            };
            assert_eq!(
                fit.by_width(960),
                PhysicalSize::new(960, 516 + (44. * scale) as u32)
            );
            for edge in 1..=8 {
                let mut r = RECT {
                    left: -500,
                    top: 100,
                    right: 460,
                    bottom: 800,
                };
                fit.constrain(edge, &mut r);
                let w = (r.right - r.left) as f64;
                let h = (r.bottom - r.top) as f64 - fit.footer as f64;
                assert!((w - h * 1920. / 1032.).abs() < 2.);
                if matches!(edge, 1 | 4 | 7) {
                    assert_eq!(r.right, 460);
                } else {
                    assert_eq!(r.left, -500);
                }
                if matches!(edge, 3..=5) {
                    assert_eq!(r.bottom, 800);
                } else {
                    assert_eq!(r.top, 100);
                }
            }
        }
        let fit = Fit {
            video: [1080, 1920],
            footer: 44,
            extra: PhysicalSize::new(0, 0),
            minimum: PhysicalSize::new(500, 933),
        };
        assert_eq!(fit.by_width(200), PhysicalSize::new(500, 933));
    }
}
