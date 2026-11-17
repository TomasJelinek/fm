fn chunk(tag: &[u8; 4], bytes: &[u8]) -> Vec<u8> {
    let mut result = tag.to_vec();
    result.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    result.extend_from_slice(bytes);
    if !bytes.len().is_multiple_of(2) {
        result.push(0);
    }
    result
}
fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = kind.to_vec();
    for chunk in chunks {
        bytes.extend_from_slice(chunk);
    }
    chunk(b"LIST", &bytes)
}
fn word(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
/// The BGR color of uncompressed frame `index`.
pub fn color(index: u8) -> [u8; 3] {
    [index.wrapping_mul(40), 255 - index.wrapping_mul(40), 30]
}
/// Authored 2x2, six-frame uncompressed AVI fixture; no downloaded media or external encoder.
pub fn video() -> Vec<u8> {
    sized_video(2, 2, 6, 6)
}
/// Authored uncompressed 6 fps AVI with `frames` solid-color frames. Its headers
/// declare `declared` frames, which may overstate the content like a partial download.
pub fn sized_video(width: u32, height: u32, frames: u8, declared: u32) -> Vec<u8> {
    let row = (width as usize * 3).next_multiple_of(4);
    let frame_bytes = (row * height as usize) as u32;
    let mut header = vec![0; 56];
    word(&mut header, 0, 166_667);
    word(&mut header, 16, declared);
    word(&mut header, 24, 1);
    word(&mut header, 32, width);
    word(&mut header, 36, height);
    let mut stream = vec![0; 56];
    stream[..4].copy_from_slice(b"vids");
    stream[4..8].copy_from_slice(b"DIB ");
    word(&mut stream, 20, 1);
    word(&mut stream, 24, 6);
    word(&mut stream, 32, declared);
    word(&mut stream, 36, frame_bytes);
    stream[52..54].copy_from_slice(&(width as u16).to_le_bytes());
    stream[54..56].copy_from_slice(&(height as u16).to_le_bytes());
    let mut bitmap = vec![0; 40];
    word(&mut bitmap, 0, 40);
    word(&mut bitmap, 4, width);
    word(&mut bitmap, 8, height);
    bitmap[12..14].copy_from_slice(&1u16.to_le_bytes());
    bitmap[14..16].copy_from_slice(&24u16.to_le_bytes());
    word(&mut bitmap, 20, frame_bytes);
    let streams = list(b"strl", &[chunk(b"strh", &stream), chunk(b"strf", &bitmap)]);
    let headers = list(b"hdrl", &[chunk(b"avih", &header), streams]);
    let frames: Vec<_> = (0..frames)
        .map(|index| {
            let mut pixels = Vec::with_capacity(frame_bytes as usize);
            for _ in 0..height {
                for _ in 0..width {
                    pixels.extend_from_slice(&color(index));
                }
                pixels.resize(pixels.len().next_multiple_of(4), 0);
            }
            chunk(b"00db", &pixels)
        })
        .collect();
    let mut avi = b"AVI ".to_vec();
    avi.extend_from_slice(&headers);
    avi.extend_from_slice(&list(b"movi", &frames));
    chunk(b"RIFF", &avi)
}
