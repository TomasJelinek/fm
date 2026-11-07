//! Optional software-renderer artifacts. Test output stays in the caller's build directory.
pub fn snapshot(pixels: &[slint::Rgb8Pixel], width: usize, height: usize, name: &str) {
    let Some(directory) = std::env::var_os("MIDNIGHT_UI_RENDER_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
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
    std::fs::write(directory.join(format!("{name}.bmp")), bmp).unwrap();
}
