//! Responsive rules, independent of platform capture and network code.
use egui::{Rect, pos2};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Desktop,
    Android,
}
pub(crate) struct Layout {
    pub(crate) mobile: bool,
    pub(crate) landscape: bool,
}
impl Layout {
    pub(crate) fn new(rect: Rect, platform: Platform) -> Self {
        let mobile = platform == Platform::Android || rect.width() < 760.0 || rect.height() < 520.0;
        Self {
            mobile,
            landscape: mobile && rect.width() > rect.height(),
        }
    }
}
pub(crate) fn safe_rect(rect: Rect, insets: [f32; 4]) -> Rect {
    Rect::from_min_max(
        pos2(
            rect.left() + insets[0].min(rect.width() * 0.25),
            rect.top() + insets[1].min(rect.height() * 0.25),
        ),
        pos2(
            rect.right() - insets[2].min(rect.width() * 0.25),
            rect.bottom() - insets[3].min(rect.height() * 0.25),
        ),
    )
}
