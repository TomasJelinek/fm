//! App-internal retained text surfaces; no public plugin ABI is involved.
use super::ViewerSurfaceId;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum ViewerRasterInput {
    /// Latest absolute camera in continuous effective raster pixels, independent of page rebasing.
    PanTo {
        sequence: u64,
        origin: [f64; 2],
        /// Request exact final glyph phase rather than interpolated retained pixels.
        settled: bool,
    },
    /// Coordinates belong to the displayed camera, which may be ahead of the worker.
    Select {
        pixel_id: u64,
        origin: [f64; 2],
        x: f32,
        y: f32,
        extend: bool,
    },
    SelectPage {
        pixel_id: u64,
        origin: [f64; 2],
    },
    Copy,
}

#[derive(Clone, Debug)]
pub struct ViewerRasterGutter {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA; shares the body raster's Y origin and immutable pixel ID.
    pub pixels: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub struct ViewerRasterInfo {
    /// Stable through exact overlapping page continuation; replaced on layout/navigation resets.
    pub surface: ViewerSurfaceId,
    /// Immutable body/gutter allocation identity. Metadata-only frames reuse this ID and pixels.
    pub pixel_id: u64,
    /// Last accepted absolute camera, in continuous effective raster pixels.
    pub origin: [f64; 2],
    /// Body pixel (0,0) in that coordinate space. Gutter shares its Y coordinate.
    pub raster_origin: [f64; 2],
    /// Whole visible viewport, including the fixed gutter, in effective raster pixels.
    /// The existing pixel cap can make this smaller than actual native window pixels.
    pub viewport: [u32; 2],
    /// Inclusive safe camera range covered by the retained pixels.
    pub coverage_min: [f64; 2],
    pub coverage_max: [f64; 2],
    /// Known content edges, independent from loaded coverage. None means not yet known.
    pub limit_min: [Option<f64>; 2],
    pub limit_max: [Option<f64>; 2],
    /// Only the matching surface's successfully prepared camera acknowledges a sequence.
    pub pan_sequence: u64,
    /// Normalized horizontal/vertical scroll fraction per effective raster pixel at `origin`.
    pub scroll_per_pixel: [f64; 2],
    /// Exact bounded visual/source map for vertical presentation and its local inverse.
    pub scroll_map: Option<super::ViewerScrollMap>,
    /// Pixels are exact at `origin`, including fractional glyph phase.
    pub settled: bool,
    /// Pinned horizontally; the body starts at this gutter's width in the viewport.
    pub gutter: Option<ViewerRasterGutter>,
    /// Prepared text: the pixels as contiguous strips instead of one body raster. The
    /// frame's own pixels and `gutter` are then empty.
    pub bands: Option<ViewerRasterBands>,
}

/// Prepared retained text, several screens around the camera: contiguous full-width
/// strips, top to bottom, that the window moves without new pixels while the worker
/// prepares more ahead of it. A strip's ID names its immutable pixels.
#[derive(Clone, Debug)]
pub struct ViewerRasterBands {
    /// Body strip width in effective raster pixels, starting at world x `left`.
    pub width: u32,
    pub left: f64,
    /// Pinned gutter width; zero without line numbers.
    pub gutter_width: u32,
    pub list: Arc<[ViewerRasterBand]>,
}

#[derive(Clone, Debug)]
pub struct ViewerRasterBand {
    pub id: u64,
    /// World y of the strip's first row: whole effective raster pixels.
    pub top: f64,
    pub height: u32,
    /// Premultiplied RGBA, `width` × `height`.
    pub pixels: Arc<[u8]>,
    /// Premultiplied RGBA of the gutter's same rows, `gutter_width` × `height`.
    pub gutter: Option<Arc<[u8]>>,
}

impl ViewerRasterBands {
    /// Pixel bytes of every strip, body and gutter.
    pub fn bytes(&self) -> u64 {
        self.list
            .iter()
            .map(|band| {
                band.pixels.len() as u64 + band.gutter.as_ref().map_or(0, |g| g.len() as u64)
            })
            .sum()
    }

    /// Dimensions, byte lengths and contiguity of the strips, within `budget` bytes.
    pub fn validate(&self, budget: u64) -> Result<(), &'static str> {
        let body = u64::from(self.width) * 4;
        let gutter = u64::from(self.gutter_width) * 4;
        let mut next = None;
        for band in self.list.iter() {
            let rows = u64::from(band.height);
            if band.id == 0
                || band.height == 0
                || !band.top.is_finite()
                || band.top.fract() != 0.0
                || band.pixels.len() as u64 != body * rows
                || band.gutter.as_ref().map_or(0, |g| g.len() as u64)
                    != if self.gutter_width == 0 {
                        0
                    } else {
                        gutter * rows
                    }
                || next.is_some_and(|top| top != band.top)
            {
                return Err("Invalid prepared text strip.");
            }
            next = Some(band.top + f64::from(band.height));
        }
        if self.width == 0 || self.list.is_empty() || !self.left.is_finite() {
            return Err("Invalid prepared text strips.");
        }
        if self.bytes() > budget {
            return Err("Prepared text exceeds its pixel budget.");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(id: u64, top: f64, height: u32, width: u32, gutter: u32) -> ViewerRasterBand {
        ViewerRasterBand {
            id,
            top,
            height,
            pixels: vec![0; (width * height * 4) as usize].into(),
            gutter: (gutter > 0).then(|| vec![0; (gutter * height * 4) as usize].into()),
        }
    }

    fn bands(list: Vec<ViewerRasterBand>, gutter: u32) -> ViewerRasterBands {
        ViewerRasterBands {
            width: 4,
            left: 0.0,
            gutter_width: gutter,
            list: list.into(),
        }
    }

    #[test]
    fn contiguous_whole_pixel_strips_within_the_budget_are_valid() {
        let valid = bands(
            vec![band(1, 256.0, 256, 4, 2), band(2, 512.0, 256, 4, 2)],
            2,
        );
        assert_eq!(valid.bytes(), 2 * (4 + 2) * 256 * 4);
        assert!(valid.validate(valid.bytes()).is_ok());
        assert!(valid.validate(valid.bytes() - 1).is_err(), "over budget");
    }

    #[test]
    fn gaps_fractions_wrong_lengths_and_empty_lists_are_refused() {
        let cases = [
            bands(vec![band(1, 0.0, 256, 4, 0), band(2, 512.0, 256, 4, 0)], 0),
            bands(vec![band(1, 0.5, 256, 4, 0)], 0),
            bands(vec![band(1, 0.0, 256, 3, 0)], 0),
            bands(vec![band(1, 0.0, 256, 4, 0)], 2),
            bands(vec![band(0, 0.0, 256, 4, 0)], 0),
            bands(vec![], 0),
        ];
        for (index, case) in cases.iter().enumerate() {
            assert!(case.validate(u64::MAX).is_err(), "case {index}");
        }
    }
}
