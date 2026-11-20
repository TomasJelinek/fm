//! Streaming decompression into an unpublished stage with bounded decoder windows.
use crate::*;
use std::io::{BufRead, Seek, SeekFrom};
const DECODER_MEMORY: u64 = 64 * 1024 * 1024;
pub(crate) fn decode(format: Format, source: Reader, stage: &mut Stage) -> Result<(), Error> {
    stage.copy(reader(format, source)?)?;
    Ok(())
}
/// The decompressed bytes of `source`: every gzip and bzip2 member, the xz stream, or
/// every Zstandard content frame (with skippable frames passed over).
pub(crate) fn reader(format: Format, source: Reader) -> Result<Box<dyn Read>, Error> {
    Ok(match format {
        Format::Gzip | Format::TarGzip => Box::new(flate2::read::MultiGzDecoder::new(source)),
        Format::Xz | Format::TarXz => Box::new(lzma_rust2::XzReader::new_mem_limit(
            source,
            true,
            (DECODER_MEMORY / 1024) as u32,
        )),
        Format::Bzip2 | Format::TarBzip2 => Box::new(bzip2::read::MultiBzDecoder::new(source)),
        Format::Zstd | Format::TarZstd => Box::new(Zstd {
            source: Some(source),
            frame: None,
            frames: 0,
        }),
        _ => {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Unsupported stream codec",
            ));
        }
    })
}
type ZstdFrame = ruzstd::decoding::StreamingDecoder<Reader, ruzstd::decoding::FrameDecoder>;
/// Zstandard frames in order. A content frame's checksum, when present, is verified
/// at its end; a stream without any content frame is corrupt.
struct Zstd {
    source: Option<Reader>,
    frame: Option<ZstdFrame>,
    frames: u64,
}
impl Read for Zstd {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        loop {
            if let Some(decoder) = &mut self.frame {
                let count = decoder.read(output)?;
                if count > 0 || output.is_empty() {
                    return Ok(count);
                }
                let decoder = self
                    .frame
                    .take()
                    .ok_or_else(|| corrupt("Zstandard frame"))?;
                if let Some(expected) = decoder.decoder.get_checksum_from_data()
                    && decoder.decoder.get_calculated_checksum() != Some(expected)
                {
                    return Err(corrupt("Zstandard checksum mismatch"));
                }
                self.source = Some(decoder.into_inner());
                self.frames = self.frames.saturating_add(1);
                continue;
            }
            let Some(source) = self.source.as_mut() else {
                return Ok(0);
            };
            if source.fill_buf()?.is_empty() {
                if self.frames == 0 {
                    return Err(corrupt("No Zstandard content frame"));
                }
                return Ok(0);
            }
            let start = source.stream_position()?;
            let mut magic = [0; 4];
            source.read_exact(&mut magic)?;
            if (0x184d2a50..=0x184d2a5f).contains(&u32::from_le_bytes(magic)) {
                let mut size = [0; 4];
                source.read_exact(&mut size)?;
                // Skippable content is never decoded. The read window rejects a seek
                // past the source end, so a truncated frame still fails.
                source.seek(SeekFrom::Current(i64::from(u32::from_le_bytes(size))))?;
                continue;
            }
            // Peeking the magic stays inside the read window: seeking back is free.
            source.seek(SeekFrom::Start(start))?;
            let mut frame = ruzstd::decoding::FrameDecoder::new();
            frame.set_max_window_size(DECODER_MEMORY);
            let source = self
                .source
                .take()
                .ok_or_else(|| corrupt("Zstandard source"))?;
            self.frame = Some(
                ruzstd::decoding::StreamingDecoder::new_with_decoder(source, frame)
                    .map_err(|error| corrupt(error.to_string()))?,
            );
        }
    }
}
fn corrupt(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
