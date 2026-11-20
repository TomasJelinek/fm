//! Time the provider's two opening steps for local files: decoding the retained
//! pixels and rendering the first fitted viewport (default 1000×700 physical pixels).
//! Usage: decode_timing [--viewport WIDTHxHEIGHT] FILE...
use midnight_image_viewer_sdk::{Image, api::Viewport};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    let (mut width, mut height) = (1000u32, 700u32);
    if arguments
        .first()
        .is_some_and(|argument| argument == "--viewport")
    {
        let size = arguments
            .get(1)
            .ok_or("--viewport needs WIDTHxHEIGHT")?
            .clone();
        let (w, h) = size
            .split_once('x')
            .ok_or("--viewport needs WIDTHxHEIGHT")?;
        (width, height) = (w.parse()?, h.parse()?);
        arguments.drain(..2);
    }
    for path in arguments {
        let started = Instant::now();
        let mut file = std::io::BufReader::new(std::fs::File::open(&path)?);
        let image = midnight_viewer_image::decode(&mut file)?;
        let decoded = started.elapsed();
        let metadata = image.metadata();
        let scale = (f64::from(width) / f64::from(metadata.width))
            .min(f64::from(height) / f64::from(metadata.height))
            .min(1.0);
        let viewport = Viewport {
            width: ((f64::from(metadata.width) * scale).round() as u32).max(1),
            height: ((f64::from(metadata.height) * scale).round() as u32).max(1),
            scale,
            origin_x: 0.0,
            origin_y: 0.0,
        };
        let rendering = Instant::now();
        let frame = image.render(viewport)?;
        let rendered = rendering.elapsed();
        println!(
            "{path}: {}x{} decode_ms={:.1} fit_render_ms={:.1} ({}x{} at {scale:.4})",
            metadata.width,
            metadata.height,
            decoded.as_secs_f64() * 1000.0,
            rendered.as_secs_f64() * 1000.0,
            frame.width,
            frame.height,
        );
    }
    Ok(())
}
