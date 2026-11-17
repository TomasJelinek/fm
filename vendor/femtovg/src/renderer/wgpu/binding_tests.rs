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
        // Cached groups across replacement, deletion and arena growth are the
        // riskiest cases; the shared device runs under the validation layer.
        let (device, queue) = super::super::test_gpu::device();
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

    fn uncached(&self) -> usize {
        self.canvas.renderer.bind_group_cache.borrow().uncached
    }

    fn cached(&self, image: ImageId) -> bool {
        self.canvas
            .renderer
            .bind_group_cache
            .borrow()
            .entries
            .keys()
            .any(|key| key.references(image))
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
fn binding_cache_evicts_entries_of_earlier_frames_and_prunes_unused_reallocated_images() {
    let mut scene = Scene::new();
    let images: Vec<_> = (0..MAX_BIND_GROUPS + 1)
        .map(|_| scene.image(RGBA8::new(0, 0, 255, 255)))
        .collect();
    // The clear plus these images fill the cache in one frame.
    scene.draw(&images[1..MAX_BIND_GROUPS]);
    assert_eq!(
        scene.canvas.renderer.bind_group_cache.borrow().entries.len(),
        MAX_BIND_GROUPS
    );
    assert!(scene.cached(images[1]));
    // A later frame replaces the least recently used binding of an earlier one.
    scene.draw(&[images[0]]);
    assert_eq!(
        scene.canvas.renderer.bind_group_cache.borrow().entries.len(),
        MAX_BIND_GROUPS
    );
    assert!(scene.cached(images[0]));
    assert!(!scene.cached(images[1]));
    let old = images[2];
    scene
        .canvas
        .realloc_image(old, 2, 2, PixelFormat::Rgba8, ImageFlags::NEAREST)
        .unwrap();
    scene.draw(&[images[0]]);
    assert!(!scene.cached(old), "undrawn old texture must be released");
    assert_eq!(scene.center(), [0, 0, 255, 255]);
}

#[test]
#[ignore = "requires a native GPU; run explicitly with WGPU_BACKEND=dx12 or vulkan"]
fn frames_beyond_the_cap_keep_their_cached_bindings_and_leave_only_the_overflow_uncached() {
    let mut scene = Scene::new();
    let images: Vec<_> = (0..MAX_BIND_GROUPS + 40)
        .map(|index| scene.image(RGBA8::new(0, (index % 200) as u8, 255, 255)))
        .collect();
    scene.draw(&images);
    let cached = scene.allocations();
    let overflow = scene.uncached();
    assert!(overflow > 0, "the frame uses more bindings than the cap");
    for _ in 0..2 {
        scene.draw(&images);
        // A global LRU would evict each binding just before its next use.
        assert_eq!(scene.allocations(), cached, "cached bindings must stay hits");
        assert_eq!(scene.uncached() % overflow, 0);
    }
    assert_eq!(scene.uncached(), 3 * overflow);
    let last = images.last().unwrap();
    assert_eq!(scene.center(), [0, ((images.len() - 1) % 200) as u8, 255, 255]);
    assert!(!scene.cached(*last), "the overflow is not admitted mid-frame");
}

#[test]
#[ignore = "requires a native GPU; run explicitly with WGPU_BACKEND=dx12 or vulkan"]
fn images_deleted_before_flush_are_drawn_without_caching_their_ids() {
    let mut scene = Scene::new();
    let image = scene.image(RGBA8::new(255, 0, 0, 255));
    scene.canvas.clear_rect(0, 0, 64, 64, Color::black());
    let mut path = Path::new();
    path.rect(0.0, 0.0, 64.0, 64.0);
    scene
        .canvas
        .fill_path(&path, &Paint::image(image, 0.0, 0.0, 64.0, 64.0, 0.0, 1.0));
    scene.canvas.delete_image(image);
    scene.queue.submit(scene.canvas.flush_to_output(&scene.target));
    scene.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert!(
        !scene.cached(image),
        "a binding for a deleted id can never be hit again"
    );
    assert!(scene.uncached() > 0);
}
