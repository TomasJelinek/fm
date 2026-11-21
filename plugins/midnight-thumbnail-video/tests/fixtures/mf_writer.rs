//! H.264 + AAC MP4 fixtures written at test time by Media Foundation's own encoders (the Sink
//! Writer), so no encoded binary is committed. Each frame's left half shows `left` and its right
//! half `right`, with the colors switching halfway through; the audio is a 440 Hz tone.
#![allow(dead_code)]

use std::{
    io,
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
};
use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFAttributes, IMFByteStream, IMFMediaType, IMFSinkWriter, MF_MT_AAC_PAYLOAD_TYPE,
            MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
            MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
            MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
            MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
            MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_DISABLE_THROTTLING,
            MF_TRANSCODE_CONTAINERTYPE, MF_VERSION, MFAudioFormat_AAC, MFAudioFormat_PCM,
            MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
            MFCreateSinkWriterFromURL, MFMediaType_Audio, MFMediaType_Video, MFSTARTUP_LITE,
            MFShutdown, MFStartup, MFTranscodeContainerType_MPEG4, MFVideoFormat_H264,
            MFVideoFormat_NV12, MFVideoInterlace_Progressive,
        },
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
    },
    core::{HSTRING, Result},
};

pub const FIRST: [u8; 3] = [200, 40, 200];
pub const LATER: [u8; 3] = [40, 200, 40];

pub struct Movie {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Frames per second.
    pub rate: u32,
    pub audio: bool,
    /// The video codec, H.264 unless a test asks for another.
    pub video: windows::core::GUID,
    /// Let the Sink Writer use a hardware encoder (for a codec Windows encodes only that way).
    pub hardware: bool,
    /// The audio sample rate.
    pub audio_rate: u32,
}

impl Movie {
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            frames: 20,
            rate: 10,
            audio: true,
            video: MFVideoFormat_H264,
            hardware: false,
            audio_rate: 48_000,
        }
    }

    /// The left and right halves' colors in frame `index`.
    pub fn colors(&self, index: u32) -> ([u8; 3], [u8; 3]) {
        if index < self.frames / 2 {
            (FIRST, LATER)
        } else {
            (LATER, FIRST)
        }
    }

    /// Encode the movie and return the MP4 file's bytes.
    pub fn write(&self) -> io::Result<Vec<u8>> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path: PathBuf = std::env::temp_dir().join(format!(
            "midnight-mf-fixture-{}-{}.mp4",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = with_media_foundation(|| self.encode(&HSTRING::from(path.as_path())))
            .map_err(|error| io::Error::other(format!("Sink Writer failed: {error}")))
            .and_then(|()| std::fs::read(&path));
        let _ = std::fs::remove_file(&path);
        result
    }

    fn encode(&self, path: &HSTRING) -> Result<()> {
        // SAFETY: Media Foundation objects created and used on this thread only.
        unsafe {
            let attributes = attributes(&[
                (
                    &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
                    u32::from(self.hardware),
                ),
                (&MF_SINK_WRITER_DISABLE_THROTTLING, 1),
            ])?;
            attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
            let writer = MFCreateSinkWriterFromURL(path, None::<&IMFByteStream>, &attributes)?;
            let video = writer.AddStream(&self.video_type(&self.video)?)?;
            writer.SetInputMediaType(
                video,
                &self.video_type(&MFVideoFormat_NV12)?,
                None::<&IMFAttributes>,
            )?;
            let audio = if self.audio {
                let stream = writer.AddStream(&audio_type(&MFAudioFormat_AAC, self.audio_rate)?)?;
                writer.SetInputMediaType(
                    stream,
                    &audio_type(&MFAudioFormat_PCM, self.audio_rate)?,
                    None::<&IMFAttributes>,
                )?;
                Some(stream)
            } else {
                None
            };
            writer.BeginWriting()?;
            let step = 10_000_000 / i64::from(self.rate);
            for index in 0..self.frames {
                let time = i64::from(index) * step;
                self.write_frame(&writer, video, index, time, step)?;
                if let Some(audio) = audio {
                    write_tone(&writer, audio, time, step, self.audio_rate)?;
                }
            }
            writer.Finalize()
        }
    }

    fn video_type(&self, subtype: &windows::core::GUID) -> Result<IMFMediaType> {
        // SAFETY: a new media type owned here.
        unsafe {
            let media = MFCreateMediaType()?;
            media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            media.SetGUID(&MF_MT_SUBTYPE, subtype)?;
            media.SetUINT64(
                &MF_MT_FRAME_SIZE,
                (u64::from(self.width) << 32) | u64::from(self.height),
            )?;
            media.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(self.rate) << 32) | 1)?;
            media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
            media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            if *subtype != MFVideoFormat_NV12 {
                media.SetUINT32(&MF_MT_AVG_BITRATE, 2_000_000)?;
            }
            Ok(media)
        }
    }

    fn write_frame(
        &self,
        writer: &IMFSinkWriter,
        stream: u32,
        index: u32,
        time: i64,
        step: i64,
    ) -> Result<()> {
        let (left, right) = self.colors(index);
        let bytes = nv12(self.width, self.height, left, right);
        write(writer, stream, &bytes, time, step)
    }
}

