use image::ImageEncoder;
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

pub async fn save(path: PathBuf, width: u32, height: u32, pixels: Vec<u8>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || save_exclusive(&path, width, height, &pixels))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

fn save_exclusive(
    path: &Path,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), image::ImageError> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    image::codecs::png::PngEncoder::new(&mut writer).write_image(
        pixels,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn snapshot_cannot_overwrite_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.png");
        std::fs::write(&path, b"valuable existing contents").unwrap();
        assert!(super::save_exclusive(&path, 1, 1, &[0, 0, 0, 255]).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"valuable existing contents");
    }
}
