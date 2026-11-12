use super::*;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
}
impl Gpu {
    fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
            flags: wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU adapter required");
        eprintln!("viewport cache GPU adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        Self { device, queue, layout }
    }

    fn verify_deferred(&self, groups: Vec<(wgpu::BindGroup, [u32; 4])>) {
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed("@group(0) @binding(0) var<uniform> view: vec4<f32>; @group(1) @binding(0) var<storage, read_write> output: array<u32,4>; @compute @workgroup_size(1) fn main() { for (var i=0u; i<4u; i=i+1u) { output[i]=bitcast<u32>(view[i]); } }")),
        });
        let output_layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&self.layout), Some(&output_layout)],
            immediate_size: 0,
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut commands = Vec::new();
        let mut readbacks = Vec::new();
        for (group, expected) in groups {
            let output = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let output_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &output_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: output.as_entire_binding(),
                }],
            });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.set_bind_group(1, &output_group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 16);
            commands.push(encoder.finish());
            readbacks.push((readback, expected));
            // Both groups and output ownership leave this scope before submission.
        }
        self.queue.submit(commands);
        for (readback, expected) in readbacks {
            let (send, recv) = std::sync::mpsc::channel();
            readback.map_async(wgpu::MapMode::Read, .., move |result| send.send(result).unwrap());
            self.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            recv.recv().unwrap().unwrap();
            let mapped = readback.get_mapped_range(..).unwrap();
            assert_eq!(rgb::bytemuck::cast_slice::<u8, u32>(&mapped), expected);
            drop(mapped);
            readback.unmap();
        }
    }
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn repeated_viewports_reuse_bindings_and_distinct_bits_keep_immutable_values() {
    let gpu = Gpu::new();
    let mut cache = ViewportCache::new(gpu.layout.clone());
    let first = cache.get(&gpu.device, [64.0, 128.0]);
    assert_eq!(
        cache.get(&gpu.device, [64.0, 128.0]),
        first,
        "unchanged viewport must reuse its binding"
    );
    let next = cache.get(&gpu.device, [256.0, 512.0]);
    assert_ne!(first, next);
    let fractional = f32::from_bits(64f32.to_bits() + 1);
    let fine = cache.get(&gpu.device, [fractional, 128.0]);
    assert_ne!(first, fine, "distinct viewport bits must not alias");
    assert_eq!(cache.materializations, 3);
    drop(cache);
    gpu.verify_deferred(vec![
        (first, [64f32.to_bits(), 128f32.to_bits(), 0, 0]),
        (next, [256f32.to_bits(), 512f32.to_bits(), 0, 0]),
        (fine, [fractional.to_bits(), 128f32.to_bits(), 0, 0]),
    ]);
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn viewport_cache_is_bounded_and_evicted_bindings_survive_deferred_submission() {
    let gpu = Gpu::new();
    let mut cache = ViewportCache::new(gpu.layout.clone());
    let oldest = cache.get(&gpu.device, [64.0, 64.0]);
    let warm = cache.get(&gpu.device, [65.0, 64.0]);
    for index in 2..MAX_VIEWPORTS {
        cache.get(&gpu.device, [64.0 + index as f32, 64.0]);
    }
    assert_eq!(cache.get(&gpu.device, [65.0, 64.0]), warm);
    cache.get(&gpu.device, [128.0, 64.0]);
    assert_eq!(cache.entries.len(), MAX_VIEWPORTS);
    assert_eq!(
        cache.get(&gpu.device, [65.0, 64.0]),
        warm,
        "recently used entry must survive eviction"
    );
    assert_ne!(
        cache.get(&gpu.device, [64.0, 64.0]),
        oldest,
        "least recently used entry must be evicted"
    );
    assert_eq!(cache.entries.len(), MAX_VIEWPORTS);
    drop(cache);
    gpu.verify_deferred(vec![
        (oldest, [64f32.to_bits(), 64f32.to_bits(), 0, 0]),
        (warm, [65f32.to_bits(), 64f32.to_bits(), 0, 0]),
    ]);
}
