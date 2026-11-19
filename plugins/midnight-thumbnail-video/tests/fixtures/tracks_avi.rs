//! Owned two-track AVI. Raw video carries exact solid-color pixels. An optional MJPG track is
//! deliberately outside the installed profile and carries only JPEG boundary markers; it is
//! never a decoder fixture for that codec. Its metadata/packets exercise real demux selection.

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

fn stream(frames: u32, compressed: bool, frame_rate: u32) -> Vec<u8> {
    let handler = if compressed { b"MJPG" } else { b"DIB " };
    let mut stream = vec![0; 56];
    stream[..4].copy_from_slice(b"vids");
    stream[4..8].copy_from_slice(handler);
    word(&mut stream, 20, 1);
    word(&mut stream, 24, frame_rate);
    word(&mut stream, 32, frames);
    word(&mut stream, 36, 16);
    stream[52..54].copy_from_slice(&2u16.to_le_bytes());
    stream[54..56].copy_from_slice(&2u16.to_le_bytes());
    let mut bitmap = vec![0; 40];
    word(&mut bitmap, 0, 40);
    word(&mut bitmap, 4, 2);
    word(&mut bitmap, 8, 2);
    bitmap[12..14].copy_from_slice(&1u16.to_le_bytes());
    bitmap[14..16].copy_from_slice(&24u16.to_le_bytes());
    if compressed {
        bitmap[16..20].copy_from_slice(b"MJPG");
    }
    word(&mut bitmap, 20, 16);
    list(b"strl", &[chunk(b"strh", &stream), chunk(b"strf", &bitmap)])
}

/// `preferred` receives a thirty-packet prefix; the other track has six packets. This makes
/// the preferred track exercise FFmpeg's ranking. For a non-first preference the first
/// track also runs at 1 fps, giving the preferred track a higher indexed bitrate.
/// Tests independently assert FFmpeg's actual preference before asserting our selection.
pub fn two_tracks(preferred: u8, unsupported_preferred: bool) -> Vec<u8> {
    indexed_tracks(preferred, unsupported_preferred, 30)
}

/// More unsupported packets than one cursor call may skip. A seek for the selected alternate
/// must use its index instead of walking forward through the unrelated default video.
pub fn long_unsupported_prefix() -> Vec<u8> {
    indexed_tracks(0, true, 5000)
}

fn indexed_tracks(preferred: u8, unsupported_preferred: bool, preferred_frames: u32) -> Vec<u8> {
    assert!(preferred < 2);
    let mut header = vec![0; 56];
    word(&mut header, 0, 166_667);
    word(&mut header, 12, 0x10); // AVIF_HASINDEX
    word(&mut header, 16, preferred_frames);
    word(&mut header, 24, 2);
    word(&mut header, 32, 2);
    word(&mut header, 36, 2);
    let streams: Vec<_> = (0..2)
        .map(|track| {
            stream(
                if track == preferred {
                    preferred_frames
                } else {
                    6
                },
                unsupported_preferred && track == preferred,
                if preferred == 1 && track == 0 { 1 } else { 6 },
            )
        })
        .collect();
    let headers = list(
        b"hdrl",
        &[
            chunk(b"avih", &header),
            streams[0].clone(),
            streams[1].clone(),
        ],
    );
    let mut frames = Vec::new();
    let mut index = Vec::new();
    let mut offset = 4u32; // Offsets are relative to the LIST's "movi" FourCC.
    for track in [preferred, 1 - preferred] {
        let compressed = unsupported_preferred && track == preferred;
        for _ in 0..if track == preferred {
            preferred_frames
        } else {
            6
        } {
            let pixels = if compressed {
                vec![0xff, 0xd8, 0xff, 0xd9]
            } else {
                let [blue, green, red] = if track == 0 {
                    [10, 20, 30]
                } else {
                    [50, 60, 70]
                };
                vec![
                    blue, green, red, blue, green, red, 0, 0, blue, green, red, blue, green, red,
                    0, 0,
                ]
            };
            let tag = [
                b'0',
                b'0' + track,
                b'd',
                if compressed { b'c' } else { b'b' },
            ];
            let frame = chunk(&tag, &pixels);
            index.extend_from_slice(&tag);
            index.extend_from_slice(&0x10u32.to_le_bytes()); // AVIIF_KEYFRAME
            index.extend_from_slice(&offset.to_le_bytes());
            index.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
            offset += frame.len() as u32;
            frames.push(frame);
        }
    }
    let mut avi = b"AVI ".to_vec();
    avi.extend_from_slice(&headers);
    avi.extend_from_slice(&list(b"movi", &frames));
    avi.extend_from_slice(&chunk(b"idx1", &index));
    chunk(b"RIFF", &avi)
}
