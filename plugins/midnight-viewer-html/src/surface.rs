//! Owned bounded CPU surfaces. CEF paint memory never leaves its callback.

/// Bounded premultiplied popup composition for Chromium select controls.
#[derive(Default)]
pub struct Layers {
    width: u32,
    height: u32,
    base: Vec<u8>,
    popup: Vec<u8>,
    popup_width: u32,
    popup_height: u32,
    x: i32,
    y: i32,
    visible: bool,
}

impl Layers {
    pub(crate) fn popup_visible(&self) -> bool {
        self.visible
    }

    /// Publishes a converted view paint. Without a visible popup the pixels move
    /// straight into the returned frame and no base copy is retained; while a popup is
    /// visible the paint is kept as the composition base and the composed frame is
    /// returned. Invalid dimensions return None.
    pub fn view(&mut self, width: u32, height: u32, pixels: Vec<u8>) -> Option<Vec<u8>> {
        if u64::from(width) * u64::from(height) > 16 * 1024 * 1024
            || pixels.len() != width as usize * height as usize * 4
        {
            return None;
        }
        self.width = width;
        self.height = height;
        if !self.visible {
            self.base = Vec::new();
            return Some(pixels);
        }
        self.base = pixels;
        self.frame()
    }
    pub fn popup_rect(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.popup.clear();
    }
    /// Shows or hides the popup. Hiding returns the retained base as the next frame
    /// (moved, not copied). Showing returns None: no base is retained while no popup
    /// is visible, so the caller must request a fresh view paint to compose onto.
    pub fn show(&mut self, visible: bool) -> Option<Vec<u8>> {
        self.visible = visible;
        if visible {
            return None;
        }
        self.popup.clear();
        (!self.base.is_empty()).then(|| std::mem::take(&mut self.base))
    }
    pub fn popup(&mut self, width: u32, height: u32, bgra: &[u8]) -> bool {
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || u64::from(width) * u64::from(height) > 1024 * 1024
            || bgra.len() != width as usize * height as usize * 4
        {
            return false;
        }
        self.popup_width = width;
        self.popup_height = height;
        self.popup.clear();
        for p in bgra.as_chunks::<4>().0 {
            self.popup.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        true
    }
    /// Composes the retained base with the visible popup; None without a base.
    pub fn frame(&self) -> Option<Vec<u8>> {
        if self.base.is_empty() {
            return None;
        }
        let mut frame = self.base.clone();
        if self.visible && !self.popup.is_empty() {
            for y in 0..self.popup_height {
                let target_y = i64::from(self.y) + i64::from(y);
                if target_y < 0 || target_y >= i64::from(self.height) {
                    continue;
                }
                for x in 0..self.popup_width {
                    let target_x = i64::from(self.x) + i64::from(x);
                    if target_x < 0 || target_x >= i64::from(self.width) {
                        continue;
                    }
                    let source = (y as usize * self.popup_width as usize + x as usize) * 4;
                    let target = (target_y as usize * self.width as usize + target_x as usize) * 4;
                    let inverse_alpha = 255u32 - u32::from(self.popup[source + 3]);
                    for channel in 0..4 {
                        let blended = u32::from(self.popup[source + channel])
                            + (u32::from(frame[target + channel]) * inverse_alpha + 127) / 255;
                        frame[target + channel] = blended.min(255) as u8;
                    }
                }
            }
        }
        Some(frame)
    }
}
pub struct Surface {
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    view_paint: u64,
}
impl Surface {
    pub fn new(revision: u64, width: u32, height: u32) -> Result<Self, &'static str> {
        if revision == 0
            || width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || u64::from(width) * u64::from(height) > 16 * 1024 * 1024
        {
            return Err("invalid surface dimensions");
        }
        Ok(Self {
            revision,
            width,
            height,
            view_paint: 0,
        })
    }
    pub fn resize(&mut self, revision: u64, width: u32, height: u32) -> Result<(), &'static str> {
        *self = Self::new(revision, width, height)?;
        Ok(())
    }

    /// Counts even view callbacks skipped after native geometry validation fails.
    /// A sequence gap prevents using damage measured since an unqueued paint.
    pub(crate) fn begin_view_paint(&mut self) -> Option<u64> {
        self.view_paint = self.view_paint.checked_add(1)?;
        Some(self.view_paint)
    }
    pub fn paint_cropped(
        &mut self,
        revision: u64,
        width: u32,
        height: u32,
        bgra: &[u8],
    ) -> Option<Vec<u8>> {
        if revision != self.revision
            || width < self.width
            || height < self.height
            || width > self.width + 16
            || height > self.height + 16
            || bgra.len() != width as usize * height as usize * 4
        {
            return None;
        }
        // Extend exact-size typed rows instead of growing a byte Vec for every
        // pixel. This retains development overflow checks without their per-pixel
        // capacity work. Flattening reuses this allocation; no extra image copy.
        let mut rgba = Vec::<[u8; 4]>::with_capacity(self.width as usize * self.height as usize);
        for row in bgra
            .chunks_exact(width as usize * 4)
            .take(self.height as usize)
        {
            rgba.extend(
                row[..self.width as usize * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(bgra_to_rgba),
            );
        }
        Some(rgba.into_flattened())
    }
}

