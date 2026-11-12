//! Source-pixel image navigation, independent of native paths and UI scaling.

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImageGeometry {
    pub width: u32,
    pub height: u32,
    pub viewport: [u32; 2],
    pub scale: f64,
    pub origin: [f64; 2],
    pub fitted: bool,
    fit_upscale: bool,
}

impl ImageGeometry {
    pub fn zoom_from(
        &mut self,
        zoom: midnight_core::ViewerZoomPreference,
        source: [f64; 2],
        anchor: [f32; 2],
    ) -> bool {
        if source.iter().any(|value| !value.is_finite())
            || anchor.iter().any(|value| !value.is_finite())
        {
            return false;
        }
        match zoom {
            midnight_core::ViewerZoomPreference::Fit => self.fit(),
            _ => {
                let Some(scale) = zoom
                    .scale()
                    .filter(|scale| (1.0 / 32768.0..=64.0).contains(scale))
                else {
                    return false;
                };
                let minimum = 1.0 / (32768.0 * self.raster_geometry().1);
                self.scale = scale.clamp(minimum.min(64.0), 64.0);
                for axis in 0..2 {
                    self.origin[axis] = source[axis]
                        - f64::from(anchor[axis]).clamp(0.0, f64::from(self.viewport[axis]))
                            / self.scale;
                }
                self.fitted = false;
                self.clamp_origin();
            }
        }
        true
    }
    pub fn restore_zoom(&mut self, preference: midnight_core::ViewerZoomPreference) {
        match preference {
            midnight_core::ViewerZoomPreference::Fit => self.fit(),
            midnight_core::ViewerZoomPreference::ActualPixels => self.actual(),
            midnight_core::ViewerZoomPreference::ScaleBasisPoints(value) => {
                self.zoom(f64::from(value) / 10_000.0 / self.scale, self.center())
            }
        }
    }

    pub fn new(width: u32, height: u32, viewport: [u32; 2]) -> Self {
        let mut state = Self {
            width: width.max(1),
            height: height.max(1),
            viewport: viewport.map(|v| v.max(1)),
            scale: 1.0,
            origin: [0.0; 2],
            fitted: true,
            fit_upscale: false,
        };
        state.fit();
        state
    }

    pub fn document(width: u32, height: u32, viewport: [u32; 2]) -> Self {
        let mut state = Self::new(width, height, viewport);
        state.fit_upscale = true;
        state.fit();
        state
    }

    pub fn fit(&mut self) {
        self.scale = (f64::from(self.viewport[0]) / f64::from(self.width))
            .min(f64::from(self.viewport[1]) / f64::from(self.height))
            .min(if self.fit_upscale { 64.0 } else { 1.0 });
        self.origin = [0.0; 2];
        self.fitted = true;
        self.clamp_origin();
    }

    pub fn actual(&mut self) {
        self.zoom(1.0 / self.scale, self.center());
    }

    /// Start another document page without replacing the user's zoom choice.
    /// Fit adapts to the page; manual scales retain their full runtime precision.
    pub fn change_page(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.origin = [0.0; 2];
        if self.fitted {
            self.fit();
        } else {
            self.clamp_origin();
        }
    }

    pub fn resize(&mut self, viewport: [u32; 2]) {
        let before = self.center();
        self.viewport = viewport.map(|v| v.max(1));
        if self.fitted {
            self.fit();
        } else {
            let after = self.center();
            for axis in 0..2 {
                self.origin[axis] += (before[axis] - after[axis]) / self.scale;
            }
            self.clamp_origin();
        }
    }

    pub fn zoom(&mut self, factor: f64, anchor: [f64; 2]) {
        if !factor.is_finite() || factor <= 0.0 || anchor.iter().any(|v| !v.is_finite()) {
            return;
        }
        let minimum = 1.0 / (32768.0 * self.raster_geometry().1);
        let next = (self.scale * factor).clamp(minimum.min(64.0), 64.0);
        for (axis, coordinate) in anchor.into_iter().enumerate() {
            let coordinate = coordinate.clamp(0.0, f64::from(self.viewport[axis]));
            self.origin[axis] += coordinate / self.scale - coordinate / next;
        }
        self.scale = next;
        self.fitted = false;
        self.clamp_origin();
    }

