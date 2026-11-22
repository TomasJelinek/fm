//! How far one wheel notch moves content: the shared notch distance, scaled by the
//! system's wheel setting. Pure policy: each host reads the setting on its platform and
//! supplies it (the viewer: `midnight-viewer-ui/src/wheel_setting.rs`).
use crate::kinetic_native::LINE_PIXELS;

/// The system default that [`LINE_PIXELS`] stands for: Windows' "lines to scroll at a
/// time" starts at three.
const DEFAULT_LINES: u32 = 3;
/// Windows offers 1 to 100 lines per notch.
const MAX_LINES: u32 = 100;

/// A system's wheel setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelSetting {
    /// Lines per notch.
    Lines(u32),
    /// One screen per notch.
    Page,
}

/// Three lines per notch: Windows' default, and the setting of systems without one.
impl Default for WheelSetting {
    fn default() -> Self {
        Self::Lines(DEFAULT_LINES)
    }
}

/// Logical pixels one wheel notch (one winit line) moves content whose visible extent
/// along the notch's axis is `page` logical pixels (0 when unknown): [`LINE_PIXELS`] at
/// the default three lines, in proportion to other line counts, and `page` for one
/// screen per notch ([`LINE_PIXELS`] when the page is unknown).
pub fn notch_pixels_for(setting: WheelSetting, page: f32) -> f32 {
    match setting {
        WheelSetting::Lines(lines) => {
            LINE_PIXELS * lines.clamp(1, MAX_LINES) as f32 / DEFAULT_LINES as f32
        }
        WheelSetting::Page if page.is_finite() && page >= 1.0 => page,
        WheelSetting::Page => LINE_PIXELS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notch_moves_the_shared_distance_at_the_default_and_scales_with_the_setting() {
        assert_eq!(
            notch_pixels_for(WheelSetting::default(), 700.0),
            LINE_PIXELS
        );
        assert_eq!(notch_pixels_for(WheelSetting::Lines(3), 700.0), LINE_PIXELS);
        assert_eq!(
            notch_pixels_for(WheelSetting::Lines(1), 700.0),
            LINE_PIXELS / 3.0
        );
        assert_eq!(
            notch_pixels_for(WheelSetting::Lines(6), 700.0),
            LINE_PIXELS * 2.0
        );
        // No setting stops the wheel or makes one notch unbounded.
        assert_eq!(
            notch_pixels_for(WheelSetting::Lines(0), 700.0),
            LINE_PIXELS / 3.0
        );
        assert_eq!(
            notch_pixels_for(WheelSetting::Lines(u32::MAX), 700.0),
            LINE_PIXELS * 100.0 / 3.0
        );
    }

    #[test]
    fn one_screen_per_notch_moves_the_visible_extent() {
        assert_eq!(notch_pixels_for(WheelSetting::Page, 700.0), 700.0);
        for page in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            assert_eq!(notch_pixels_for(WheelSetting::Page, page), LINE_PIXELS);
        }
    }
}
