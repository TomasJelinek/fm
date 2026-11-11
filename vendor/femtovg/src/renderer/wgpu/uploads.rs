//! Bounded upload-buffer recycling without reusing in-flight GPU memory.
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

const MAX_UPLOADS: usize = 8;
const MAX_UPLOAD_BYTES: u64 = 2 * 1024 * 1024;
const MAPPED: u8 = 0;
const PENDING: u8 = 1;
const FAILED: u8 = 2;

#[derive(Debug)]
struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}
#[derive(Debug, Default)]
pub(super) struct UploadPool {
    slots: Vec<Slot>,
    #[cfg(test)]
    materializations: usize,
}

/// If an unsubmitted command buffer is discarded, its callback is dropped. Such
/// a source is still unmapped: discard it instead of treating it as writable.
struct Completion(Arc<AtomicU8>);
impl Completion {
    fn complete(self, success: bool) {
        self.0.store(if success { MAPPED } else { FAILED }, Ordering::Release);
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        let _ = self
            .0
            .compare_exchange(PENDING, FAILED, Ordering::AcqRel, Ordering::Acquire);
    }
}

pub(super) struct Upload {
    buffer: wgpu::Buffer,
    uniform_offset: u64,
    uniform_bytes: u64,
    completion: Option<Completion>,
}

fn source(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("FemtoVG upload staging"),
        size,
        usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: true,
    })
}

impl UploadPool {
    fn ready(&self, needed: u64) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.state.load(Ordering::Acquire) == MAPPED && slot.buffer.size() >= needed)
    }

    pub(super) fn begin(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        vertices: &wgpu::Buffer,
        uniforms: &wgpu::Buffer,
        vertex_bytes: &[u8],
        uniform_bytes: u64,
    ) -> Option<Upload> {
        let vertex_len = vertex_bytes.len() as u64;
        let uniform_offset = vertex_len.checked_add(wgpu::MAP_ALIGNMENT - 1)? & !(wgpu::MAP_ALIGNMENT - 1);
        let needed = uniform_offset.checked_add(uniform_bytes)?;
        if needed == 0 || needed > MAX_UPLOAD_BYTES {
            return None;
        }
        self.slots.retain(|slot| slot.state.load(Ordering::Acquire) != FAILED);
        let mut index = self.ready(needed);
        // A returned callback often runs during Queue::submit already. Poll only
        // if pending sources could prevent reuse; never wait for the GPU here.
        if index.is_none()
            && self
                .slots
                .iter()
                .any(|slot| slot.state.load(Ordering::Acquire) == PENDING)
        {
            let _ = device.poll(wgpu::PollType::Poll);
            self.slots.retain(|slot| slot.state.load(Ordering::Acquire) != FAILED);
            index = self.ready(needed);
        }
        let index = match index {
            Some(index) => index,
            None => {
                let vacant = self
                    .slots
                    .iter()
                    .position(|slot| slot.state.load(Ordering::Acquire) == MAPPED);
                if vacant.is_none() && self.slots.len() >= MAX_UPLOADS {
                    return None;
                }
                let slot = Slot {
                    buffer: source(device, needed.max(4096).next_power_of_two()),
                    state: Arc::new(AtomicU8::new(MAPPED)),
                };
                #[cfg(test)]
                {
                    self.materializations += 1;
                }
                match vacant {
                    Some(index) => {
                        self.slots[index] = slot;
                        index
                    }
                    None => {
                        self.slots.push(slot);
                        self.slots.len() - 1
                    }
                }
            }
        };
        let slot = &self.slots[index];
        slot.state.store(PENDING, Ordering::Release);
        let upload = Upload {
            buffer: slot.buffer.clone(),
            uniform_offset,
            uniform_bytes,
            completion: Some(Completion(slot.state.clone())),
        };
        if vertex_len != 0 {
            upload
                .buffer
                .get_mapped_range_mut(..vertex_len)
                .unwrap()
                .copy_from_slice(vertex_bytes);
            encoder.copy_buffer_to_buffer(&upload.buffer, 0, vertices, 0, vertex_len);
        }
        if uniform_bytes != 0 {
            encoder.copy_buffer_to_buffer(&upload.buffer, uniform_offset, uniforms, 0, uniform_bytes);
        }
        Some(upload)
    }

    #[cfg(test)]
    fn statistics(&self) -> (usize, usize, u64) {
        (
            self.materializations,
            self.slots.len(),
            self.slots.iter().map(|slot| slot.buffer.size()).sum(),
        )
    }
}

impl Upload {
    /// A busy/oversized frame uses uncached sources. Inline copies preserve
    /// ordering even when the caller submits several returned command buffers
    /// together. Queue::write_buffer would run before *all* of those commands.
    pub(super) fn one_shot(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        vertices: &wgpu::Buffer,
        uniforms: &wgpu::Buffer,
        vertex_bytes: &[u8],
        uniform_bytes: u64,
    ) -> Self {
        if !vertex_bytes.is_empty() {
            let buffer = source(device, vertex_bytes.len() as u64);
            buffer.get_mapped_range_mut(..).unwrap().copy_from_slice(vertex_bytes);
            buffer.unmap();
            encoder.copy_buffer_to_buffer(&buffer, 0, vertices, 0, vertex_bytes.len() as u64);
        }
        // Nonempty renderer command lists always reserve at least one slot.
        assert!(uniform_bytes != 0);
        let buffer = source(device, uniform_bytes);
        encoder.copy_buffer_to_buffer(&buffer, 0, uniforms, 0, uniform_bytes);
        Self {
            buffer,
            uniform_offset: 0,
            uniform_bytes,
            completion: None,
        }
    }

    pub(super) fn finish(self, encoder: &wgpu::CommandEncoder, uniforms: &[u8]) {
        assert!(uniforms.len() as u64 <= self.uniform_bytes);
        if !uniforms.is_empty() {
            self.buffer
                .get_mapped_range_mut(self.uniform_offset..self.uniform_offset + uniforms.len() as u64)
                .unwrap()
                .copy_from_slice(uniforms);
        }
        self.buffer.unmap();
        if let Some(completion) = self.completion {
            // The source stays unavailable until this command buffer is submitted,
            // all GPU reads finish, and WGPU successfully maps it again.
            encoder.map_buffer_on_submit(&self.buffer, wgpu::MapMode::Write, .., move |result| {
                completion.complete(result.is_ok());
            });
        }
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
