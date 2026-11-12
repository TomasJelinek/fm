//! Bounded reuse of immutable texture/sampler bindings between frames.
//!
//! DX12 allocates an internal sampler-index buffer for each bind group. Creating
//! the same groups for every alternating image/text draw causes allocation and
//! GPU synchronization work even when the underlying images have not changed.
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
}

#[derive(Clone, Debug, PartialEq)]
struct Resource {
    texture: wgpu::Texture,
    flags: ImageFlags,
}

fn resource(images: &ImageStore<Image>, id: Option<ImageId>) -> Result<Option<Resource>, ()> {
    match id.and_then(|id| images.get(id)) {
        Some(image) => match &image.texture {
            Texture::Internal(texture) => Ok(Some(Resource {
                texture: texture.clone(),
                flags: image.info.flags(),
            })),
            Texture::External(_) => Err(()),
        },
        None => Ok(None),
    }
}

fn matches_resource(images: &ImageStore<Image>, id: Option<ImageId>, saved: &Option<Resource>) -> bool {
    match (id.and_then(|id| images.get(id)), saved) {
        (None, None) => true,
        (Some(image), Some(saved)) => matches!(&image.texture, Texture::Internal(texture)
            if *texture == saved.texture && image.info.flags() == saved.flags),
        _ => false,
    }
}

#[derive(Debug, Default)]
pub(super) struct BindGroupCache {
    entries: HashMap<Key, Entry>,
    uniform_buffer: Option<wgpu::Buffer>,
    clock: u64,
    #[cfg(test)]
    materializations: usize,
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
        let (Ok(image), Ok(glyph)) = (resource(images, key.image), resource(images, key.glyph)) else {
            return materialize();
        };
        let resources = [image, glyph];
        // Growing the uniform arena replaces its GPU buffer. Cached bindings
        // must reference the replacement before any new dynamic offsets are used.
        if self.uniform_buffer.as_ref() != Some(&mapper.uniform_buffer) {
            self.entries.clear();
            self.uniform_buffer = Some(mapper.uniform_buffer.clone());
        }
        if self.clock == u64::MAX {
            self.entries.clear();
            self.clock = 0;
        }
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.resources == resources {
                entry.last_used = self.clock;
                return entry.group.clone();
            }
            // ImageStore::realloc preserves the id and directly replaces the
            // renderer image. Validate handles and sampler flags, not only ids.
            self.entries.remove(&key);
        }
        if self.entries.len() >= MAX_BIND_GROUPS {
            let oldest = *self.entries.iter().min_by_key(|(_, e)| e.last_used).unwrap().0;
            self.entries.remove(&oldest);
        }
        let group = materialize();
        #[cfg(test)]
        {
            self.materializations += 1;
        }
        self.entries.insert(
            key,
            Entry {
                group: group.clone(),
                resources,
                last_used: self.clock,
            },
        );
        group
    }

    pub(super) fn remove_image(&mut self, image: ImageId) {
        self.entries.retain(|key, _| !key.references(image));
    }

    pub(super) fn prune_replaced_images(&mut self, images: &ImageStore<Image>) {
        // Also release bindings for reallocated ids that were not drawn again.
        // Pixel-only updates keep the same resource and remain cache hits.
        self.entries.retain(|key, entry| {
            matches_resource(images, key.image, &entry.resources[0])
                && matches_resource(images, key.glyph, &entry.resources[1])
        });
    }
}

#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