    pub fn scroll(&mut self, delta: [f64; 2]) {
        for (axis, delta) in delta.into_iter().enumerate() {
            if delta.is_finite() {
                self.origin[axis] += delta / self.scale;
            }
        }
        self.clamp_origin();
    }

    pub fn pan_to(&mut self, origin: [f64; 2]) {
        if origin.iter().all(|value| value.is_finite()) {
            self.origin = origin;
            self.clamp_origin();
        }
    }

    pub fn seek(&mut self, axis: usize, position: f64) {
        if axis >= 2 || !position.is_finite() {
            return;
        }
        self.origin[axis] = position.clamp(0.0, 1.0) * self.maximum(axis).max(0.0);
        self.clamp_origin();
    }

    pub fn metrics(&self, axis: usize) -> (f64, f64) {
        if axis >= 2 {
            return (0.0, 1.0);
        }
        let maximum = self.maximum(axis);
        if maximum <= 0.0 {
            return (0.0, 1.0);
        }
        (
            (self.origin[axis] / maximum).clamp(0.0, 1.0),
            (f64::from(self.viewport[axis]) / (self.dimension(axis) * self.scale)).clamp(0.0, 1.0),
        )
    }

    pub fn center(&self) -> [f64; 2] {
        self.viewport.map(|value| f64::from(value) / 2.0)
    }

    pub fn raster_geometry(&self) -> ([u32; 2], f64) {
        let [width, height] = self.viewport.map(f64::from);
        let density = (4096.0 / width)
            .min(4096.0 / height)
            .min((4_194_304.0 / (width * height)).sqrt())
            .min(1.0);
        (
            self.viewport
                .map(|dimension| (f64::from(dimension) * density).max(1.0) as u32),
            density,
        )
    }

    fn dimension(&self, axis: usize) -> f64 {
        f64::from(if axis == 0 { self.width } else { self.height })
    }

    fn maximum(&self, axis: usize) -> f64 {
        self.dimension(axis) - f64::from(self.viewport[axis]) / self.scale
    }

