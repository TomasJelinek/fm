//! Optional software-renderer artifacts, written only when `MIDNIGHT_UI_RENDER_DIR`
//! names a directory (and the name contains `MIDNIGHT_UI_RENDER_FILTER`, when set).
//! Test output stays in the caller's build directory. Each file name starts with the
//! running test's name, so tests that run in parallel never write the same file.
#![allow(dead_code)]

/// Whether a snapshot with this name would be written; lets callers skip rendering
/// a frame that exists only for the artifact.
pub fn enabled(name: &str) -> bool {
    directory(name).is_some()
}

fn directory(name: &str) -> Option<std::path::PathBuf> {
    let directory = std::env::var_os("MIDNIGHT_UI_RENDER_DIR")?;
    if directory.is_empty()
        || std::env::var("MIDNIGHT_UI_RENDER_FILTER").is_ok_and(|filter| !name.contains(&filter))
    {
        return None;
    }
    Some(std::path::PathBuf::from(directory))
}

pub fn snapshot(pixels: &[slint::Rgb8Pixel], width: usize, height: usize, name: &str) {
    let Some(directory) = directory(name) else {
        return;
    };
    std::fs::create_dir_all(&directory).unwrap();
    let stride = (width * 3).next_multiple_of(4);
    let mut bmp = vec![0_u8; 54 + stride * height];
    bmp[..2].copy_from_slice(b"BM");
    let length = bmp.len() as u32;
    bmp[2..6].copy_from_slice(&length.to_le_bytes());
    bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&(width as u32).to_le_bytes());
    bmp[22..26].copy_from_slice(&(height as u32).to_le_bytes());
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    for y in 0..height {
        for x in 0..width {
            let p = pixels[y * width + x];
            let offset = 54 + (height - y - 1) * stride + x * 3;
            bmp[offset..offset + 3].copy_from_slice(&[p.b, p.g, p.r]);
        }
    }
    // libtest names each test thread after its test; keep it file-name safe.
    let test = std::thread::current()
        .name()
        .unwrap_or("test")
        .replace("::", ".")
        .replace(
            |c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
            "_",
        );
    std::fs::write(directory.join(format!("{test}--{name}.bmp")), bmp).unwrap();
}
