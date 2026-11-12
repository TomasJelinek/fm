use super::*;
use crate::{renderer::WGPURenderer, Canvas, Color, Paint, Path, PixelFormat};
use imgref::Img;
use rgb::RGBA8;

struct Scene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    canvas: Canvas<WGPURenderer>,
    target: wgpu::Texture,
}

impl Scene {
    fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
            flags: wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("this explicit GPU test requires an adapter");
        eprintln!("binding-cache GPU adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("binding cache regression"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .expect("GPU device");
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("binding cache pixels"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut canvas = Canvas::new(WGPURenderer::new(device.clone(), queue.clone())).unwrap();
        canvas.set_size(64, 64, 1.0);
        Self {
            device,
            queue,
            canvas,
            target,
        }
    }

    fn image(&mut self, color: RGBA8) -> ImageId {
        self.canvas
            .create_image(Img::new(&[color][..], 1, 1), ImageFlags::NEAREST)
            .unwrap()
    }

    fn draw(&mut self, images: &[ImageId]) {
        self.canvas.clear_rect(0, 0, 64, 64, Color::black());
        let mut path = Path::new();
        path.rect(0.0, 0.0, 64.0, 64.0);
        for id in images {
            self.canvas
                .fill_path(&path, &Paint::image(*id, 0.0, 0.0, 64.0, 64.0, 0.0, 1.0));
        }
        self.queue.submit(self.canvas.flush_to_output(&self.target));
        self.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }

    fn allocations(&self) -> usize {
        self.canvas.renderer.bind_group_cache.borrow().materializations
    }

    fn center(&self) -> [u8; 4] {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("binding cache readback"),
            size: 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| sender.send(result).unwrap());
        self.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = buffer.slice(..).get_mapped_range().unwrap();
        let offset = 32 * 256 + 32 * 4;
        let color = mapped[offset..offset + 4].try_into().unwrap();
        drop(mapped);
        buffer.unmap();
        color
    }
}

#[test]
#[ignore = "requires a native GPU; run explicitly with WGPU_BACKEND=dx12 or vulkan"]
fn cached_bindings_reuse_resources_but_show_pixel_updates_and_same_id_reallocation() {
    let mut scene = Scene::new();
    let image = scene.image(RGBA8::new(255, 0, 0, 255));
    scene.draw(&[image]);
    assert_eq!(scene.center(), [255, 0, 0, 255]);
    let allocations = scene.allocations();
    for _ in 0..3 {
        scene.draw(&[image]);
    }
    assert_eq!(scene.allocations(), allocations, "unchanged frames must reuse bindings");

    scene
        .canvas
        .update_image(image, Img::new(&[RGBA8::new(0, 0, 255, 255)][..], 1, 1), 0, 0)
        .unwrap();
    scene.draw(&[image]);
    assert_eq!(scene.center(), [0, 0, 255, 255]);
    assert_eq!(
        scene.allocations(),
        allocations,
        "pixel updates retain the same texture view"
    );

    scene
        .canvas
        .realloc_image(
            image,
            2,
            2,
            PixelFormat::Rgba8,
            ImageFlags::NEAREST | ImageFlags::REPEAT_X,
        )
        .unwrap();
    let green = [RGBA8::new(0, 255, 0, 255); 4];
    scene
        .canvas
        .update_image(image, Img::new(&green[..], 2, 2), 0, 0)
        .unwrap();
    scene.draw(&[image]);
    assert_eq!(scene.center(), [0, 255, 0, 255], "same id must bind the new texture");
    assert!(scene.allocations() > allocations);
    scene.canvas.delete_image(image);
    assert!(!scene
        .canvas
        .renderer
        .bind_group_cache
        .borrow()
        .entries
        .keys()
        .any(|key| key.references(image)));
}

#[test]
#[ignore = "requires a native GPU; run explicitly with WGPU_BACKEND=dx12 or vulkan"]
fn uniform_arena_growth_rebinds_the_new_buffer_and_retains_correct_pixels() {
    let mut scene = Scene::new();
    let image = scene.image(RGBA8::new(255, 0, 0, 255));
    scene.draw(&[image]);
    let old_buffer = scene.canvas.renderer.bind_group_cache.borrow().uniform_buffer.clone();
    let initial = scene.allocations();
    let many = vec![image; 300];
    scene.draw(&many);
    assert_ne!(
        scene.canvas.renderer.bind_group_cache.borrow().uniform_buffer,
        old_buffer
    );
    assert!(
        scene.allocations() > initial,
        "growth must rebuild bindings against the new buffer"
    );
    assert_eq!(scene.center(), [255, 0, 0, 255]);
    let grown = scene.allocations();
    scene.draw(&many);
    assert_eq!(scene.allocations(), grown);
    assert_eq!(scene.center(), [255, 0, 0, 255]);
}

#[test]
#[ignore = "requires a native GPU; run explicitly with WGPU_BACKEND=dx12 or vulkan"]
fn binding_cache_evicts_oldest_entries_and_prunes_unused_reallocated_images() {
    let mut scene = Scene::new();
    let images: Vec<_> = (0..MAX_BIND_GROUPS + 1)
        .map(|_| scene.image(RGBA8::new(0, 0, 255, 255)))
        .collect();
    scene.draw(&images);
    {
        let cache = scene.canvas.renderer.bind_group_cache.borrow();
        assert_eq!(cache.entries.len(), MAX_BIND_GROUPS);
        assert!(!cache.entries.keys().any(|key| key.references(images[0])));
        assert!(cache.entries.keys().any(|key| key.references(*images.last().unwrap())));
    }
    let old = *images.last().unwrap();
    scene
        .canvas
        .realloc_image(old, 2, 2, PixelFormat::Rgba8, ImageFlags::NEAREST)
        .unwrap();
    scene.draw(&[images[0]]);
    let cache = scene.canvas.renderer.bind_group_cache.borrow();
    assert!(cache.entries.len() <= MAX_BIND_GROUPS);
    assert!(
        !cache.entries.keys().any(|key| key.references(old)),
        "undrawn old texture must be released"
    );
    drop(cache);
    assert_eq!(scene.center(), [0, 0, 255, 255]);
}
