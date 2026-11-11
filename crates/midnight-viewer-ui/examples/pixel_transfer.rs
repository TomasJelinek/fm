//! Isolate the UI's safe raster-buffer copy paths without creating a native window.
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::time::Instant;
fn main() {
    for (width, height) in [(1000, 600), (2250, 1400)] {
        let source: Vec<u8> = (0..width as usize * height as usize * 4)
            .map(|i| (i % 251) as u8)
            .collect();
        for bulk in [false, true] {
            let mut times = Vec::new();
            for _ in 0..50 {
                let start = Instant::now();
                let pixels = if bulk {
                    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
                    pixels.make_mut_bytes().copy_from_slice(&source);
                    pixels
                } else {
                    SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&source, width, height)
                };
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(
                    pixels.as_bytes(),
                    source,
                    "copy must preserve every channel byte"
                );
                std::hint::black_box(pixels);
            }
            times.sort_by(f64::total_cmp);
            println!(
                "width={width} height={height} bulk={bulk} p50_ms={:.3} p95_ms={:.3} max_ms={:.3}",
                times[25], times[47], times[49]
            );
        }
    }
}
