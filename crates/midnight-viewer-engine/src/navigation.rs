//! File positions stay exact even when a scrollbar cannot represent every byte.
pub(crate) fn anchor(position: f64, length: u64) -> u64 {
    if !position.is_finite() {
        return 0;
    }
    (position.clamp(0.0, 1.0) * length.saturating_sub(1) as f64) as u64
}

pub(crate) fn viewport(position: f64, length: u64) -> (u64, bool) {
    if position >= 1.0 && length > 0 {
        (length, true)
    } else {
        (anchor(position, length), false)
    }
}

/// A scrollbar's thumb already accounts for the visible part of the file.
/// The exact pixel inverse is used when that range is already decoded.
pub(crate) fn scrollbar_viewport(position: f64, length: u64, extent: f64) -> (u64, bool) {
    if position >= 1.0 {
        return viewport(position, length);
    }
    let extent = if extent.is_finite() {
        extent.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let position = if position.is_finite() {
        position.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let anchor = (position * length as f64 * (1.0 - extent)) as u64;
    (anchor.min(length.saturating_sub(1)), false)
}
/// Provisional byte-based extent until a complete file line index exists.
pub(crate) fn scroll_metrics(
    start: u64,
    end: u64,
    length: u64,
    height: f32,
    viewport: f32,
    origin: f32,
) -> (f64, f64) {
    if length == 0 {
        return (0.0, 1.0);
    }
    let height = f64::from(height.max(1.0));
    let page_bytes = end.saturating_sub(start) as f64;
    let visible = page_bytes * (f64::from(viewport) / height).clamp(0.0, 1.0);
    let extent = (visible / length as f64).clamp(0.0, 1.0);
    let top = start as f64 + page_bytes * (f64::from(origin) / height).clamp(0.0, 1.0);
    let position = if extent >= 1.0 {
        0.0
    } else {
        (top / (length as f64 - visible).max(1.0)).clamp(0.0, 1.0)
    };
    (position, extent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_file_navigation_does_not_narrow_offsets_to_u32() {
        let length = (1_u64 << 45) + 47;
        assert!(anchor(0.5, length) > u64::from(u32::MAX));
        assert_eq!(anchor(1.0, length), length - 1);
        assert_eq!(anchor(f64::NAN, length), 0);
        assert_eq!(anchor(-1.0, length), 0);
    }

    #[test]
    fn empty_files_have_a_valid_zero_anchor() {
        assert_eq!(anchor(1.0, 0), 0);
        assert_eq!(scroll_metrics(0, 0, 0, 1.0, 1.0, 0.0), (0.0, 1.0));
    }

    #[test]
    fn bottom_seek_requests_a_page_ending_at_eof() {
        assert_eq!(viewport(1.0, 80_000), (80_000, true));
        assert_eq!(viewport(0.5, 80_000), (39_999, false));
        assert_eq!(viewport(1.0, 0), (0, false));
    }

    #[test]
    fn scroll_metrics_hide_short_content_and_reach_bottom_after_local_scrolling() {
        assert_eq!(scroll_metrics(0, 100, 100, 100.0, 200.0, 0.0), (0.0, 1.0));
        assert_eq!(
            scroll_metrics(800, 1000, 1000, 400.0, 200.0, 200.0),
            (1.0, 0.1)
        );
        let (position, extent) = scroll_metrics(0, 1000, 2000, 2000.0, 200.0, 200.0);
        assert!(position > 0.0);
        assert_eq!(extent, 0.05);
    }
}
