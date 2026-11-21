//! Page-local rendering of one band of document tiles: a rectangle of whole tiles of the
//! canvas at one scale. Each page the band meets is rendered once, by the plugin, at exactly
//! the band's scale; its pixels go straight into the band's missing tiles.
use crate::{
    document_view::DocumentView,
    image_scene::Band,
    image_tiles::{Charge, EDGE, Key, TILE_BYTES},
};
use midnight_core::{DocumentLayout, ViewerPixels};
use midnight_image_viewer_host::{Frame, ImageSession, Viewport};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_EDGE: u32 = 4096;
const MAX_PIXELS: u64 = 4_194_304;

/// Page slices the provider was asked to render in this process: a measurement counter
/// (`pdf_scroll_probe_tests`), never a limit.
pub(crate) static PAGE_RENDERS: AtomicU64 = AtomicU64::new(0);

pub(crate) struct PageSlice {
    pub page: usize,
    /// Top-left pixel of the slice in the band.
    pub destination: [u32; 2],
    pub viewport: Viewport,
}

/// The page slices of one band and the tiles they fill.
pub(crate) struct BandPlan {
    /// Band size in pixels.
    pub size: [u32; 2],
    /// Top-left pixel of the band in canvas raster coordinates.
    pub origin: [i64; 2],
    pub slices: Vec<PageSlice>,
    pub tiles: Vec<(Key, Arc<Charge>)>,
}

impl BandPlan {
    pub fn new(document: &DocumentView, band: Band) -> Result<Self, String> {
        let scale = band.scale;
        if !scale.is_finite() || !(1.0 / 32768.0..=64.0).contains(&scale) {
            return Err("Document render scale exceeds its limit.".into());
        }
        let [x0, y0, x1, y1] = band.rect;
        let edge = i64::from(EDGE);
        let origin = [i64::from(x0) * edge, i64::from(y0) * edge];
        let size = [(x1 - x0) as u32 * EDGE, (y1 - y0) as u32 * EDGE];
        raster_length(size)?;
        let paged = document.layout == DocumentLayout::Paged;
        let top = origin[1] as f64 / scale;
        let bottom = (origin[1] as f64 + f64::from(size[1])) / scale;
        let range = if paged {
            document.page..document.page + 1
        } else {
            let first = document
                .placements
                .partition_point(|page| page.bottom() <= top);
            let end = document
                .placements
                .partition_point(|page| page.origin[1] < bottom);
            first..end.max(first)
        };
        let mut slices = Vec::new();
        for page in range {
            let placement = document.placements[page];
            let page_origin = if paged { [0.0; 2] } else { placement.origin };
            let low: [u32; 2] = std::array::from_fn(|axis| {
                (page_origin[axis] * scale - origin[axis] as f64)
                    .floor()
                    .clamp(0.0, f64::from(size[axis])) as u32
            });
            let high: [u32; 2] = std::array::from_fn(|axis| {
                ((page_origin[axis] + f64::from(placement.size[axis])) * scale
                    - origin[axis] as f64)
                    .ceil()
                    .clamp(0.0, f64::from(size[axis])) as u32
            });
            if high[0] <= low[0] || high[1] <= low[1] {
                continue;
            }
            let viewport = Viewport {
                width: high[0] - low[0],
                height: high[1] - low[1],
                scale,
                origin_x: (origin[0] as f64 + f64::from(low[0])) / scale - page_origin[0],
                origin_y: (origin[1] as f64 + f64::from(low[1])) / scale - page_origin[1],
            };
            viewport.validate().map_err(|error| error.to_string())?;
            slices.push(PageSlice {
                page,
                destination: low,
                viewport,
            });
        }
        if band.tiles.iter().any(|(key, _)| {
            key.x < x0 || key.x >= x1 || key.y < y0 || key.y >= y1 || key.page != band.key
        }) {
            return Err("Document band tile lies outside its band.".into());
        }
        Ok(Self {
            size,
            origin,
            slices,
            tiles: band.tiles,
        })
    }
}

