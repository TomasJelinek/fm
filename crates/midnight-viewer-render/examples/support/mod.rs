use midnight_viewer_render::Raster;
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

pub fn write_bmp(path: &Path, raster: &Raster) -> io::Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    file.write_all(b"BM")?;
    file.write_all(&(54 + raster.rgba.len() as u32).to_le_bytes())?;
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
    file.flush()
}
