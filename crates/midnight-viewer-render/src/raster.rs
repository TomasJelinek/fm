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

pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, ready for the shared host surface.
    pub rgba: Vec<u8>,
}

const MAX_PIXELS: usize = 16 * 1024 * 1024;

impl TextRenderer {
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
        let mut raster = Raster {
            width: viewport.width,
            height: viewport.height,
            rgba: vec![0; pixels * 4],
        };
        let background = premultiply(viewport.background);
        for pixel in raster.rgba.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&background);
        }
        if let Some(rectangles) = rectangles {
            for rect in rectangles {
                raster.fill(rect, viewport.origin, viewport.selection_color);
            }
        }
        self.glyphs.paint(page, viewport, &mut raster)?;
        Ok(raster)
    }
}

impl Raster {
    fn fill(&mut self, rect: Rect, origin: [f32; 2], color: [u8; 4]) {
        let x0 = (rect.x0 - f64::from(origin[0]))
            .floor()
            .clamp(0.0, f64::from(self.width)) as usize;
        let x1 = (rect.x1 - f64::from(origin[0]))
            .ceil()
            .clamp(0.0, f64::from(self.width)) as usize;
        let y0 = (rect.y0 - f64::from(origin[1]))
            .floor()
            .clamp(0.0, f64::from(self.height)) as usize;
        let y1 = (rect.y1 - f64::from(origin[1]))
            .ceil()
            .clamp(0.0, f64::from(self.height)) as usize;
        let color = premultiply(color);
        for y in y0..y1 {
            for x in x0..x1 {
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
