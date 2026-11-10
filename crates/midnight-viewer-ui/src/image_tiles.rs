//! Immutable Slint textures are reused by ID; panning only changes GPU coordinates.
use crate::{ViewerRasterTile, image_camera::grid_bounds};
use midnight_core::{ViewerImageInfo, ViewerSurfaceId, ViewerTile, ViewerTileId};
use slint::{Image, Model, ModelRc, SharedPixelBuffer, VecModel};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

// At most32MiB in the cache/model and another32MiB in the previous Slint
// model while a repaint replaces its items. Driver-deferred GPU releases are separate.
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
const EDGE: f64 = 256.0;

struct Texture {
    image: Image,
    touched: u64,
    bytes: usize,
    width: u32,
    height: u32,
}

#[derive(Default)]
pub(super) struct Textures {
    textures: HashMap<ViewerTileId, Texture>,
    positions: HashMap<[i32; 2], ViewerTileId>,
    surface: Option<ViewerSurfaceId>,
    clock: u64,
    bytes: usize,
    pub uploads: u64,
    model: Rc<VecModel<ViewerRasterTile>>,
    rows: Vec<(ViewerTileId, [i32; 2])>,
    base: Option<[f64; 2]>,
}

impl Textures {
    pub fn base(&self) -> Option<[f64; 2]> {
        self.base
    }
    pub fn grid(&self) -> HashSet<[i32; 2]> {
        self.positions.keys().copied().collect()
    }

    pub fn receive(
        &mut self,
        tiles: &[ViewerTile],
        info: &ViewerImageInfo,
        displayed: [f64; 2],
        replace_camera: bool,
    ) -> Result<ModelRc<ViewerRasterTile>, String> {
        if tiles.len() > 128 {
            return Err("Image scene exceeds its tile bound.".into());
        }
        let same_surface = self.surface == Some(info.surface);
        if !same_surface || replace_camera {
            self.positions.clear();
            self.surface = Some(info.surface);
        }
        let rebase = !same_surface
            || self.base.is_none_or(|base| {
                (0..2).any(|axis| {
                    ((displayed[axis] - base[axis]) / info.visible_source[axis]).abs() > 64.0
                })
            });
        if rebase {
            self.base = Some(info.origin);
        }
        let (low, high) = grid_bounds(displayed, info);
        let mut protected: HashSet<_> = self
            .positions
            .iter()
            .filter(|(position, _)| {
                position[0] >= low[0]
                    && position[0] < high[0]
                    && position[1] >= low[1]
                    && position[1] < high[1]
            })
            .map(|(_, id)| *id)
            .collect();
        let (next_low, next_high) = grid_bounds(info.origin, info);
        protected.extend(
            tiles
                .iter()
                .filter(|tile| {
                    let position = coordinate(tile, info);
                    position[0] >= next_low[0]
                        && position[0] < next_high[0]
                        && position[1] >= next_low[1]
                        && position[1] < next_high[1]
                })
                .map(|tile| tile.id),
        );
        for tile in tiles {
            tile.validate().map_err(str::to_owned)?;
            if tile.width != 256 || tile.height != 256 {
                return Err("Unexpected image tile dimensions.".into());
            }
            self.clock = self.clock.saturating_add(1);
            if let Some(texture) = self.textures.get_mut(&tile.id) {
                if texture.width != tile.width || texture.height != tile.height {
                    return Err("Image tile identity changed.".into());
                }
                texture.touched = self.clock;
            } else {
                let bytes = tile.pixels.rgba().len();
                while self.textures.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
                    let oldest = self
                        .textures
                        .iter()
                        .filter(|(id, _)| !protected.contains(id))
                        .min_by_key(|(_, texture)| texture.touched)
                        .map(|(id, _)| *id);
                    let Some(oldest) = oldest else {
                        return Err("Image texture working set exceeds its budget.".into());
                    };
                    if let Some(old) = self.textures.remove(&oldest) {
                        self.bytes -= old.bytes;
                    }
                }
                let pixels = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                    tile.pixels.rgba(),
                    tile.width,
                    tile.height,
                );
                self.textures.insert(
                    tile.id,
                    Texture {
                        image: Image::from_rgba8_premultiplied(pixels),
                        touched: self.clock,
                        bytes,
                        width: tile.width,
                        height: tile.height,
                    },
                );
                self.bytes += bytes;
                self.uploads = self.uploads.saturating_add(1);
            }
            let position = coordinate(tile, info);
            self.positions.insert(position, tile.id);
        }
        self.positions
            .retain(|_, id| self.textures.contains_key(id));
        let dimensions = info.visible_source.map(|extent| extent * info.raster_scale);
        let wanted: HashSet<_> = self.positions.values().copied().collect();
        for index in (0..self.rows.len()).rev() {
            if !wanted.contains(&self.rows[index].0) {
                self.rows.remove(index);
                self.model.remove(index);
            }
        }
        let base = self.base.unwrap_or(info.origin);
        let mut positions: Vec<_> = self
            .positions
            .iter()
            .map(|(position, id)| (*id, *position))
            .collect();
        positions.sort_unstable_by_key(|(id, _)| id.0);
        for (id, position) in positions {
            let previous = self.rows.iter().position(|(old, _)| *old == id);
            if previous.is_some_and(|index| !rebase && self.rows[index].1 == position) {
                continue;
            }
            if let Some(texture) = self.textures.get(&id) {
                let row = ViewerRasterTile {
                    image: texture.image.clone(),
                    x: ((f64::from(position[0]) * EDGE - base[0] * info.raster_scale)
                        / dimensions[0]) as f32,
                    y: ((f64::from(position[1]) * EDGE - base[1] * info.raster_scale)
                        / dimensions[1]) as f32,
                    width: (EDGE / dimensions[0]) as f32,
                    height: (EDGE / dimensions[1]) as f32,
                };
                if let Some(index) = previous {
                    self.rows[index] = (id, position);
                    self.model.set_row_data(index, row);
                } else {
                    self.rows.push((id, position));
                    self.model.push(row);
                }
            }
        }
        Ok(self.model.clone().into())
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

fn coordinate(tile: &ViewerTile, info: &ViewerImageInfo) -> [i32; 2] {
    std::array::from_fn(|axis| {
        ((f64::from(tile.position[axis]) + info.origin[axis] * info.raster_scale) / EDGE).round()
            as i32
    })
}

#[cfg(test)]
#[path = "image_tile_cache_tests.rs"]
mod tests;
