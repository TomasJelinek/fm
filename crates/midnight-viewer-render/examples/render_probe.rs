//! Reproducible renderer timing and image evidence; output stays below `_build`.
use midnight_viewer_render::{TextOptions, TextRenderer, Viewport};
use std::{
    fs::File,
    io::{BufWriter, Write},
    time::Instant,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut renderer = TextRenderer::default();
    let source = "Source: fn main() { println!(\"Hello\"); }\r\n\
        Unicode: Příliš žluťoučký kůň — 文档 日本語\r\n\
        RTL: العربية עברית end\r\n\
        Combining: e\u{301} a\u{308}; emoji: 👩\u{200d}💻 🌍\r\n\
        Tabs:\tname\tvalue\r\n\
        Copy preserves the original tabs and CRLF.\r\n";
    let view = Viewport {
        width: 1280,
        height: 720,
        origin: [0.0; 2],
        background: [24, 27, 34, 255],
        selection_color: [45, 80, 140, 255],
    };
    let started = Instant::now();
    let page = renderer.layout_with_options(
        source,
        &TextOptions {
            size: 28.0,
            ..TextOptions::default()
        },
        &[],
    )?;
    println!("cold layout: {:?}", started.elapsed());
    let start = source.find("العربية").unwrap();
    let raster = renderer.rasterize(&page, view, Some(start..start + "العربية".len()))?;
    if let Some(path) = std::env::args_os().nth(1) {
        let mut file = BufWriter::new(File::create(path)?);
        let length = 54 + raster.rgba.len() as u32;
        file.write_all(b"BM")?;
        file.write_all(&length.to_le_bytes())?;
        file.write_all(&[0; 4])?;
        file.write_all(&54u32.to_le_bytes())?;
        file.write_all(&40u32.to_le_bytes())?;
        file.write_all(&(raster.width as i32).to_le_bytes())?;
        file.write_all(&(-(raster.height as i32)).to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&32u16.to_le_bytes())?;
        file.write_all(&[0; 24])?;
        for pixel in raster.rgba.as_chunks::<4>().0 {
            file.write_all(&[pixel[2], pixel[1], pixel[0], 255])?;
        }
        file.flush()?;
    }
    for (name, text) in [
        ("4096 lines", "abcdef 1234567890\n".repeat(4095)),
        ("256 KiB line", "abcdefgh".repeat(32768)),
    ] {
        let started = Instant::now();
        let page = renderer.layout(&text, 18.0, 1.0)?;
        println!("{name} layout: {:?}", started.elapsed());
        let started = Instant::now();
        for frame in 0..10 {
            let mut view = view;
            view.origin = [frame as f32 * 3.0, frame as f32 * 2.0];
            std::hint::black_box(renderer.rasterize(&page, view, None)?);
        }
        println!("{name} ten frames: {:?}", started.elapsed());
    }
    Ok(())
}
