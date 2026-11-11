use crate::{LayoutError, Rect, TextPage, TextRenderer};
use std::ops::Range;

#[derive(Clone, Copy)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub origin: [f32; 2],
    pub background: [u8; 4],
    pub selection_color: [u8; 4],
}

impl Viewport {
    pub(crate) fn translated(self, offset: [f32; 2]) -> Self {
        Self {
            origin: std::array::from_fn(|axis| {
                stable_pixel(f64::from(self.origin[axis] - offset[axis])) as f32
            }),
            ..self
        }
    }
}

// A caret restored through different composite parents can leave sub-millipixel
// cancellation error. Normalize final paint coordinates, including root borders,
// without discarding meaningful fractional scrolling.
pub(crate) fn stable_pixel(value: f64) -> f64 {
    let pixel = value.round();
    if (value - pixel).abs() < 1.0 / 1024.0 {
        pixel
    } else {
        value
    }
}

pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, ready for the shared host surface.
    pub rgba: Vec<u8>,
}

pub(crate) struct Fill {
    pub rect: Rect,
    pub color: [u8; 4],
    pub radius: f32,
}

const MAX_PIXELS: usize = 16 * 1024 * 1024;

impl TextRenderer {
    /// Paint a prepared annotation into an existing surface, without reshaping or
    /// allocating another viewport. Annotation coordinates are physical pixels.
    pub fn paint_over(
        &mut self,
        page: &TextPage,
        raster: &mut Raster,
        origin: [f32; 2],
    ) -> Result<(), LayoutError> {
        if origin.iter().any(|value| !value.is_finite()) {
            return Err(LayoutError::InvalidGeometry);
        }
        self.glyphs.paint(
            page,
            Viewport {
                width: raster.width,
                height: raster.height,
                origin,
                background: [0; 4],
                selection_color: [0; 4],
            },
            raster,
        )
    }

    pub fn rasterize(
        &mut self,
        page: &TextPage,
        viewport: Viewport,
        selection: Option<Range<usize>>,
    ) -> Result<Raster, LayoutError> {
        let pixels = (viewport.width as usize)
            .checked_mul(viewport.height as usize)
            .filter(|&n| n > 0 && n <= MAX_PIXELS)
            .ok_or(LayoutError::LimitExceeded)?;
        if viewport.origin.iter().any(|value| !value.is_finite()) {
            return Err(LayoutError::InvalidGeometry);
        }
        let rectangles = selection
            .map(|range| page.selection_geometry(range))
            .transpose()?;
        let mut trace = cfg!(debug_assertions)
            .then(|| std::env::var_os("MIDNIGHT_VIEWER_SCROLL_TRACE"))
            .flatten()
            .map(|_| std::time::Instant::now());
        let mut raster = Raster {
            width: viewport.width,
            height: viewport.height,
            rgba: vec![0; pixels * 4],
        };
        let background = premultiply(viewport.background);
        for pixel in raster.rgba.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&background);
        }
        trace_phase("clear", &mut trace);
        paint_backgrounds(page, viewport, &mut raster);
        if let Some(rectangles) = rectangles {
            for rect in rectangles {
                raster.fill(rect, viewport.origin, viewport.selection_color);
            }
        }
        trace_phase("backgrounds", &mut trace);
        self.glyphs.paint(page, viewport, &mut raster)?;
        trace_phase("glyphs", &mut trace);
        Ok(raster)
    }
}

