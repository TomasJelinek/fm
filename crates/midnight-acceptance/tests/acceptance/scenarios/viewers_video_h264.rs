//! An H.264 + AAC MP4 written here by Windows' own encoders (Media Foundation's Sink Writer),
//! so no encoded file is committed. The bundled FFmpeg profile has no H.264 decoder; the
//! viewer and the thumbnails reach Windows' decoders through the Media Foundation helpers.
#![allow(
    unsafe_code,
    reason = "Media Foundation's Sink Writer is a COM API; every call is on this thread"
)]
use std::path::Path;
use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFAttributes, IMFByteStream, IMFMediaType, IMFSinkWriter, MF_MT_AAC_PAYLOAD_TYPE,
            MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
            MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
            MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
            MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
            MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_TRANSCODE_CONTAINERTYPE, MF_VERSION,
            MFAudioFormat_AAC, MFAudioFormat_PCM, MFCreateAttributes, MFCreateMediaType,
            MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Audio,
            MFMediaType_Video, MFSTARTUP_LITE, MFShutdown, MFStartup,
            MFTranscodeContainerType_MPEG4, MFVideoFormat_H264, MFVideoFormat_NV12,
            MFVideoInterlace_Progressive,
        },
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
    },
    core::{GUID, HSTRING, Result},
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FRAMES: u32 = 20;
const RATE: u32 = 10;
/// One frame (and its audio) in 100 ns units.
const STEP: i64 = 10_000_000 / RATE as i64;

/// Write a two-second movie to `path`: ten frames of `first`, then ten of `later`, with a
/// silent AAC track, so the scenario plays audio through the output without a sound.
pub(super) fn write(
    path: &Path,
    first: [u8; 3],
    later: [u8; 3],
) -> std::result::Result<(), String> {
    // SAFETY: COM and Media Foundation start and stop on this thread around the work.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|error| format!("COM: {error}"))?;
        let result = MFStartup(MF_VERSION, MFSTARTUP_LITE).and_then(|()| {
            let result = encode(&HSTRING::from(path), first, later);
            let _ = MFShutdown();
            result
        });
        CoUninitialize();
        result.map_err(|error| format!("Windows' H.264 or AAC encoder failed: {error}"))
    }
}

unsafe fn encode(path: &HSTRING, first: [u8; 3], later: [u8; 3]) -> Result<()> {
    // SAFETY: the caller started Media Foundation on this thread; objects stay on it.
    unsafe {
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 2)?;
        let attributes = attributes.ok_or_else(windows::core::Error::empty)?;
        attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 0)?;
        attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        let writer = MFCreateSinkWriterFromURL(path, None::<&IMFByteStream>, &attributes)?;
        let video = writer.AddStream(&video_type(&MFVideoFormat_H264)?)?;
        writer.SetInputMediaType(
            video,
            &video_type(&MFVideoFormat_NV12)?,
            None::<&IMFAttributes>,
        )?;
        let audio = writer.AddStream(&audio_type(&MFAudioFormat_AAC)?)?;
        writer.SetInputMediaType(
            audio,
            &audio_type(&MFAudioFormat_PCM)?,
            None::<&IMFAttributes>,
        )?;
        writer.BeginWriting()?;
        for index in 0..FRAMES {
            let time = i64::from(index) * STEP;
            let colour = if index < FRAMES / 2 { first } else { later };
            write_sample(&writer, video, &nv12(colour), time)?;
            write_sample(&writer, audio, &silence(), time)?;
        }
        writer.Finalize()
    }
}

unsafe fn video_type(subtype: &GUID) -> Result<IMFMediaType> {
    // SAFETY: a new media type owned here.
    unsafe {
        let media = MFCreateMediaType()?;
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        media.SetUINT64(
            &MF_MT_FRAME_SIZE,
            (u64::from(WIDTH) << 32) | u64::from(HEIGHT),
        )?;
        media.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(RATE) << 32) | 1)?;
        media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
        media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        if *subtype == MFVideoFormat_H264 {
            media.SetUINT32(&MF_MT_AVG_BITRATE, 1_000_000)?;
        }
        Ok(media)
    }
}

unsafe fn audio_type(subtype: &GUID) -> Result<IMFMediaType> {
    // SAFETY: a new media type owned here.
    unsafe {
        let media = MFCreateMediaType()?;
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        media.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        media.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        media.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000)?;
        media.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        if *subtype == MFAudioFormat_AAC {
            media.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16_000)?;
            media.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
        } else {
            media.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
            media.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 192_000)?;
        }
        Ok(media)
    }
}

unsafe fn write_sample(writer: &IMFSinkWriter, stream: u32, bytes: &[u8], time: i64) -> Result<()> {
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
        sample.SetSampleDuration(STEP)?;
        writer.WriteSample(stream, &sample)
    }
}

/// One limited-range BT.601 NV12 frame of `colour`.
fn nv12([r, g, b]: [u8; 3]) -> Vec<u8> {
    let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
    let y = 16.0 + (65.481 * r + 128.553 * g + 24.966 * b) / 255.0;
    let cb = 128.0 + (-37.797 * r - 74.203 * g + 112.0 * b) / 255.0;
    let cr = 128.0 + (112.0 * r - 93.786 * g - 18.214 * b) / 255.0;
    let [y, cb, cr] = [y, cb, cr].map(|value| value.round().clamp(0.0, 255.0) as u8);
    let luma = (WIDTH * HEIGHT) as usize;
    let mut bytes = vec![y; luma];
    bytes.extend([cb, cr].repeat(luma / 4));
    bytes
}

/// One frame's worth of silent 48 kHz stereo PCM.
fn silence() -> Vec<u8> {
    vec![0; (STEP * 48_000 / 10_000_000) as usize * 4]
}
