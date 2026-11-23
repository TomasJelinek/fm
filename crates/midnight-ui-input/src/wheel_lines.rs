//! How far one wheel notch moves content: Chromium's distance for the system's wheel
//! setting. Pure policy: each host reads the setting on its platform and supplies it (the
//! viewer: `midnight-viewer-ui/src/wheel_setting.rs`).
//!
//! Chromium (`ui/events/blink/web_input_event_builders_win.cc`) scrolls 100/3 DIP per line
//! on Windows times "lines to scroll at a time" (`SPI_GETWHEELSCROLLLINES`), and one page,
//! 87.5 % of the visible extent (`cc/input/scroll_utils.h`), when Windows scrolls a screen
//! at a time. On Linux it scrolls a fixed 120 DIP per notch on X11 and Wayland alike
//! (`ui/events/x/events_x_utils.cc` `kWheelScrollAmount`, Wayland's `value120`); there is
//! no system setting there.
use crate::kinetic_native::LINE_PIXELS;

/// The system default that [`LINE_PIXELS`] stands for: Windows' "lines to scroll at a
/// time" starts at three.
const DEFAULT_LINES: u32 = 3;
/// Windows offers 1 to 100 lines per notch.
const MAX_LINES: u32 = 100;
/// Chromium's notch where the system has no lines-per-notch setting (Linux).
pub const FIXED_NOTCH_PIXELS: f32 = 120.0;
/// The share of the visible extent that a page scrolls (`kMinFractionToStepWhenPaging`).
const PAGE_FRACTION: f32 = 0.875;

/// A system's wheel setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelSetting {
    /// Lines per notch (Windows).
    Lines(u32),
    /// One screen per notch (Windows).
    Page,
    /// No setting: Chromium's fixed notch (Linux, [`FIXED_NOTCH_PIXELS`]).
    Fixed,
}

/// Windows' default of three lines per notch; elsewhere, the fixed notch of a system
/// without a setting.
impl Default for WheelSetting {
    fn default() -> Self {
        if cfg!(windows) {
            Self::Lines(DEFAULT_LINES)
        } else {
            Self::Fixed
        }
    }
}

/// Logical pixels one wheel notch (one winit line) moves content whose visible extent
/// along the notch's axis is `page` logical pixels (0 when unknown), as Chromium moves it:
/// [`LINE_PIXELS`] (100) at three lines, in proportion to other line counts, 87.5 % of
/// `page` for one screen per notch ([`LINE_PIXELS`] when the page is unknown), and
/// [`FIXED_NOTCH_PIXELS`] (120) without a setting.
pub fn notch_pixels_for(setting: WheelSetting, page: f32) -> f32 {
    match setting {
        WheelSetting::Lines(lines) => {
            LINE_PIXELS * lines.clamp(1, MAX_LINES) as f32 / DEFAULT_LINES as f32
        }
        WheelSetting::Page if page.is_finite() && page >= 1.0 => (page * PAGE_FRACTION).max(1.0),
        WheelSetting::Page => LINE_PIXELS,
        WheelSetting::Fixed => FIXED_NOTCH_PIXELS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notch_moves_chromiums_distance_for_the_windows_lines_setting() {
        // 100/3 px per line: 33.3 at one line, 100 at Windows' default three, 200 at six.
        assert_eq!(notch_pixels_for(WheelSetting::Lines(3), 700.0), 100.0);
        assert!((notch_pixels_for(WheelSetting::Lines(1), 700.0) - 100.0 / 3.0).abs() < 1e-4);
        assert_eq!(notch_pixels_for(WheelSetting::Lines(6), 700.0), 200.0);
        assert_eq!(LINE_PIXELS, 100.0);
        // No setting stops the wheel or makes one notch unbounded.
        assert!((notch_pixels_for(WheelSetting::Lines(0), 700.0) - 100.0 / 3.0).abs() < 1e-4);
        assert!(
            (notch_pixels_for(WheelSetting::Lines(u32::MAX), 700.0) - 100.0 * 100.0 / 3.0).abs()
                < 1e-2
        );
    }

    #[test]
    fn one_screen_per_notch_moves_chromiums_page_step() {
        assert_eq!(notch_pixels_for(WheelSetting::Page, 800.0), 700.0);
        assert_eq!(notch_pixels_for(WheelSetting::Page, 1.0), 1.0);
        for page in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            assert_eq!(notch_pixels_for(WheelSetting::Page, page), LINE_PIXELS);
        }
    }

    #[test]
    fn linux_moves_chromiums_fixed_notch_and_windows_defaults_to_three_lines() {
        assert_eq!(notch_pixels_for(WheelSetting::Fixed, 700.0), 120.0);
        let expected = if cfg!(windows) { 100.0 } else { 120.0 };
        assert_eq!(notch_pixels_for(WheelSetting::default(), 700.0), expected);
    }
}
