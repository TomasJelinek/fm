//! Independent decoder. FFmpeg only sees the bounded public broker stream after worker lockdown.
mod native;
use ffmpeg_next as ffmpeg;
use midnight_thumbnail_sdk::{
    ReadSeek,
    api::{Frame, Request},
};
use std::io;

pub fn decode(request: &Request, source: &mut dyn ReadSeek) -> io::Result<Frame> {
    midnight_thumbnail_sdk::api::rgba_length(request.width, request.height)?;
    if request.frame >= midnight_thumbnail_sdk::api::MAX_ANIMATION_FRAMES
        || request.source_length > i64::MAX as u64
    {
        return Err(io::Error::other("Invalid video request"));
    }
    // The minimal native runtime intentionally has no internal threading support. Tests may call
    // this library concurrently; serialize them just as the one-request executable does naturally.
    static DECODE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _decode = DECODE
        .lock()
        .map_err(|_| io::Error::other("Previous native decode panicked"))?;
    ffmpeg::init().map_err(io::Error::other)?;
    native::with_input(source, request.source_length, |input| {
        thumbnail(input, request)
    })
}
fn thumbnail(input: &mut ffmpeg::format::context::Input, request: &Request) -> io::Result<Frame> {
    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| io::Error::other("No video stream"))?;
    let index = stream.index();
    let time_base = stream.time_base();
    let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .map_err(io::Error::other)?;
    if !matches!(
        context.id(),
        ffmpeg::codec::Id::VP8
            | ffmpeg::codec::Id::VP9
            | ffmpeg::codec::Id::THEORA
            | ffmpeg::codec::Id::FFV1
            | ffmpeg::codec::Id::RAWVIDEO
    ) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Video codec is outside the installed baseline",
        ));
    }
    let rotation = native::rotation(&stream)?;
    if !matches!(rotation, 0 | 90 | 180 | 270) {
        return Err(io::Error::other("Unsupported video orientation"));
    }
    let mut context = context;
    native::limit_decoder(&mut context);
    let mut decoder = context.decoder().video().map_err(io::Error::other)?;
    if decoder.width() == 0
        || decoder.height() == 0
        || u64::from(decoder.width()) * u64::from(decoder.height()) > 32 * 1024 * 1024
    {
        return Err(io::Error::other("Video dimensions exceed decoder bound"));
    }
    // Every request is independently seekable and cacheable. Sample between 10% and 85%.
    let duration = input.duration().max(0);
    let fraction = 10 + i64::from(request.frame) * 15;
    let target = duration.saturating_mul(fraction) / 100;
    if target > 0 {
        input.seek(target, ..target).map_err(io::Error::other)?;
        decoder.flush();
    }
    let target_timestamp = if time_base.numerator() > 0 && time_base.denominator() > 0 {
        i64::try_from(
            i128::from(target) * i128::from(time_base.denominator())
                / (1_000_000 * i128::from(time_base.numerator())),
        )
        .map_err(|_| io::Error::other("Video timestamp exceeds supported range"))?
    } else {
        0
    };
    let mut decoded = ffmpeg::frame::Video::empty();
    for _ in 0..4096 {
        let mut packet = ffmpeg::Packet::empty();
        match packet.read(input) {
            Ok(()) => {
                if packet.stream() != index {
                    continue;
                }
                decoder.send_packet(&packet).map_err(io::Error::other)?;
            }
            Err(ffmpeg::Error::Eof) => {
                decoder.send_eof().map_err(io::Error::other)?;
            }
            Err(error) => return Err(io::Error::other(error)),
        }
        while decoder.receive_frame(&mut decoded).is_ok() {
            if decoded
                .timestamp()
                .is_some_and(|timestamp| timestamp < target_timestamp)
            {
                continue;
            }
            return pixels(&decoded, request, rotation);
        }
    }
    Err(io::Error::other(
        "No representative frame within decode budget",
    ))
}
fn pixels(decoded: &ffmpeg::frame::Video, request: &Request, rotation: i32) -> io::Result<Frame> {
    if decoded.width() == 0
        || decoded.height() == 0
        || u64::from(decoded.width()) * u64::from(decoded.height()) > 32 * 1024 * 1024
    {
        return Err(io::Error::other("Decoded video dimensions exceed bound"));
    }
    let (bound_w, bound_h) = if rotation == 90 || rotation == 270 {
        (request.height, request.width)
    } else {
        (request.width, request.height)
    };
    let scale = (f64::from(bound_w) / f64::from(decoded.width()))
        .min(f64::from(bound_h) / f64::from(decoded.height()))
        .min(1.0);
    let width = (f64::from(decoded.width()) * scale).round().max(1.0) as u32;
    let height = (f64::from(decoded.height()) * scale).round().max(1.0) as u32;
    let mut scaling = ffmpeg::software::scaling::context::Context::get(
        decoded.format(),
        decoded.width(),
        decoded.height(),
        ffmpeg::format::Pixel::RGBA,
        width,
        height,
        ffmpeg::software::scaling::Flags::BILINEAR,
    )
    .map_err(io::Error::other)?;
    let transfer_709 = native::color_conversion(decoded, &mut scaling)?;
    let mut resized = ffmpeg::frame::Video::empty();
    scaling
        .run(decoded, &mut resized)
        .map_err(io::Error::other)?;
    let output_w = if rotation == 90 || rotation == 270 {
        height
    } else {
        width
    };
    let output_h = if rotation == 90 || rotation == 270 {
        width
    } else {
        height
    };
    let mut rgba = vec![0; midnight_thumbnail_sdk::api::rgba_length(output_w, output_h)?];
    let stride = resized.stride(0);
    let data = resized.data(0);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let (out_x, out_y) = match rotation {
                90 => (height as usize - y - 1, x),
                180 => (width as usize - x - 1, height as usize - y - 1),
                270 => (y, width as usize - x - 1),
                _ => (x, y),
            };
            let source = y * stride + x * 4;
            let target = (out_y * output_w as usize + out_x) * 4;
            rgba[target..target + 4].copy_from_slice(
                data.get(source..source + 4)
                    .ok_or_else(|| io::Error::other("Invalid scaled video stride"))?,
            );
        }
    }
    if transfer_709 {
        let table: [u8; 256] = std::array::from_fn(|value| {
            let encoded = value as f64 / 255.0;
            let linear = if encoded < 0.081 {
                encoded / 4.5
            } else {
                ((encoded + 0.099) / 1.099).powf(1.0 / 0.45)
            };
            let srgb = if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            (srgb * 255.0).round().clamp(0.0, 255.0) as u8
        });
        for pixel in rgba.as_chunks_mut::<4>().0 {
            for component in &mut pixel[..3] {
                *component = table[usize::from(*component)];
            }
        }
    }
    Ok(Frame {
        id: request.id,
        frame: request.frame,
        width: output_w,
        height: output_h,
        rgba,
    })
}
