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
    /// Pixels are exact at `origin`, including fractional glyph phase.
    pub settled: bool,
    /// Pinned horizontally; the body starts at this gutter's width in the viewport.
    pub gutter: Option<ViewerRasterGutter>,
}
