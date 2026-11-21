//! Locked-down video playback worker with the platform's decoders (Media Foundation on
//! Windows; the distribution's FFmpeg, then GStreamer, on Linux). It links no FFmpeg.
#![forbid(unsafe_code)]

fn main() -> std::io::Result<()> {
    midnight_thumbnail_video::system::playback_main()
}
