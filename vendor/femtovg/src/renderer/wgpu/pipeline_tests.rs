use super::{PipelineCache, MAX_PIPELINES};
use crate::{renderer::WGPURenderer, Canvas, Color, Paint, Path};

fn cache() -> PipelineCache<u32, u32> {
    PipelineCache::default()
}

/// Uses `key` in its own flush.
fn use_in_flush(cache: &mut PipelineCache<u32, u32>, key: u32) {
    cache.get_or_insert_with(key, || key);
    cache.end_flush();
}

#[test]
fn a_pipeline_survives_flushes_that_do_not_use_it() {
    let mut cache = cache();
    use_in_flush(&mut cache, 1);
    for _ in 0..1000 {
        use_in_flush(&mut cache, 2);
    }
    assert_eq!(*cache.get_or_insert_with(1, || unreachable!("rebuilt")), 1);
    assert_eq!(cache.materializations(), 2);
}

#[test]
fn past_the_cap_the_least_recently_used_pipelines_are_dropped() {
    let mut cache = cache();
    let total = MAX_PIPELINES as u32 + 3;
    for key in 0..total {
        use_in_flush(&mut cache, key);
    }
    assert_eq!(cache.len(), MAX_PIPELINES);
    assert!((0..3).all(|key| !cache.contains(&key)));
    assert!((3..total).all(|key| cache.contains(&key)));
}

#[test]
fn reuse_makes_a_pipeline_recent_again() {
    let mut cache = cache();
    for key in 0..MAX_PIPELINES as u32 {
        use_in_flush(&mut cache, key);
    }
    cache.get_or_insert_with(0, || unreachable!("rebuilt"));
    cache.end_flush();
    use_in_flush(&mut cache, 1000);
    assert!(cache.contains(&0), "the reused pipeline stays");
    assert!(!cache.contains(&1), "the least recently used one goes");
    assert_eq!(cache.len(), MAX_PIPELINES);
}

#[test]
fn pipelines_of_the_current_flush_are_never_dropped() {
    let mut cache = cache();
    let many = MAX_PIPELINES as u32 + 8;
    for key in 0..many {
        cache.get_or_insert_with(key, || key);
    }
    cache.end_flush();
    assert_eq!(
        cache.len(),
        many as usize,
        "one flush over the cap keeps all its pipelines"
    );
    for key in 0..3 {
        cache.get_or_insert_with(key, || unreachable!("rebuilt"));
    }
    cache.end_flush();
    assert_eq!(cache.len(), MAX_PIPELINES);
    assert!((0..3).all(|key| cache.contains(&key)));
    assert_eq!(cache.materializations(), many as usize);
}

struct Scene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    canvas: Canvas<WGPURenderer>,
    target: wgpu::Texture,
}

impl Scene {
    fn new() -> Self {
        let (device, queue) = super::super::test_gpu::device();
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pipeline cache pixels"),
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

    fn flush(&mut self) {
        self.queue.submit(self.canvas.flush_to_output(&self.target));
        self.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }

    /// The separate clear flush Slint makes whenever a rendering notifier is set.
    fn clear(&mut self) {
        self.canvas.clear_rect(0, 0, 64, 64, Color::black());
        self.flush();
    }

    /// An L shape: a concave fill, drawn with stencil and cover pipelines.
    fn fill_concave(&mut self) {
        let mut path = Path::new();
        path.move_to(8.0, 8.0);
        path.line_to(56.0, 8.0);
        path.line_to(56.0, 24.0);
        path.line_to(24.0, 24.0);
        path.line_to(24.0, 56.0);
        path.line_to(8.0, 56.0);
        path.close();
        self.canvas.fill_path(&path, &Paint::color(Color::rgb(255, 0, 0)));
        self.flush();
    }

    fn cached(&self) -> usize {
        self.canvas.renderer.pipeline_cache.borrow().len()
    }

    fn materializations(&self) -> usize {
        self.canvas.renderer.pipeline_cache.borrow().materializations()
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pipeline cache readback"),
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
        let offset = (y * 256 + x * 4) as usize;
        let color = mapped[offset..offset + 4].try_into().unwrap();
        drop(mapped);
        buffer.unmap();
        color
    }
}

#[test]
#[ignore = "requires a native GPU; run explicitly with WGPU_BACKEND=dx12 or vulkan"]
fn pipelines_survive_a_flush_that_did_not_use_them() {
    let mut scene = Scene::new();
    // The first frame, as Slint draws it with a rendering notifier: a clear-only
    // flush, then the content.
    scene.clear();
    scene.fill_concave();
    let drawn = scene.cached();
    assert!(drawn >= 2, "a concave fill uses stencil and cover pipelines");
    let materialized = scene.materializations();
    for frame in 0..3 {
        scene.clear();
        assert!(
            scene.cached() >= drawn,
            "frame {frame}: the clear-only flush left {} of {drawn} pipelines",
            scene.cached()
        );
        scene.fill_concave();
    }
    assert_eq!(scene.materializations(), materialized, "later frames rebuilt pipelines");
    assert_eq!(scene.pixel(16, 16), [255, 0, 0, 255], "inside the L");
    assert_eq!(scene.pixel(40, 40), [0, 0, 0, 255], "in its notch");
}
