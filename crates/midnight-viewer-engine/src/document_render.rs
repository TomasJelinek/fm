//! Bounded page-local rendering into one document viewport. No retained tile amplification.
use crate::document_view::DocumentView;
use midnight_core::DocumentLayout;
use midnight_image_viewer_host::{Frame, ImageSession, Viewport};
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

const MAX_EDGE: u32 = 4096;
const MAX_PIXELS: u64 = 4_194_304;

pub(crate) struct PageSlice {
    pub page: usize,
    pub destination: [u32; 2],
    pub viewport: Viewport,
}

pub(crate) struct DocumentRenderPlan {
    pub size: [u32; 2],
    pub slices: Vec<PageSlice>,
}

impl DocumentRenderPlan {
    pub fn new(document: &DocumentView) -> Result<Self, String> {
        let camera = &document.geometry;
        let (size, density) = camera.raster_geometry();
        raster_length(size)?;
        let scale = camera.scale * density;
        if !scale.is_finite() || !(1.0 / 32768.0..=64.0).contains(&scale) {
            return Err("Document render scale exceeds its limit.".into());
        }
        let mut slices = Vec::new();
        let top = camera.origin[1];
        let bottom = top + f64::from(size[1]) / scale;
        let range = if document.layout == DocumentLayout::Paged {
            document.page..document.page + 1
        } else {
            let first = document
                .placements
                .partition_point(|page| page.bottom() <= top);
            let end = document
                .placements
                .partition_point(|page| page.origin[1] < bottom);
            first..end
        };
        for page in range {
            let placement = document.placements[page];
            let origin = if document.layout == DocumentLayout::Paged {
                [0.0; 2]
            } else {
                placement.origin
            };
            let low: [u32; 2] = std::array::from_fn(|axis| {
                ((origin[axis] - camera.origin[axis]) * scale)
                    .floor()
                    .clamp(0.0, f64::from(size[axis])) as u32
            });
            let high: [u32; 2] = std::array::from_fn(|axis| {
                ((origin[axis] + f64::from(placement.size[axis]) - camera.origin[axis]) * scale)
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
                origin_x: camera.origin[0] + f64::from(low[0]) / scale - origin[0],
                origin_y: camera.origin[1] + f64::from(low[1]) / scale - origin[1],
            };
            viewport.validate().map_err(|error| error.to_string())?;
            slices.push(PageSlice {
                page,
                destination: low,
                viewport,
            });
        }
        Ok(Self { size, slices })
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

#[derive(Debug)]
pub(crate) enum Composition {
    Frame(Frame),
    Superseded,
}

pub(crate) fn compose(
    renderer: &mut impl PageRenderer,
    plan: &DocumentRenderPlan,
    revision: u64,
    wanted: &AtomicU64,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Composition, String> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("Document render deadline exceeds its limit.")?;
    if !current(revision, wanted, cancel)? {
        return Ok(Composition::Superseded);
    }
    remaining(deadline)?;
    let length = raster_length(plan.size)?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(length)
        .map_err(|_| "Document viewport allocation failed.")?;
    rgba.resize(length, 0);
    for slice in &plan.slices {
        if !current(revision, wanted, cancel)? {
            return Ok(Composition::Superseded);
        }
        // Only close/source retirement owns cancel. Camera supersession never ends the SDK session.
        let mut scratch =
            renderer.render_page(slice.page, slice.viewport, cancel, remaining(deadline)?)?;
        if !current(revision, wanted, cancel)? {
            return Ok(Composition::Superseded);
        }
        remaining(deadline)?;
        if [scratch.width, scratch.height] != [slice.viewport.width, slice.viewport.height]
            || scratch.rgba.len() != raster_length([scratch.width, scratch.height])?
        {
            return Err("Invalid document page raster.".into());
        }
        crate::image_pixels::premultiply(&mut scratch.rgba);
        composite(&mut rgba, plan.size, slice.destination, &scratch)?;
    }
    if !current(revision, wanted, cancel)? {
        return Ok(Composition::Superseded);
    }
    remaining(deadline)?;
    renderer.validate(cancel)?;
    if !current(revision, wanted, cancel)? {
        return Ok(Composition::Superseded);
    }
    remaining(deadline)?;
    Ok(Composition::Frame(Frame {
        width: plan.size[0],
        height: plan.size[1],
        rgba,
    }))
}

fn current(revision: u64, wanted: &AtomicU64, cancel: &AtomicBool) -> Result<bool, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Document rendering cancelled.".into());
    }
    Ok(wanted.load(Ordering::Acquire) == revision)
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

fn composite(
    output: &mut [u8],
    size: [u32; 2],
    destination: [u32; 2],
    scratch: &Frame,
) -> Result<(), String> {
    if destination[0]
        .checked_add(scratch.width)
        .is_none_or(|end| end > size[0])
        || destination[1]
            .checked_add(scratch.height)
            .is_none_or(|end| end > size[1])
    {
        return Err("Document page lies outside the output raster.".into());
    }
    for row in 0..scratch.height as usize {
        let source_start = row * scratch.width as usize * 4;
        let target_start =
            ((destination[1] as usize + row) * size[0] as usize + destination[0] as usize) * 4;
        let source = &scratch.rgba[source_start..source_start + scratch.width as usize * 4];
        let target = &mut output[target_start..target_start + scratch.width as usize * 4];
        for (source, target) in source
            .as_chunks::<4>()
            .0
            .iter()
            .zip(target.as_chunks_mut::<4>().0)
        {
            let inverse_alpha = 255 - u16::from(source[3]);
            for channel in 0..4 {
                target[channel] = (u16::from(source[channel])
                    + (u16::from(target[channel]) * inverse_alpha + 127) / 255)
                    .min(255) as u8;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "document_render_tests.rs"]
mod tests;
