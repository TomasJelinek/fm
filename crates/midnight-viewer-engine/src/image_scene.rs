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
    signature: Option<(usize, u64, [u32; 2])>,
    pub surface: ViewerSurfaceId,
    pub sequence: u64,
    previous_origin: [f64; 2],
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
            direction: [0.0; 2],
            plan: None,
        }
    }

    pub fn update(&mut self, page: usize, geometry: &ImageGeometry) -> Result<(), String> {
        let (size, density) = geometry.raster_geometry();
        let scale = geometry.scale * density;
        let signature = (page, scale.to_bits(), geometry.viewport);
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

    pub fn next(&self) -> Option<(Key, Arc<image_tiles::Charge>)> {
        let key = self
            .plan
            .as_ref()?
            .keys()
            .find(|key| !self.cache.contains(*key))?;
        Some((key, self.cache.reserve()?))
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
        // Keep a useful in-flight pan tile; never install an old page or zoom reply.
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