fn audio_type(subtype: &windows::core::GUID, rate: u32) -> Result<IMFMediaType> {
    // SAFETY: a new media type owned here.
    unsafe {
        let media = MFCreateMediaType()?;
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        media.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        media.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        media.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        media.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        if *subtype == MFAudioFormat_AAC {
            media.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16_000)?;
            media.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
        } else {
            media.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
            media.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * 4)?;
        }
        Ok(media)
    }
}

fn write_tone(writer: &IMFSinkWriter, stream: u32, time: i64, step: i64, rate: u32) -> Result<()> {
    let rate = i64::from(rate);
    let first = time * rate / 10_000_000;
    let frames = step * rate / 10_000_000;
    let mut bytes = Vec::with_capacity(frames as usize * 4);
    for frame in first..first + frames {
        let value = (frame as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin();
        let sample = (value * 8_000.0) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    write(writer, stream, &bytes, time, step)
}

fn write(
    writer: &IMFSinkWriter,
    stream: u32,
    bytes: &[u8],
    time: i64,
    duration: i64,
) -> Result<()> {
    // SAFETY: a new buffer of `bytes.len()` bytes is locked only while it is filled.
    unsafe {
        let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
        let mut data = std::ptr::null_mut();
        buffer.Lock(&mut data, None, None)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(bytes.len() as u32)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(time)?;
        sample.SetSampleDuration(duration)?;
        writer.WriteSample(stream, &sample)
    }
}

/// Limited-range BT.601 NV12 with `left` and `right` color halves.
pub fn nv12(width: u32, height: u32, left: [u8; 3], right: [u8; 3]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let ycbcr = |[r, g, b]: [u8; 3]| {
        let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
        let y = 16.0 + (65.481 * r + 128.553 * g + 24.966 * b) / 255.0;
        let cb = 128.0 + (-37.797 * r - 74.203 * g + 112.0 * b) / 255.0;
        let cr = 128.0 + (112.0 * r - 93.786 * g - 18.214 * b) / 255.0;
        [y, cb, cr].map(|value| value.round().clamp(0.0, 255.0) as u8)
    };
    let (left, right) = (ycbcr(left), ycbcr(right));
    let mut bytes = vec![0; w * h + w * h.div_ceil(2)];
    for y in 0..h {
        for x in 0..w {
            bytes[y * w + x] = if x < w / 2 { left[0] } else { right[0] };
        }
    }
    for y in 0..h.div_ceil(2) {
        for pair in 0..w / 2 {
            let color = if pair * 2 < w / 2 { left } else { right };
            bytes[w * h + y * w + pair * 2] = color[1];
            bytes[w * h + y * w + pair * 2 + 1] = color[2];
        }
    }
    bytes
}

fn attributes(values: &[(&windows::core::GUID, u32)]) -> Result<IMFAttributes> {
    let mut attributes = None;
    // SAFETY: valid output pointer; the store is used on this thread.
    unsafe {
        MFCreateAttributes(&mut attributes, values.len() as u32 + 1)?;
        let attributes = attributes.ok_or_else(windows::core::Error::empty)?;
        for (key, value) in values {
            attributes.SetUINT32(*key, *value)?;
        }
        Ok(attributes)
    }
}

/// Run `work` with COM and Media Foundation started on this thread.
pub fn with_media_foundation<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
    // SAFETY: paired initialization and shutdown on this thread.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let result = MFStartup(MF_VERSION, MFSTARTUP_LITE).and_then(|()| {
            let result = work();
            let _ = MFShutdown();
            result
        });
        CoUninitialize();
        result
    }
}

/// Rewrite the matrix of every track whose header has a picture size, so the movie turns
/// `quarter_turns` times 90 degrees clockwise for display (as phones record portrait video).
pub fn rotate(bytes: &mut [u8], quarter_turns: u32) -> bool {
    let (a, b, c, d): (i32, i32, i32, i32) = match quarter_turns % 4 {
        1 => (0, 0x1_0000, -0x1_0000, 0),
        2 => (-0x1_0000, 0, 0, -0x1_0000),
        3 => (0, -0x1_0000, 0x1_0000, 0),
        _ => (0x1_0000, 0, 0, 0x1_0000),
    };
    let mut patched = false;
    let mut start = 0;
    while let Some(found) = find(&bytes[start..], b"tkhd") {
        let header = start + found + 4;
        let version = bytes[header];
        // version, flags, times, track id, reserved, duration, reserved, layer, group, volume,
        // reserved, then the matrix and the picture size.
        let matrix = header + 4 + if version == 1 { 32 } else { 20 } + 8 + 8;
        let size = matrix + 36;
        if size + 8 <= bytes.len() && bytes[size..size + 8] != [0; 8] {
            for (index, value) in [a, b, 0, c, d, 0, 0, 0, 0x4000_0000]
                .into_iter()
                .enumerate()
            {
                bytes[matrix + index * 4..matrix + index * 4 + 4]
                    .copy_from_slice(&value.to_be_bytes());
            }
            patched = true;
        }
        start = header;
    }
    patched
}

/// Replace the video sample entry's four-character code, keeping everything else.
pub fn replace_sample_entry(bytes: &mut [u8], from: &[u8; 4], to: &[u8; 4]) -> bool {
    let Some(stsd) = find(bytes, b"stsd") else {
        return false;
    };
    let Some(entry) = find(&bytes[stsd..], from) else {
        return false;
    };
    bytes[stsd + entry..stsd + entry + 4].copy_from_slice(to);
    true
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