/// Maximum metadata/work-plan size before using the whole-conversion path.
pub(crate) const MAX_DAMAGE_RECTS: usize = 64;

/// CEF damage in signed native-buffer pixels, before the viewport's DPI crop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DamageRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Default)]
struct PatchRows {
    source: usize,
    target: usize,
    bytes: usize,
    rows: usize,
}

fn bgra_to_rgba(pixel: &[u8; 4]) -> [u8; 4] {
    [pixel[2], pixel[1], pixel[0], pixel[3]]
}

impl Surface {
    /// Patch an exclusive, immediately preceding RGBA viewport from a full CEF
    /// BGRA paint. The caller proves document/slot ownership and base geometry;
    /// this method checks revision, buffer geometry and every damage rectangle.
    ///
    /// False leaves every base byte untouched, permitting whole conversion.
    /// At most 64 rectangles and half a viewport's summed clipped pixel area
    /// are admitted; overlapping area counts repeatedly. No pixel buffer is
    /// retained or allocated. Coordinates are native pixels, without scaling.
    pub(crate) fn patch_cropped(
        &self,
        revision: u64,
        width: u32,
        height: u32,
        bgra: &[u8],
        damage: &[DamageRect],
        rgba: &mut [u8],
    ) -> bool {
        if Self::new(self.revision, self.width, self.height).is_err()
            || revision != self.revision
            || width.checked_sub(self.width).is_none_or(|extra| extra > 16)
            || height
                .checked_sub(self.height)
                .is_none_or(|extra| extra > 16)
            || damage.is_empty()
            || damage.len() > MAX_DAMAGE_RECTS
        {
            return false;
        }
        let Some(source_stride) = (width as usize).checked_mul(4) else {
            return false;
        };
        let Some(target_stride) = (self.width as usize).checked_mul(4) else {
            return false;
        };
        let Some(source_len) = source_stride.checked_mul(height as usize) else {
            return false;
        };
        let Some(target_len) = target_stride.checked_mul(self.height as usize) else {
            return false;
        };
        if bgra.len() != source_len || rgba.len() != target_len {
            return false;
        }
        let mut patches = [PatchRows::default(); MAX_DAMAGE_RECTS];
        let mut count = 0;
        let mut area = 0_usize;
        // Build a bounded plan before the first write, including all row ends.
        for rect in damage {
            if rect.width <= 0 || rect.height <= 0 {
                return false;
            }
            let Some(right) = i64::from(rect.x).checked_add(i64::from(rect.width)) else {
                return false;
            };
            let Some(bottom) = i64::from(rect.y).checked_add(i64::from(rect.height)) else {
                return false;
            };
            let left = i64::from(rect.x).clamp(0, i64::from(self.width));
            let top = i64::from(rect.y).clamp(0, i64::from(self.height));
            let right = right.clamp(0, i64::from(self.width));
            let bottom = bottom.clamp(0, i64::from(self.height));
            if right <= left || bottom <= top {
                continue;
            }
            // Clipping bounds these nonnegative values to valid surface sizes.
            let columns = (right - left) as usize;
            let rows = (bottom - top) as usize;
            let Some(bytes) = columns.checked_mul(4) else {
                return false;
            };
            let Some(next_area) = columns.checked_mul(rows).and_then(|a| area.checked_add(a))
            else {
                return false;
            };
            if next_area > target_len / 4 / 2 {
                return false;
            }
            let offset = |stride: usize| {
                (top as usize)
                    .checked_mul(stride)?
                    .checked_add((left as usize).checked_mul(4)?)
            };
            let Some(source) = offset(source_stride) else {
                return false;
            };
            let Some(target) = offset(target_stride) else {
                return false;
            };
            let end = |start: usize, stride: usize| {
                rows.checked_sub(1)?
                    .checked_mul(stride)?
                    .checked_add(start)?
                    .checked_add(bytes)
            };
            if end(source, source_stride).is_none_or(|end| end > source_len)
                || end(target, target_stride).is_none_or(|end| end > target_len)
            {
                return false;
            }
            patches[count] = PatchRows {
                source,
                target,
                bytes,
                rows,
            };
            count += 1;
            area = next_area;
        }
        if area == 0 {
            return false;
        }
        // All indices, row ends and aggregate work are proven above; this pass
        // has no rejection after any byte has been changed.
        for patch in &patches[..count] {
            for row in 0..patch.rows {
                let source = patch.source + row * source_stride;
                let target = patch.target + row * target_stride;
                for (output, input) in rgba[target..target + patch.bytes]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(bgra[source..source + patch.bytes].as_chunks::<4>().0)
                {
                    *output = bgra_to_rgba(input);
                }
            }
        }
        true
    }
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
