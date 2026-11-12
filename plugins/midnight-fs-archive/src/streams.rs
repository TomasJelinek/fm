//! Streaming decompression into an unpublished stage with bounded decoder windows.
use crate::*;
use std::io::{BufRead, Seek, SeekFrom};
const DECODER_MEMORY: u64 = 64 * 1024 * 1024;
pub(crate) fn decode(format: Format, source: Reader, stage: &mut Stage) -> Result<(), Error> {
    match format {
        Format::Gzip | Format::TarGzip => {
            stage.copy(flate2::read::MultiGzDecoder::new(source))?;
        }
        Format::Xz | Format::TarXz => {
            stage.copy(lzma_rust2::XzReader::new_mem_limit(
                source,
                true,
                (DECODER_MEMORY / 1024) as u32,
            ))?;
        }
        Format::Bzip2 | Format::TarBzip2 => {
            stage.copy(bzip2::read::MultiBzDecoder::new(source))?;
        }
        Format::Zstd | Format::TarZstd => zstd(source, stage)?,
        _ => {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Unsupported stream codec",
            ));
        }
    }
    Ok(())
}
fn zstd(mut source: Reader, stage: &mut Stage) -> Result<(), Error> {
    let mut frames = 0u64;
    while !source.fill_buf()?.is_empty() {
        let start = source.stream_position()?;
        let mut magic = [0; 4];
        source.read_exact(&mut magic)?;
        if (0x184d2a50..=0x184d2a5f).contains(&u32::from_le_bytes(magic)) {
            let mut size = [0; 4];
            source.read_exact(&mut size)?;
            let count = u64::from(u32::from_le_bytes(size));
            let copied = io::copy(&mut (&mut source).take(count), &mut io::sink())?;
            if copied != count {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            continue;
        }
        source.seek(SeekFrom::Start(start))?;
        let mut frame = ruzstd::decoding::FrameDecoder::new();
        frame.set_max_window_size(DECODER_MEMORY);
        let mut decoder = ruzstd::decoding::StreamingDecoder::new_with_decoder(source, frame)
            .map_err(|error| Error::new(ErrorCode::Corrupt, error.to_string()))?;
        stage.copy(&mut decoder)?;
        if let Some(expected) = decoder.decoder.get_checksum_from_data()
            && decoder.decoder.get_calculated_checksum() != Some(expected)
        {
            return Err(Error::new(
                ErrorCode::Corrupt,
                "Zstandard checksum mismatch",
            ));
        }
        source = decoder.into_inner();
        frames = frames.checked_add(1).ok_or_else(Error::limit)?;
    }
    if frames == 0 {
        return Err(Error::new(ErrorCode::Corrupt, "No Zstandard content frame"));
    }
    Ok(())
}
