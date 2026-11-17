use super::*;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}
impl Gpu {
    fn new() -> Self {
        let (device, queue) = super::super::test_gpu::device();
        Self { device, queue }
    }
    fn target(&self, size: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upload regression target"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }
    fn encoder(&self) -> wgpu::CommandEncoder {
        self.device.create_command_encoder(&Default::default())
    }
    fn wait(&self) {
        self.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    fn read(&self, source: &wgpu::Buffer, size: u64) -> Vec<u8> {
        let target = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upload regression readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.encoder();
        encoder.copy_buffer_to_buffer(source, 0, &target, 0, size);
        self.queue.submit([encoder.finish()]);
        let (send, recv) = std::sync::mpsc::channel();
        target.map_async(wgpu::MapMode::Read, .., move |result| send.send(result).unwrap());
        self.wait();
        recv.recv().unwrap().unwrap();
        let bytes = target.get_mapped_range(..).unwrap().to_vec();
        target.unmap();
        bytes
    }
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn uploads_recycle_after_completion_and_preserve_both_regions() {
    let gpu = Gpu::new();
    let mut pool = UploadPool::default();
    let vertices = gpu.target(64);
    let uniforms = gpu.target(64);
    let mut old = None;
    for value in [3_u8, 7, 11] {
        let mut encoder = gpu.encoder();
        let upload = pool
            .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[value; 16], 64)
            .expect("normal frame needs a reusable upload");
        if let Some(previous) = old.replace(upload.buffer.clone()) {
            assert_eq!(upload.buffer, previous);
        }
        upload.finish(&[value + 1; 64]);
        gpu.queue.submit([encoder.finish()]);
        gpu.wait();
        assert_eq!(gpu.read(&vertices, 16), [value; 16]);
        assert_eq!(gpu.read(&uniforms, 64), [value + 1; 64]);
    }
    assert_eq!(pool.statistics().0, 1, "completed buffers must be recycled");
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn unsubmitted_uploads_remain_distinct_and_the_pool_is_bounded() {
    let gpu = Gpu::new();
    let mut pool = UploadPool::default();
    let mut commands = Vec::new();
    let mut targets = Vec::new();
    let mut buffers = Vec::new();
    for index in 0..MAX_UPLOADS {
        let vertices = gpu.target(64);
        let uniforms = gpu.target(64);
        let mut encoder = gpu.encoder();
        let upload = pool
            .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[index as u8; 16], 64)
            .unwrap();
        assert!(
            !buffers.contains(&upload.buffer),
            "unsubmitted data must never be overwritten"
        );
        buffers.push(upload.buffer.clone());
        upload.finish(&[index as u8 + 10; 64]);
        commands.push(encoder.finish());
        targets.push((vertices, uniforms));
    }
    let mut encoder = gpu.encoder();
    assert!(pool
        .begin(&gpu.device, &mut encoder, &targets[0].0, &targets[0].1, &[99; 16], 64)
        .is_none());
    assert_eq!(pool.statistics().1, MAX_UPLOADS);
    assert!(pool.statistics().2 <= MAX_UPLOAD_BYTES * MAX_UPLOADS as u64);
    gpu.queue.submit(commands);
    gpu.wait();
    for (index, (vertices, uniforms)) in targets.iter().enumerate() {
        assert_eq!(gpu.read(vertices, 16), [index as u8; 16]);
        assert_eq!(gpu.read(uniforms, 64), [index as u8 + 10; 64]);
    }
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn nonblocking_poll_in_begin_recycles_a_completed_slot() {
    // Production renders never wait for the GPU: the previous frame's slot is
    // still pending at the next `begin`, which must reuse it only after its own
    // nonblocking poll observed the completed remap.
    let gpu = Gpu::new();
    let mut pool = UploadPool::default();
    let vertices = gpu.target(64);
    let uniforms = gpu.target(64);
    let mut encoder = gpu.encoder();
    let upload = pool
        .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[1; 16], 64)
        .unwrap();
    let first = upload.buffer.clone();
    upload.finish(&[2; 64]);
    gpu.queue.submit([encoder.finish()]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut discarded = 0;
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "a completed slot was never recycled by begin's poll"
        );
        let mut encoder = gpu.encoder();
        let upload = pool
            .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[3; 16], 64)
            .expect("discarded attempts release their slots");
        let reused = upload.buffer == first;
        upload.finish(&[4; 64]);
        if reused {
            gpu.queue.submit([encoder.finish()]);
            break;
        }
        // Not completed yet: discard this attempt; its slot fails and leaves.
        drop(encoder.finish());
        discarded += 1;
    }
    gpu.wait();
    assert_eq!(gpu.read(&vertices, 16), [3; 16]);
    assert_eq!(gpu.read(&uniforms, 64), [4; 64]);
    assert_eq!(pool.statistics().0, 1 + discarded, "only discarded attempts allocate");
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn discarded_commands_and_renderer_drop_do_not_recycle_unmapped_memory() {
    let gpu = Gpu::new();
    let vertices = gpu.target(64);
    let uniforms = gpu.target(64);
    let mut pool = UploadPool::default();
    let mut encoder = gpu.encoder();
    let upload = pool
        .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[4; 16], 64)
        .unwrap();
    let abandoned = upload.buffer.clone();
    upload.finish(&[5; 64]);
    drop(encoder.finish());
    let mut encoder = gpu.encoder();
    let upload = pool
        .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[8; 16], 64)
        .unwrap();
    assert_ne!(upload.buffer, abandoned, "a dropped submission never completed its map");
    upload.finish(&[9; 64]);
    let command = encoder.finish();
    drop(pool);
    gpu.queue.submit([command]);
    gpu.wait();
    assert_eq!(gpu.read(&vertices, 16), [8; 16]);
    assert_eq!(gpu.read(&uniforms, 64), [9; 64]);
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn completed_slots_grow_without_exceeding_the_size_budget() {
    let gpu = Gpu::new();
    let mut pool = UploadPool::default();
    let vertices = gpu.target(8192);
    let uniforms = gpu.target(8192);
    let mut encoder = gpu.encoder();
    let upload = pool
        .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[1; 16], 64)
        .unwrap();
    let small = upload.buffer.clone();
    upload.finish(&[2; 64]);
    gpu.queue.submit([encoder.finish()]);
    gpu.wait();
    let mut encoder = gpu.encoder();
    let upload = pool
        .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[7; 8192], 8192)
        .unwrap();
    assert_ne!(upload.buffer, small);
    upload.finish(&[8; 8192]);
    gpu.queue.submit([encoder.finish()]);
    gpu.wait();
    assert_eq!(gpu.read(&vertices, 8192), [7; 8192]);
    assert_eq!(gpu.read(&uniforms, 8192), [8; 8192]);
    let mut encoder = gpu.encoder();
    assert!(pool
        .begin(
            &gpu.device,
            &mut encoder,
            &vertices,
            &uniforms,
            &[0; 16],
            MAX_UPLOAD_BYTES
        )
        .is_none());
    assert!(pool.statistics().1 <= MAX_UPLOADS);
    assert!(pool.statistics().2 <= MAX_UPLOAD_BYTES * MAX_UPLOADS as u64);
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn cached_and_fallback_uploads_keep_deferred_command_order() {
    let gpu = Gpu::new();
    let mut pool = UploadPool::default();
    let vertices = gpu.target(64);
    let uniforms = gpu.target(64);
    let mut commands = Vec::new();
    let mut captured = Vec::new();
    for index in 0..=MAX_UPLOADS {
        let mut encoder = gpu.encoder();
        let cached = pool.begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[index as u8; 16], 64);
        assert_eq!(cached.is_some(), index < MAX_UPLOADS);
        let upload = cached.unwrap_or_else(|| {
            Upload::one_shot(&gpu.device, &mut encoder, &vertices, &uniforms, &[index as u8; 16], 64)
        });
        upload.finish(&[index as u8 + 20; 64]);
        let vertex_copy = gpu.target(16);
        let uniform_copy = gpu.target(64);
        encoder.copy_buffer_to_buffer(&vertices, 0, &vertex_copy, 0, 16);
        encoder.copy_buffer_to_buffer(&uniforms, 0, &uniform_copy, 0, 64);
        captured.push((vertex_copy, uniform_copy));
        commands.push(encoder.finish());
    }
    gpu.queue.submit(commands);
    gpu.wait();
    for (index, (vertices, uniforms)) in captured.iter().enumerate() {
        assert_eq!(gpu.read(vertices, 16), [index as u8; 16]);
        assert_eq!(gpu.read(uniforms, 64), [index as u8 + 20; 64]);
    }
    assert_eq!(pool.statistics().0, MAX_UPLOADS);
}

#[test]
#[ignore = "requires an actual GPU adapter"]
fn oversized_uploads_use_correct_uncached_storage() {
    let gpu = Gpu::new();
    let mut pool = UploadPool::default();
    let size = MAX_UPLOAD_BYTES + 256;
    let vertices = gpu.target(16);
    let uniforms = gpu.target(size);
    let mut encoder = gpu.encoder();
    assert!(pool
        .begin(&gpu.device, &mut encoder, &vertices, &uniforms, &[42; 16], size)
        .is_none());
    let upload = Upload::one_shot(&gpu.device, &mut encoder, &vertices, &uniforms, &[42; 16], size);
    upload.finish(&vec![73; size as usize]);
    gpu.queue.submit([encoder.finish()]);
    gpu.wait();
    assert_eq!(gpu.read(&vertices, 16), [42; 16]);
    assert!(gpu.read(&uniforms, size).iter().all(|byte| *byte == 73));
    assert_eq!(pool.statistics(), (0, 0, 0));
}