pub(crate) trait PageRenderer {
    fn render_page(
        &mut self,
        page: usize,
        viewport: Viewport,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Frame, String>;

    fn validate(&mut self, cancel: &AtomicBool) -> Result<(), String>;
}

impl PageRenderer for ImageSession {
    fn render_page(
        &mut self,
        page: usize,
        viewport: Viewport,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Frame, String> {
        ImageSession::render_page(self, page, viewport, cancel, timeout)
            .map_err(|error| error.to_string())
    }

    fn validate(&mut self, cancel: &AtomicBool) -> Result<(), String> {
        ImageSession::validate(self, cancel).map_err(|error| error.to_string())
    }
}

/// Render every page slice of `plan` once and fill its tiles. The plugin validates the
/// captured source before and after each slice; a band without slices validates it here,
/// so every completed band proves the source current.
pub(crate) fn render_band(
    renderer: &mut impl PageRenderer,
    plan: BandPlan,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<(Key, Arc<ViewerPixels>)>, String> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("Document render deadline exceeds its limit.")?;
    let mut tiles = Vec::with_capacity(plan.tiles.len());
    for _ in &plan.tiles {
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(TILE_BYTES)
            .map_err(|_| "Document tile allocation failed.")?;
        rgba.resize(TILE_BYTES, 0);
        tiles.push(rgba);
    }
    let mut touched = vec![false; tiles.len()];
    for slice in &plan.slices {
        PAGE_RENDERS.fetch_add(1, Ordering::Relaxed);
        let mut frame =
            renderer.render_page(slice.page, slice.viewport, cancel, remaining(deadline)?)?;
        if [frame.width, frame.height] != [slice.viewport.width, slice.viewport.height]
            || frame.rgba.len() != raster_length([frame.width, frame.height])?
        {
            return Err("Invalid document page raster.".into());
        }
        crate::image_pixels::premultiply(&mut frame.rgba);
        for (((key, _), tile), touched) in plan.tiles.iter().zip(&mut tiles).zip(&mut touched) {
            *touched |= composite(tile, *key, &plan, slice, &frame, *touched);
        }
    }
    if plan.slices.is_empty() {
        renderer.validate(cancel)?;
    }
    remaining(deadline)?;
    plan.tiles
        .into_iter()
        .zip(tiles)
        .map(|((key, charge), rgba)| {
            ViewerPixels::new(rgba, charge)
                .map(|pixels| (key, Arc::new(pixels)))
                .map_err(str::to_owned)
        })
        .collect()
}

fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| "Document viewport rendering timed out.".into())
}

fn raster_length([width, height]: [u32; 2]) -> Result<usize, String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || pixels > MAX_PIXELS {
        return Err("Document viewport exceeds its raster limit.".into());
    }
    Ok(pixels as usize * 4)
}

/// Draw the part of `frame` (one page slice, premultiplied) that falls into the tile `key`
/// over it, and report whether any did. A tile no earlier slice drew into is still
/// transparent, where drawing over is a copy; a later slice (a page closer than one pixel
/// to the previous one) blends source-over.
fn composite(
    tile: &mut [u8],
    key: Key,
    plan: &BandPlan,
    slice: &PageSlice,
    frame: &Frame,
    blend: bool,
) -> bool {
    let edge = i64::from(EDGE);
    // The tile and the slice in band pixels.
    let tile_low = [
        i64::from(key.x) * edge - plan.origin[0],
        i64::from(key.y) * edge - plan.origin[1],
    ];
    let slice_low = slice.destination.map(i64::from);
    let slice_high = [
        slice_low[0] + i64::from(frame.width),
        slice_low[1] + i64::from(frame.height),
    ];
    debug_assert!(
        slice_high[0] <= i64::from(plan.size[0]) && slice_high[1] <= i64::from(plan.size[1]),
        "a page slice lies within its band"
    );
    let low = [tile_low[0].max(slice_low[0]), tile_low[1].max(slice_low[1])];
    let high = [
        (tile_low[0] + edge).min(slice_high[0]),
        (tile_low[1] + edge).min(slice_high[1]),
    ];
    if high[0] <= low[0] || high[1] <= low[1] {
        return false;
    }
    let width = (high[0] - low[0]) as usize * 4;
    for y in low[1]..high[1] {
        let source_start =
            (((y - slice_low[1]) * i64::from(frame.width) + (low[0] - slice_low[0])) * 4) as usize;
        let target_start = (((y - tile_low[1]) * edge + (low[0] - tile_low[0])) * 4) as usize;
        let source = &frame.rgba[source_start..source_start + width];
        let target = &mut tile[target_start..target_start + width];
        if !blend {
            target.copy_from_slice(source);
            continue;
        }
        for (source, target) in source
            .as_chunks::<4>()
            .0
            .iter()
            .zip(target.as_chunks_mut::<4>().0)
        {
            match source[3] {
                255 => *target = *source,
                0 => {}
                alpha => {
                    let inverse_alpha = 255 - u16::from(alpha);
                    for channel in 0..4 {
                        target[channel] = (u16::from(source[channel])
                            + (u16::from(target[channel]) * inverse_alpha + 127) / 255)
                            .min(255) as u8;
                    }
                }
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "document_render_tests.rs"]
mod tests;
