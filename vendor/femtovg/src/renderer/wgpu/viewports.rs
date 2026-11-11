//! Immutable viewport bindings, owned by one renderer and its fixed device/layout.
use wgpu::util::DeviceExt;

const MAX_VIEWPORTS: usize = 32;

#[derive(Debug)]
struct Entry {
    bits: [u32; 2],
    group: wgpu::BindGroup,
}

#[derive(Debug)]
pub(super) struct ViewportCache {
    layout: wgpu::BindGroupLayout,
    entries: std::collections::VecDeque<Entry>,
    #[cfg(test)]
    materializations: usize,
}

impl ViewportCache {
    pub(super) fn new(layout: wgpu::BindGroupLayout) -> Self {
        Self {
            layout,
            entries: std::collections::VecDeque::with_capacity(MAX_VIEWPORTS),
            #[cfg(test)]
            materializations: 0,
        }
    }

    pub(super) fn get(&mut self, device: &wgpu::Device, viewport: [f32; 2]) -> wgpu::BindGroup {
        let bits = viewport.map(f32::to_bits);
        if let Some(index) = self.entries.iter().position(|entry| entry.bits == bits) {
            let entry = self.entries.remove(index).unwrap();
            let group = entry.group.clone();
            self.entries.push_back(entry);
            return group;
        }
        let group = self.create(device, viewport);
        if self.entries.len() == MAX_VIEWPORTS {
            // WGPU retains bindings already referenced by deferred/in-flight
            // commands. Eviction drops only our ownership, never mutates data.
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            bits,
            group: group.clone(),
        });
        group
    }

    fn create(&mut self, device: &wgpu::Device, viewport: [f32; 2]) -> wgpu::BindGroup {
        let padded = [viewport[0], viewport[1], 0.0, 0.0];
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Uniform Buffer for Viewport"),
            contents: rgb::bytemuck::cast_slice(&padded),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        #[cfg(test)]
        {
            self.materializations += 1;
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Immutable viewport binding"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }
}

#[cfg(test)]
#[path = "viewport_tests.rs"]
mod tests;
