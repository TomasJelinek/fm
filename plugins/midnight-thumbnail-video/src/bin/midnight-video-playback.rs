//! Locked-down video playback worker with the bundled FFmpeg profile. Native source access
//! stays in its host.
#![forbid(unsafe_code)]

use std::io;

fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    midnight_worker_sdk::enter(&mut input, &mut output)?;
    midnight_thumbnail_video::playback_worker::serve(
        input,
        output,
        midnight_thumbnail_video::playback::open,
    )
}
