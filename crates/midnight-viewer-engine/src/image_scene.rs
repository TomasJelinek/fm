//! Bounded retained image scene, independent of the decoder actor and presentation toolkit.
//!
//! Documents use the same scene as retained tiles of a canvas (one page in the Paged
//! layout, every page in the Continuous one) at the exact physical scale. Missing tiles
//! are rendered in bands, visible ones first, then the guard rows prefetched in the
//! direction of motion. A completed band is kept whenever its tiles still belong to the
//! current camera's plan: a newer camera never discards finished work that it can use,
//! so continuous input cannot starve publication.
use crate::{
    document_view::PagePlacement,
    image_geometry::ImageGeometry,
    image_tiles::{self, EDGE, Key, Plan, TILE_BYTES, Tiles},
};
use midnight_core::{ViewerPixels, ViewerSurfaceId, ViewerTile};
use std::{
    collections::HashSet,
    sync::{Arc, atomic::AtomicU64},
};

/// At most 16 tiles across and down: a page slice of a band stays within the 4096-pixel
/// edge.
const MAX_BAND_EDGE: i32 = 16;
/// At most 64 tiles (4 Mpx): the plugin's viewport pixel bound.
const MAX_BAND_TILES: i32 = 64;
/// A raster view edge leaves room for guard tiles within the tile position bound.
const MAX_VIEW_EDGE: f64 = midnight_core::VIEWER_TILE_POSITION_BOUND as f64 - 2048.0;

/// A document's tile canvas: one page in the Paged layout, every page in the Continuous one.
pub(crate) struct Canvas {
    /// `Key::page` of its tiles: the page in the Paged layout, `image_tiles::CANVAS` in the
    /// Continuous one.
    pub key: usize,
    /// Pages in canvas units, ordered from top to bottom.
    pub pages: Arc<[PagePlacement]>,
    pub extent: [f64; 2],
}

/// A rectangle of whole tiles of one canvas at one scale, and the missing tiles in it that
/// the band renders, each with its reserved pixel budget.
pub(crate) struct Band {
    pub key: usize,
    pub scale: f64,
    /// Tile columns and rows: `[x0, y0, x1, y1]`, end exclusive.
    pub rect: [i32; 4],
    pub tiles: Vec<(Key, Arc<image_tiles::Charge>)>,
}

struct DocumentScene {
    pages: Arc<[PagePlacement]>,
    /// Advances on every camera change other than a UI pan, so the UI adopts the camera.
    epoch: u64,
}

type Signature = (usize, u64, u64, [u32; 2], u64);