impl Raster {
    fn fill_rounded(&mut self, fill: &Fill, origin: [f32; 2]) {
        if fill.radius <= 0.0 {
            self.fill(fill.rect, origin, fill.color);
            return;
        }
        let x0 = stable_pixel(fill.rect.x0 - f64::from(origin[0]));
        let x1 = stable_pixel(fill.rect.x1 - f64::from(origin[0]));
        let y0 = stable_pixel(fill.rect.y0 - f64::from(origin[1]));
        let y1 = stable_pixel(fill.rect.y1 - f64::from(origin[1]));
        let radius = f64::from(fill.radius)
            .min((x1 - x0) / 2.0)
            .min((y1 - y0) / 2.0);
        let color = premultiply(fill.color);
        let left = x0.floor().clamp(0.0, f64::from(self.width)) as usize;
        let right = x1.ceil().clamp(0.0, f64::from(self.width)) as usize;
        if left >= right {
            return;
        }
        // Between the corner arcs dx is zero, so every pixel in this row has
        // exactly the same coverage. Keep scalar corner math and batch the body.
        let middle_left = (x0 + radius - 0.5).ceil().clamp(left as f64, right as f64) as usize;
        let middle_right =
            ((x1 - radius - 0.5).floor() + 1.0).clamp(middle_left as f64, right as f64) as usize;
        for y in
            y0.floor().max(0.0) as usize..y1.ceil().min(f64::from(self.height)).max(0.0) as usize
        {
            let py = y as f64 + 0.5;
            let dy = (y0 + radius - py).max(0.0).max(py - y1 + radius);
            let coverage = (radius + 0.5 - dy).clamp(0.0, 1.0);
            self.blend_span(
                y,
                middle_left,
                middle_right,
                color.map(|channel| (f64::from(channel) * coverage).round() as u8),
            );
            for x in (left..middle_left).chain(middle_right..right) {
                let px = x as f64 + 0.5;
                let dx = (x0 + radius - px).max(0.0).max(px - x1 + radius);
                let coverage = (radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
                self.blend(
                    x,
                    y,
                    color.map(|channel| (f64::from(channel) * coverage).round() as u8),
                );
            }
        }
    }
    pub(crate) fn fill(&mut self, rect: Rect, origin: [f32; 2], color: [u8; 4]) {
        let x0 = stable_pixel(rect.x0 - f64::from(origin[0]))
            .floor()
            .clamp(0.0, f64::from(self.width)) as usize;
        let x1 = stable_pixel(rect.x1 - f64::from(origin[0]))
            .ceil()
            .clamp(0.0, f64::from(self.width)) as usize;
        let y0 = stable_pixel(rect.y0 - f64::from(origin[1]))
            .floor()
            .clamp(0.0, f64::from(self.height)) as usize;
        let y1 = stable_pixel(rect.y1 - f64::from(origin[1]))
            .ceil()
            .clamp(0.0, f64::from(self.height)) as usize;
        let color = premultiply(color);
        for y in y0..y1 {
            self.blend_span(y, x0, x1, color);
        }
    }

    fn blend_span(&mut self, y: usize, left: usize, right: usize, color: [u8; 4]) {
        if left >= right {
            return;
        }
        if color[3] == 255 {
            let row = y * self.width as usize;
            self.rgba[(row + left) * 4..(row + right) * 4]
                .as_chunks_mut::<4>()
                .0
                .fill(color);
        } else if color[3] != 0 {
            for x in left..right {
                self.blend(x, y, color);
            }
        }
    }

    pub(crate) fn blend(&mut self, x: usize, y: usize, color: [u8; 4]) {
        let offset = (y * self.width as usize + x) * 4;
        let target = &mut self.rgba[offset..offset + 4];
        let remaining = 255 - u16::from(color[3]);
        for (target, source) in target.iter_mut().zip(color) {
            *target =
                (u16::from(source) + (u16::from(*target) * remaining + 127) / 255).min(255) as u8;
        }
    }
}

fn paint_backgrounds(page: &TextPage, view: Viewport, raster: &mut Raster) {
    if let Some(grid) = &page.composite {
        for fill in &grid.backgrounds {
            raster.fill_rounded(fill, view.origin);
        }
        for part in &grid.parts {
            if part.origin[1] + part.page.height() >= view.origin[1]
                && part.origin[1] <= view.origin[1] + view.height as f32
            {
                paint_backgrounds(&part.page, view.translated(part.origin), raster);
            }
        }
    }
}

pub(crate) fn premultiply(color: [u8; 4]) -> [u8; 4] {
    let alpha = u16::from(color[3]);
    let channel = |value| ((u16::from(value) * alpha + 127) / 255) as u8;
    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        color[3],
    ]
}

fn trace_phase(phase: &str, started: &mut Option<std::time::Instant>) {
    if let Some(started) = started {
        eprintln!(
            "raster_phase phase={phase} elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
        *started = std::time::Instant::now();
    }
}

#[cfg(test)]
#[path = "raster_fill_tests.rs"]
mod fill_tests;
