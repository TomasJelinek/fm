//! Bounded standalone compression decoders. No dictionary or path access.
use crate::*;

pub(crate) fn decode(
    format: Format,
    source: &[u8],
    budget: u64,
    memory: u64,
) -> Result<Vec<u8>, Error> {
    match format {
        Format::Xz | Format::TarXz => bounded_read(
            lzma_rust2::XzReader::new_mem_limit(
                source,
                true,
                (memory / 1024).min(u32::MAX as u64) as u32,
            ),
            budget,
        ),
        Format::Bzip2 | Format::TarBzip2 => {
            bounded_read(bzip2::read::MultiBzDecoder::new(source), budget)
        }
        Format::Zstd | Format::TarZstd => zstd(source, budget, memory),
        _ => Err(Error::new(
            ErrorCode::Unsupported,
            "Unsupported stream codec",
        )),
    }
}

fn zstd(mut source: &[u8], budget: u64, memory: u64) -> Result<Vec<u8>, Error> {
    let mut result = Vec::new();
    let mut frames = 0usize;
    while !source.is_empty() {
        // Skippable frames carry only bounded metadata and produce no output.
        if source.len() >= 4
            && (0x184d2a50..=0x184d2a5f).contains(&u32::from_le_bytes(
                source[..4].try_into().map_err(|_| Error::limit())?,
            ))
        {
            if source.len() < 8 {
                return Err(Error::new(
                    ErrorCode::Corrupt,
                    "Truncated Zstandard skippable frame",
                ));
            }
            let size =
                u32::from_le_bytes(source[4..8].try_into().map_err(|_| Error::limit())?) as usize;
            source = source
                .get(8..)
                .and_then(|bytes| bytes.get(size..))
                .ok_or_else(|| {
                    Error::new(ErrorCode::Corrupt, "Truncated Zstandard skippable frame")
                })?;
            continue;
        }
        let mut frame = ruzstd::decoding::FrameDecoder::new();
        frame.set_max_window_size(memory);
        let mut decoder = ruzstd::decoding::StreamingDecoder::new_with_decoder(source, frame)
            .map_err(|error| Error::new(ErrorCode::Corrupt, error.to_string()))?;
        let bytes = bounded_read(&mut decoder, budget.saturating_sub(result.len() as u64))?;
        if let Some(expected) = decoder.decoder.get_checksum_from_data()
            && decoder.decoder.get_calculated_checksum() != Some(expected)
        {
            return Err(Error::new(
                ErrorCode::Corrupt,
                "Zstandard checksum mismatch",
            ));
        }
        result.extend_from_slice(&bytes);
        source = decoder.into_inner();
        frames += 1;
    }
    if frames == 0 {
        return Err(Error::new(ErrorCode::Corrupt, "No Zstandard content frame"));
    }
    Ok(result)
}