    fn clamp_origin(&mut self) {
        for axis in 0..2 {
            let maximum = self.maximum(axis);
            self.origin[axis] = if maximum <= 0.0 {
                maximum / 2.0
            } else {
                self.origin[axis].clamp(0.0, maximum)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.00001,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn actual_pixels_remain_physical_pixels_when_the_output_surface_is_bounded() {
        let mut image = ImageGeometry::new(8000, 6000, [3840, 2160]);
        image.actual();
        let (surface, density) = image.raster_geometry();
        assert!(u64::from(surface[0]) * u64::from(surface[1]) <= 4 * 1024 * 1024);
        assert!(surface[0] <= 4096 && surface[1] <= 4096);
        assert!(density < 1.0);
        assert!((image.scale * density * 3840.0 / f64::from(surface[0]) - 1.0).abs() < 0.001);
        assert!((image.scale * density * 2160.0 / f64::from(surface[1]) - 1.0).abs() < 0.001);
    }

    #[test]
    fn fit_preserves_aspect_and_centers_without_upscaling_small_images() {
        let image = ImageGeometry::new(4000, 1000, [1000, 1000]);
        near(image.scale, 0.25);
        assert_eq!(image.origin, [0.0, -1500.0]);
        assert_eq!(image.metrics(0), (0.0, 1.0));
        let small = ImageGeometry::new(100, 60, [400, 300]);
        assert_eq!(small.scale, 1.0);
        assert_eq!(small.origin, [-150.0, -120.0]);
    }

    #[test]
    fn actual_pixels_preserves_center_and_scrollbars_reach_both_ends() {
        let mut image = ImageGeometry::new(4000, 1000, [1000, 1000]);
        image.actual();
        assert_eq!(image.scale, 1.0);
        assert_eq!(image.origin, [1500.0, 0.0]);
        assert_eq!(image.metrics(0), (0.5, 0.25));
        image.scroll([100000.0, -100000.0]);
        assert_eq!(image.origin, [3000.0, 0.0]);
        image.seek(0, 0.0);
        assert_eq!(image.origin[0], 0.0);
        image.seek(0, 1.0);
        assert_eq!(image.metrics(0), (1.0, 0.25));
    }

    #[test]
    fn zoom_keeps_the_source_pixel_under_the_pointer() {
        let mut image = ImageGeometry::new(3000, 2000, [600, 400]);
        image.actual();
        let anchor = [123.0, 76.0];
        let before = [
            image.origin[0] + anchor[0] / image.scale,
            image.origin[1] + anchor[1] / image.scale,
        ];
        image.zoom(1.25, anchor);
        for axis in 0..2 {
            near(
                image.origin[axis] + anchor[axis] / image.scale,
                before[axis],
            );
        }
        image.zoom(0.8, anchor);
        near(image.scale, 1.0);
    }

    #[test]
    fn resize_refits_or_preserves_manual_center_as_appropriate() {
        let mut image = ImageGeometry::new(4000, 1000, [1000, 1000]);
        image.resize([500, 500]);
        near(image.scale, 0.125);
        image.actual();
        image.resize([800, 600]);
        assert_eq!(image.scale, 1.0);
        assert_eq!(image.origin, [1600.0, 200.0]);
        image.fit();
        near(image.scale, 0.2);
        assert!(image.fitted);
    }

    #[test]
    fn page_changes_preserve_exact_manual_scale_and_reset_scroll_with_valid_bounds() {
        let mut page = ImageGeometry::document(400, 600, [160, 120]);
        page.actual();
        page.zoom(1.234567, page.center());
        let scale = page.scale;
        for (width, height) in [(1000, 400), (40, 60), (400, 600)] {
            page.scroll([100_000.0; 2]);
            page.change_page(width, height);
            assert_eq!(page.scale, scale);
            assert!(!page.fitted);
            assert_eq!(page.viewport, [160, 120]);
            assert_eq!((page.width, page.height), (width, height));
            for axis in 0..2 {
                near(page.origin[axis], (page.maximum(axis) / 2.0).min(0.0));
                assert_eq!(page.metrics(axis).0, 0.0);
            }
        }
        page.actual();
        page.change_page(600, 800);
        assert_eq!(page.scale, 1.0);
        assert!(!page.fitted);
        page.resize([200, 100]);
        assert_eq!(page.scale, 1.0);
    }

    #[test]
    fn page_changes_keep_fit_mode_and_adapt_to_different_page_dimensions() {
        let mut page = ImageGeometry::document(400, 600, [160, 120]);
        for (width, height, scale) in [(1000, 400, 0.16), (400, 600, 0.2), (40, 60, 2.0)] {
            page.change_page(width, height);
            assert!(page.fitted);
            near(page.scale, scale);
            assert_eq!(page.metrics(0), (0.0, 1.0));
            assert_eq!(page.metrics(1), (0.0, 1.0));
        }
    }

    #[test]
    fn invalid_inputs_do_not_poison_state_and_zoom_is_bounded() {
        let mut image = ImageGeometry::new(32768, 32768, [1, 1]);
        near(image.scale, 1.0 / 32768.0);
        image.zoom(f64::NAN, [0.0, 0.0]);
        image.zoom(-1.0, [0.0, 0.0]);
        image.scroll([f64::INFINITY, f64::NAN]);
        image.seek(1, f64::NAN);
        assert!(image.origin.iter().all(|value| value.is_finite()));
        image.zoom(f64::MAX, [0.0, 0.0]);
        assert!(image.scale <= 64.0);
        image.seek(50, 1.0);
        assert_eq!(image.metrics(50), (0.0, 1.0));
    }
}
