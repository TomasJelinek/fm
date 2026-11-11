//! Bounded retained image scene, independent of the decoder actor and presentation toolkit.
use crate::{
    image_geometry::ImageGeometry,
    image_tiles::{self, Key, Plan, Tiles},
};
use midnight_core::{ViewerPixels, ViewerSurfaceId, ViewerTile};
use std::sync::{Arc, atomic::AtomicU64};

pub(crate) struct Scene {
    ids: Arc<AtomicU64>,
    cache: Tiles,
    signature: Option<(usize, u64, u64, [u32; 2])>,
    pub surface: ViewerSurfaceId,
    pub sequence: u64,
    previous_origin: [f64; 2],
    raster_image_size: [f64; 2],
    direction: [f64; 2],
    pub plan: Option<Plan>,
}

impl Scene {
    pub fn new(ids: Arc<AtomicU64>) -> Self {
        Self {
            cache: Tiles::new(ids.clone()),
            ids,
            signature: None,
            surface: ViewerSurfaceId(0),
            sequence: 0,
            previous_origin: [0.0; 2],
            raster_image_size: [0.0; 2],
            direction: [0.0; 2],
            plan: None,
        }
    }

    pub fn update(&mut self, page: usize, geometry: &ImageGeometry) -> Result<(), String> {
        self.update_camera(page, geometry, true)
    }

    pub fn update_document(&mut self, page: usize, geometry: &ImageGeometry) -> Result<(), String> {
        self.update_camera(page, geometry, false)
    }

    fn update_camera(
        &mut self,
        page: usize,
        geometry: &ImageGeometry,
        retain_source_pixels: bool,
    ) -> Result<(), String> {
        let (mut size, density) = geometry.raster_geometry();
        let requested_scale = geometry.scale * density;
        let scale = if retain_source_pixels {
            requested_scale.min(1.0)
        } else {
            requested_scale
        };
        if scale < requested_scale {
            // Magnification changes presentation coordinates, not retained source pixels.
            // Plan's extra fractional pixel still covers the rounded-down viewport edge.
            size = geometry
                .viewport
                .map(|edge| (f64::from(edge) * (scale / geometry.scale)).max(1.0) as u32);
        }
        self.raster_image_size =
            [geometry.width, geometry.height].map(|size| f64::from(size) * scale);
        let signature = (
            page,
            scale.to_bits(),
            geometry.scale.to_bits(),
            geometry.viewport,
        );
        if self.signature != Some(signature) {
            self.surface = ViewerSurfaceId(image_tiles::next_id(&self.ids)?);
            self.sequence = 0;
            self.signature = Some(signature);
            self.direction = [0.0; 2];
        } else {
            for axis in 0..2 {
                let delta = geometry.origin[axis] - self.previous_origin[axis];
                if delta != 0.0 {
                    self.direction[axis] = delta;
                }
            }
        }
        self.previous_origin = geometry.origin;
        self.plan = Some(Plan::new(
            page,
            scale,
            geometry.origin,
            size,
            self.direction,
        ));
        Ok(())
    }

    pub fn accept_pan(
        &mut self,
        geometry: &mut ImageGeometry,
        surface: ViewerSurfaceId,
        sequence: u64,
        origin: [f64; 2],
    ) -> bool {
        if surface != self.surface
            || sequence <= self.sequence
            || origin.iter().any(|value| !value.is_finite())
        {
            return false;
        }
        self.sequence = sequence;
        geometry.pan_to(origin);
        true
    }

    /// Prepare transparent letterbox tiles locally; only source pixels need the decoder.
    pub fn next(&mut self) -> Result<Option<(Key, Arc<image_tiles::Charge>)>, String> {
        let Some(plan) = &self.plan else {
            return Ok(None);
        };
        for key in plan.keys() {
            if self.cache.contains(key) {
                continue;
            }
            let Some(charge) = self.cache.reserve() else {
                return Ok(None);
            };
            let outside = key.x < 0
                || key.y < 0
                || f64::from(key.x) * f64::from(image_tiles::EDGE) >= self.raster_image_size[0]
                || f64::from(key.y) * f64::from(image_tiles::EDGE) >= self.raster_image_size[1];
            if !outside {
                return Ok(Some((key, charge)));
            }
            // Preserve the complete camera grid and ordinary allocation ownership.
            // Publishing still waits for this revision's source validation in ImageView.
            let pixels = ViewerPixels::transparent(
                image_tiles::EDGE as usize * image_tiles::EDGE as usize * 4,
                charge,
            )?;
            self.cache.insert(key, Arc::new(pixels), plan)?;
        }
        Ok(None)
    }

    pub fn needs_tiles(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|plan| plan.keys().any(|key| !self.cache.contains(key)))
    }

    pub fn complete(&mut self, key: Key, pixels: Arc<ViewerPixels>) -> Result<(), String> {
        let Some(plan) = &self.plan else {
            return Ok(());
        };
        // Keep useful in-flight source pixels; page and sampling-scale changes have distinct keys.
        if !plan.keys().any(|needed| needed == key) {
            return Ok(());
        }
        self.cache.insert(key, pixels, plan)
    }

    pub fn tiles(&mut self) -> Option<Vec<ViewerTile>> {
        let plan = self.plan.as_ref()?;
        if plan.visible.iter().any(|key| !self.cache.contains(*key)) {
            return None;
        }
        let guards_ready = plan.guards.iter().all(|key| self.cache.contains(*key));
        Some(
            plan.keys()
                .filter(|key| guards_ready || plan.visible.contains(key))
                .filter_map(|key| {
                    self.cache.get(key).map(|(id, pixels)| ViewerTile {
                        id,
                        position: [
                            (f64::from(key.x) * f64::from(image_tiles::EDGE)
                                - plan.raster_origin[0]) as f32,
                            (f64::from(key.y) * f64::from(image_tiles::EDGE)
                                - plan.raster_origin[1]) as f32,
                        ],
                        width: image_tiles::EDGE,
                        height: image_tiles::EDGE,
                        pixels,
                    })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
#[path = "image_scene_tests.rs"]
mod tests;
