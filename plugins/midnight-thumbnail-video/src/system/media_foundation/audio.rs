//! The audio stream: a second source reader over its own byte stream, so PCM demand stays
//! independent of the video. Media Foundation decodes the first audio stream and converts it to
//! 48 kHz stereo 16-bit PCM; anything it cannot convert plays as an audio notice.
use super::{failure, remote::Reads, source_reader, video::seek};
use crate::{
    media::{AudioTrack, Block},
    system::pcm::{CHANNELS, Clip, RATE, Timeline},
};
use std::io;
use windows::Win32::Media::MediaFoundation::{
    IMFSourceReader, MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
    MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR,
    MFAudioFormat_PCM, MFCreateMediaType, MFMediaType_Audio,
};

const ALL: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
const AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
/// Decode this much before a seek target, then drop it, so the first block after a seek starts
/// from settled decoder state.
const PREROLL_100NS: i64 = 800_000;
const BITS: u32 = 16;

pub(super) struct AudioReader {
    reader: Option<IMFSourceReader>,
    timeline: Timeline,
    ended: bool,
}

impl AudioReader {
    /// `None` when the media has no audio stream. A stream Media Foundation cannot convert to
    /// PCM is kept as an unqualified track, so the viewer can show its notice.
    pub(super) fn open(reads: &Reads, duration_micros: Option<u64>) -> io::Result<Option<Self>> {
        let reader = source_reader(reads)?;
        // SAFETY: a new media type owned here and the live reader on its owning thread.
        unsafe {
            reader.SetStreamSelection(ALL, false).map_err(failure)?;
            if reader.SetStreamSelection(AUDIO, true).is_err() {
                return Ok(None);
            }
            let wanted = MFCreateMediaType().map_err(failure)?;
            wanted
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .map_err(failure)?;
            wanted
                .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)
                .map_err(failure)?;
            wanted
                .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, BITS)
                .map_err(failure)?;
            wanted
                .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)
                .map_err(failure)?;
            wanted
                .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32)
                .map_err(failure)?;
            let frame_bytes = CHANNELS as u32 * BITS / 8;
            wanted
                .SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, frame_bytes)
                .map_err(failure)?;
            wanted
                .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, frame_bytes * RATE)
                .map_err(failure)?;
            let converted = reader.SetCurrentMediaType(AUDIO, None, &wanted).is_ok()
                && reader.GetCurrentMediaType(AUDIO).is_ok_and(|current| {
                    current
                        .GetGUID(&MF_MT_SUBTYPE)
                        .is_ok_and(|subtype| subtype == MFAudioFormat_PCM)
                        && current
                            .GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE)
                            .is_ok_and(|bits| bits == BITS)
                        && current
                            .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
                            .is_ok_and(|rate| rate == RATE)
                        && current
                            .GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
                            .is_ok_and(|channels| channels == CHANNELS as u32)
                });
            Ok(Some(Self {
                reader: converted.then_some(reader),
                timeline: Timeline::new(duration_micros),
                ended: false,
            }))
        }
    }

    pub(super) fn track(&self) -> AudioTrack {
        if self.reader.is_some() {
            AudioTrack::Qualified
        } else {
            AudioTrack::Unqualified
        }
    }

    pub(super) fn seek(&mut self, position_micros: u64) -> io::Result<()> {
        let reader = self.reader.as_ref().ok_or_else(unqualified)?;
        let target = i64::try_from(position_micros)
            .ok()
            .and_then(|micros| micros.checked_mul(10))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Audio seek exceeds timestamp range",
                )
            })?;
        seek(reader, target.saturating_sub(PREROLL_100NS))?;
        self.timeline.seek(position_micros);
        self.ended = false;
        Ok(())
    }

    pub(super) fn pcm(&mut self, credit: u16) -> io::Result<Option<Block>> {
        let Self {
            reader,
            timeline,
            ended,
        } = self;
        let reader = reader.as_ref().ok_or_else(unqualified)?;
        timeline.next(credit, || next_clip(reader, ended))
    }
}

/// The next decoded PCM clip, or `None` at the end of the audio stream.
fn next_clip(reader: &IMFSourceReader, ended: &mut bool) -> io::Result<Option<Clip>> {
    if *ended {
        return Ok(None);
    }
    for _ in 0..4096 {
        let (mut flags, mut time, mut sample) = (0u32, 0i64, None);
        // SAFETY: valid output pointers for one synchronous read on the owning thread.
        unsafe {
            reader.ReadSample(
                AUDIO,
                0,
                None,
                Some(&raw mut flags),
                Some(&raw mut time),
                Some(&raw mut sample),
            )
        }
        .map_err(failure)?;
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            return Err(io::Error::other(
                "Media Foundation reported an audio stream error",
            ));
        }
        if let Some(sample) = sample {
            let samples = pcm_samples(&sample)?;
            if !samples.is_empty() {
                return Ok(Some(Clip {
                    start_nanos: time.saturating_mul(100),
                    samples,
                }));
            }
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            *ended = true;
            return Ok(None);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "audio packet traversal exceeded its decode-step budget",
    ))
}

fn pcm_samples(sample: &windows::Win32::Media::MediaFoundation::IMFSample) -> io::Result<Vec<i16>> {
    // SAFETY: the buffer is locked only while its bytes are copied out.
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().map_err(failure)?;
        let mut data = std::ptr::null_mut();
        let mut length = 0u32;
        buffer
            .Lock(&mut data, None, Some(&raw mut length))
            .map_err(failure)?;
        let samples = if data.is_null() || !(length as usize).is_multiple_of(CHANNELS * 2) {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decoded audio is not whole stereo frames",
            ))
        } else {
            Ok(std::slice::from_raw_parts(data, length as usize)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| i16::from_le_bytes(*pair))
                .collect())
        };
        let _ = buffer.Unlock();
        samples
    }
}

fn unqualified() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "This media has no qualified PCM track",
    )
}