pub(crate) struct Scene {
    ids: Arc<AtomicU64>,
    cache: Tiles,
    signature: Option<Signature>,
    pub surface: ViewerSurfaceId,
    pub sequence: u64,
    previous_origin: [f64; 2],
    raster_image_size: [f64; 2],
    direction: [f64; 2],
    pub plan: Option<Plan>,
    document: Option<DocumentScene>,
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
            document: None,
        }
    }

    pub fn update(&mut self, page: usize, geometry: &ImageGeometry) -> Result<(), String> {
        let (mut size, density) = geometry.raster_geometry();
        let requested_scale = geometry.scale * density;
        let scale = requested_scale.min(1.0);
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
            0,
        );
        self.track(signature, geometry.origin)?;
        self.plan = Some(Plan::new(
            page,
            scale,
            geometry.origin,
            size,
            self.direction,
        ));
        Ok(())
    }

    /// Plan a document canvas at exactly `geometry.scale` physical pixels per unit.
    /// `jump` marks a camera the UI did not choose itself, such as a page turn: it gets a
    /// new surface.
    pub fn update_document(
        &mut self,
        canvas: &Canvas,
        geometry: &ImageGeometry,
        jump: bool,
    ) -> Result<(), String> {
        let (size, density) = document_raster(geometry.viewport);
        let scale = geometry.scale * density;
        if !scale.is_finite() || !(1.0 / 32768.0..=64.0).contains(&scale) {
            return Err("Document render scale exceeds its limit.".into());
        }
        let epoch = match &mut self.document {
            Some(document) => {
                if jump {
                    document.epoch = document.epoch.wrapping_add(1);
                }
                document.pages = canvas.pages.clone();
                document.epoch
            }
            None => {
                self.document = Some(DocumentScene {
                    pages: canvas.pages.clone(),
                    epoch: 0,
                });
                0
            }
        };
        self.raster_image_size = canvas.extent.map(|extent| extent * scale);
        self.cache
            .set_budget(midnight_core::viewer_document_tile_cache_bytes(size));
        let signature = (
            canvas.key,
            scale.to_bits(),
            geometry.scale.to_bits(),
            geometry.viewport,
            epoch,
        );
        self.track(signature, geometry.origin)?;
        // Half a view of guard rows ahead, within the tile position bound.
        let room = ((f64::from(midnight_core::VIEWER_TILE_POSITION_BOUND) - f64::from(size[1]))
            / f64::from(EDGE))
        .floor() as i32
            - 2;
        let rows = (midnight_core::viewer_document_guard_rows(size[1]) as i32)
            .min(room)
            .max(1);
        self.plan = Some(Plan::with_depth(
            canvas.key,
            scale,
            geometry.origin,
            size,
            self.direction,
            [1, rows],
        ));
        Ok(())
    }

    fn track(&mut self, signature: Signature, origin: [f64; 2]) -> Result<(), String> {
        if self.signature != Some(signature) {
            self.surface = ViewerSurfaceId(image_tiles::next_id(&self.ids)?);
            self.sequence = 0;
            self.signature = Some(signature);
            self.direction = [0.0; 2];
        } else {
            for (axis, value) in origin.into_iter().enumerate() {
                let delta = value - self.previous_origin[axis];
                if delta != 0.0 {
                    self.direction[axis] = delta;
                }
            }
        }
        self.previous_origin = origin;
        Ok(())
    }

    pub fn accept_pan(
        &mut self,
        geometry: &mut ImageGeometry,
        surface: ViewerSurfaceId,
        sequence: u64,
        origin: [f64; 2],
    ) -> bool {
        if !self.accept_pan_sequence(surface, sequence, origin) {
            return false;
        }
        geometry.pan_to(origin);
        true
    }

    /// The latest camera request of the current surface; older sequences, replaced
    /// surfaces and nonfinite origins are refused.
    pub fn accept_pan_sequence(
        &mut self,
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
            if !self.transparent(key) {
                return Ok(Some((key, charge)));
            }
            // Preserve the complete camera grid and ordinary allocation ownership.
            // Publishing still waits for this revision's source validation in ImageView.
            let pixels = ViewerPixels::transparent(TILE_BYTES, charge)?;
            self.cache.insert(key, Arc::new(pixels), plan)?;
        }
        Ok(None)
    }

    /// The next rectangle of missing document tiles: it starts at the first missing tile
    /// of the plan (visible tiles first), spans the adjacent missing tiles of its row, then
    /// whole rows in the direction of motion, within 16 tiles across and down and 64 tiles
    /// in all. Tiles that meet no page are created transparent here without rendering.
    pub fn next_band(&mut self) -> Result<Option<Band>, String> {
        let Some(plan) = &self.plan else {
            return Ok(None);
        };
        let mut start = None;
        for key in plan.keys() {
            if self.cache.contains(key) {
                continue;
            }
            if !self.transparent(key) {
                start = Some(key);
                break;
            }
            let Some(charge) = self.cache.reserve() else {
                return Ok(None);
            };
            let pixels = ViewerPixels::transparent(TILE_BYTES, charge)?;
            self.cache.insert(key, Arc::new(pixels), plan)?;
        }
        let Some(start) = start else {
            return Ok(None);
        };
        let wanted: HashSet<Key> = plan
            .keys()
            .filter(|key| !self.cache.contains(*key) && !self.transparent(*key))
            .collect();
        // A band of visible tiles stays within the plan, so the view appears as soon as
        // possible. A prefetch band continues over further missing rows up to its bound:
        // a page is then interpreted once for many rows, not once per guard row.
        let prefetch = !plan.visible.contains(&start);
        let needed = |key: Key| {
            wanted.contains(&key)
                || (prefetch && !self.cache.contains(key) && !self.transparent(key))
        };
        let at = |x, y| Key { x, y, ..start };
        let (mut x0, mut x1) = (start.x, start.x + 1);
        while x1 - x0 < MAX_BAND_EDGE && wanted.contains(&at(x1, start.y)) {
            x1 += 1;
        }
        while x1 - x0 < MAX_BAND_EDGE && wanted.contains(&at(x0 - 1, start.y)) {
            x0 -= 1;
        }
        let rows = (MAX_BAND_TILES / (x1 - x0)).clamp(1, MAX_BAND_EDGE);
        let upward = self.direction[1] < 0.0;
        let (mut y0, mut y1) = (start.y, start.y + 1);
        while y1 - y0 < rows {
            let next = if upward { y0 - 1 } else { y1 };
            if !(x0..x1).all(|x| needed(at(x, next))) {
                break;
            }
            if upward {
                y0 -= 1;
            } else {
                y1 += 1;
            }
        }
        let mut tiles = Vec::new();
        'rows: for y in y0..y1 {
            for x in x0..x1 {
                if !needed(at(x, y)) {
                    continue;
                }
                let Some(charge) = self.cache.reserve() else {
                    break 'rows;
                };
                tiles.push((at(x, y), charge));
            }
        }
        if tiles.is_empty() {
            return Ok(None);
        }
        Ok(Some(Band {
            key: start.page,
            scale: plan.scale,
            rect: [x0, y0, x1, y1],
            tiles,
        }))
    }

    /// No source pixel reaches the tile: it lies outside the image or canvas, or between
    /// and beside a document's pages.
    fn transparent(&self, key: Key) -> bool {
        let edge = f64::from(EDGE);
        let [x0, y0] = [f64::from(key.x) * edge, f64::from(key.y) * edge];
        if key.x < 0
            || key.y < 0
            || x0 >= self.raster_image_size[0]
            || y0 >= self.raster_image_size[1]
        {
            return true;
        }
        let (Some(document), Some(plan)) = (&self.document, &self.plan) else {
            return false;
        };
        let scale = plan.scale;
        let [x1, y1] = [x0 + edge, y0 + edge];
        let pages = &document.pages;
        let first = pages.partition_point(|page| page.bottom() * scale <= y0);
        !pages[first..]
            .iter()
            .take_while(|page| page.origin[1] * scale < y1)
            .any(|page| {
                page.origin[0] * scale < x1
                    && (page.origin[0] + f64::from(page.size[0])) * scale > x0
            })
    }

    /// Every visible tile of the current plan exists.
    pub fn visible_ready(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|plan| plan.visible.iter().all(|key| self.cache.contains(*key)))
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

    /// Keep a band's tiles that the current camera's plan needs, whatever camera the band
    /// was rendered for. Others of the same canvas and scale (prefetched beyond the plan,
    /// or the plan moved on) replace only tiles outside the plan, least recently used first.
    pub fn complete_band(&mut self, tiles: Vec<(Key, Arc<ViewerPixels>)>) -> Result<(), String> {
        let Some(plan) = &self.plan else {
            return Ok(());
        };
        let needed: HashSet<Key> = plan.keys().collect();
        let current = plan.visible.first().map(|key| (key.page, key.scale));
        // Plan tiles first, so that a prefetched tile never displaces one of them.
        let (now, ahead): (Vec<_>, Vec<_>) =
            tiles.into_iter().partition(|(key, _)| needed.contains(key));
        for (key, pixels) in now {
            self.cache.insert(key, pixels, plan)?;
        }
        for (key, pixels) in ahead {
            if current == Some((key.page, key.scale)) {
                self.cache.insert_ahead(key, pixels, plan)?;
            }
        }
        Ok(())
    }

    /// Tiles the cache keeps at most.
    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.cache.capacity()
    }

    pub fn tiles(&mut self) -> Option<Vec<ViewerTile>> {
        let plan = self.plan.as_ref()?;
        if plan.visible.iter().any(|key| !self.cache.contains(*key)) {
            return None;
        }
        // Documents publish each guard tile as soon as it exists: the camera moves into
        // whatever is covered. Images keep their complete guard frontier.
        let guards_ready =
            self.document.is_some() || plan.guards.iter().all(|key| self.cache.contains(*key));
        Some(
            plan.keys()
                .filter(|key| guards_ready || plan.visible.contains(key))
                .filter_map(|key| {
                    self.cache.get(key).map(|(id, pixels)| ViewerTile {
                        id,
                        position: [
                            (f64::from(key.x) * f64::from(EDGE) - plan.raster_origin[0]) as f32,
                            (f64::from(key.y) * f64::from(EDGE) - plan.raster_origin[1]) as f32,
                        ],
                        width: EDGE,
                        height: EDGE,
                        pixels,
                    })
                })
                .collect(),
        )
    }
}

/// The raster view of a document viewport and its density. A document is rendered at
/// exact physical resolution whatever the view's size: memory is bounded by the tile cache
/// budget (`viewer_document_tile_cache_bytes`), never by rendering what is shown smaller.
/// Only a view beyond the tile position bound (wider than three 4K screens) is reduced,
/// so that its tiles stay addressable.
pub(crate) fn document_raster(viewport: [u32; 2]) -> ([u32; 2], f64) {
    let [width, height] = viewport.map(|edge| f64::from(edge.max(1)));
    let density = (MAX_VIEW_EDGE / width).min(MAX_VIEW_EDGE / height).min(1.0);
    (
        [width, height].map(|edge| (edge * density).max(1.0) as u32),
        density,
    )
}

#[cfg(test)]
#[path = "image_scene_tests.rs"]
mod tests;
