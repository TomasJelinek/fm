//! Bounded reuse of immutable texture/sampler bindings between frames.
//!
//! DX12 allocates an internal sampler-index buffer for each bind group. Creating
//! the same groups for every alternating image/text draw causes allocation and
//! GPU synchronization work even when the underlying images have not changed.
//! That buffer is a separate 64 KiB upload-heap placement on DX12, so the cap
//! below also bounds retained CPU-visible memory (512 x 64 KiB = 32 MiB).
use super::{BindGroupState, CommandToPipelineAndBindGroupMapper, Image, ImageOrTexture, Texture};
use crate::{image::ImageStore, ImageFlags, ImageId};
use std::collections::HashMap;

const MAX_BIND_GROUPS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    image: Option<ImageId>,
    glyph: Option<ImageId>,
}

impl Key {
    fn from_state(state: &BindGroupState) -> Option<Self> {
        // Unmanaged filter textures have no delete_image notification. Never
        // retain their potentially large backing allocations in this cache.
        let image = match &state.image {
            Some(ImageOrTexture::Image(id)) => Some(*id),
            Some(ImageOrTexture::Texture(_)) => return None,
            None => None,
        };
        Some(Self {
            image,
            glyph: state.glyph_texture.image_id(),
        })
    }

    fn references(self, image: ImageId) -> bool {
        self.image == Some(image) || self.glyph == Some(image)
    }
}

#[derive(Debug)]
struct Entry {
    group: wgpu::BindGroup,
    resources: [Option<Resource>; 2],
    last_used: u64,
    /// Render in which this entry was last used.
    frame: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct Resource {
    texture: wgpu::Texture,
    flags: ImageFlags,
}

/// Owned resource identity for a new entry. External textures and ids that are
/// no longer in the store (deleted before this flush) are never cached.
fn resource(images: &ImageStore<Image>, id: Option<ImageId>) -> Result<Option<Resource>, ()> {
    let Some(id) = id else {
        return Ok(None);
    };
    match images.get(id) {
        Some(Image {
            texture: Texture::Internal(texture),
            info,
            ..
        }) => Ok(Some(Resource {
            texture: texture.clone(),
            flags: info.flags(),
        })),
        _ => Err(()),
    }
}

/// Validates a cached resource by reference, without cloning texture handles.
fn matches_resource(images: &ImageStore<Image>, id: Option<ImageId>, saved: &Option<Resource>) -> bool {
    match (id, saved) {
        (None, None) => true,
        (Some(id), Some(saved)) => images.get(id).is_some_and(|image| {
            matches!(&image.texture, Texture::Internal(texture)
                if *texture == saved.texture && image.info.flags() == saved.flags)
        }),
        _ => false,
    }
}

#[derive(Debug, Default)]
pub(super) struct BindGroupCache {
    entries: HashMap<Key, Entry>,
    uniform_buffer: Option<wgpu::Buffer>,
    clock: u64,
    frame: u64,
    /// Number of entries already used in the current frame.
    used_this_frame: usize,
    #[cfg(test)]
    materializations: usize,
    #[cfg(test)]
    uncached: usize,
}

impl BindGroupCache {
    pub(super) fn get(
        &mut self,
        state: &BindGroupState,
        images: &ImageStore<Image>,
        mapper: &CommandToPipelineAndBindGroupMapper,
    ) -> wgpu::BindGroup {
        let materialize = || {
            state.materialize(
                &mapper.device,
                images,
                &mapper.bind_group_layout,
                &mapper.empty_texture_view,
                &mapper.sampler_cache,
                &mapper.uniform_buffer,
            )
        };
        let Some(key) = Key::from_state(state) else {
            return materialize();
        };
        // Growing the uniform arena replaces its GPU buffer. Cached bindings
        // must reference the replacement before any new dynamic offsets are used.
        if self.uniform_buffer.as_ref() != Some(&mapper.uniform_buffer) {
            self.clear();
            self.uniform_buffer = Some(mapper.uniform_buffer.clone());
        }
        if self.clock == u64::MAX {
            self.clear();
            self.clock = 0;
        }
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            // ImageStore::realloc preserves the id and directly replaces the
            // renderer image. Validate handles and sampler flags, not only ids.
            if matches_resource(images, key.image, &entry.resources[0])
                && matches_resource(images, key.glyph, &entry.resources[1])
            {
                entry.last_used = self.clock;
                if entry.frame != self.frame {
                    entry.frame = self.frame;
                    self.used_this_frame += 1;
                }
                return entry.group.clone();
            }
            if let Some(entry) = self.entries.remove(&key) {
                self.forget(&entry);
            }
        }
        let (Ok(image), Ok(glyph)) = (resource(images, key.image), resource(images, key.glyph)) else {
            return self.uncached(materialize);
        };
        if self.entries.len() >= MAX_BIND_GROUPS {
            // A frame that uses more distinct bindings than the cap accesses them
            // cyclically. Evicting one used in this frame would only force its
            // rebuild later in the same frame; leave the overflow uncached as
            // upstream does. Otherwise the least recently used entry is from an
            // earlier frame.
            if self.used_this_frame >= self.entries.len() {
                return self.uncached(materialize);
            }
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(key, _)| *key)
            {
                if let Some(entry) = self.entries.remove(&oldest) {
                    self.forget(&entry);
                }
            }
        }
        let group = materialize();
        #[cfg(test)]
        {
            self.materializations += 1;
        }
        self.used_this_frame += 1;
        self.entries.insert(
            key,
            Entry {
                group: group.clone(),
                resources: [image, glyph],
                last_used: self.clock,
                frame: self.frame,
            },
        );
        group
    }

    fn uncached(&mut self, materialize: impl FnOnce() -> wgpu::BindGroup) -> wgpu::BindGroup {
        #[cfg(test)]
        {
            self.uncached += 1;
        }
        materialize()
    }

    fn forget(&mut self, entry: &Entry) {
        if entry.frame == self.frame {
            self.used_this_frame -= 1;
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.used_this_frame = 0;
    }

    pub(super) fn remove_image(&mut self, image: ImageId) {
        self.entries.retain(|key, _| !key.references(image));
        let frame = self.frame;
        self.used_this_frame = self.entries.values().filter(|entry| entry.frame == frame).count();
    }

    /// Called once after each render: ends the current frame.
    pub(super) fn prune_replaced_images(&mut self, images: &ImageStore<Image>) {
        // Also release bindings for reallocated or deleted ids that were not drawn
        // again. Pixel-only updates keep the same resource and remain cache hits.
        self.entries.retain(|key, entry| {
            matches_resource(images, key.image, &entry.resources[0])
                && matches_resource(images, key.glyph, &entry.resources[1])
        });
        self.frame = self.frame.wrapping_add(1);
        self.used_this_frame = 0;
    }
}

#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
