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
    pub fn base(&mut self, width: u32, height: u32, pixels: Vec<u8>) {
        if u64::from(width) * u64::from(height) <= 16 * 1024 * 1024
            && pixels.len() == width as usize * height as usize * 4
        {
            self.width = width;
            self.height = height;
            self.base = pixels;
        }
    }
    pub fn popup_rect(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.popup.clear();
    }
    pub fn show(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.popup.clear();
        }
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
        })
    }
    pub fn resize(&mut self, revision: u64, width: u32, height: u32) -> Result<(), &'static str> {
        *self = Self::new(revision, width, height)?;
        Ok(())
    }
    pub fn paint(
        &mut self,
        revision: u64,
        width: u32,
        height: u32,
        bgra: &[u8],
    ) -> Option<Vec<u8>> {
        if revision != self.revision
            || width != self.width
            || height != self.height
            || bgra.len() != width as usize * height as usize * 4
        {
            return None;
        }
        let mut rgba = Vec::with_capacity(bgra.len());
        for p in bgra.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        Some(rgba)
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
                    .map(|p| [p[2], p[1], p[0], p[3]]),
            );
        }
        Some(rgba.into_flattened())
    }
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
