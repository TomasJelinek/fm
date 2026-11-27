# Linux audio underruns during video playback, 26 November 2026

A 1080p H.264 Matroska movie with AC-3 5.1 sound did not play on a Linux desktop whose
ALSA default device is PipeWire's plugin: the canvas stayed black at 0:00 with Play
shown, and the status line read "ALSA underrun left unconfirmed downstream media · ALSA
underrun left unconfirmed downstream media". Playback now recovers from an underrun and
continues, and a failure shows once in the status line
([audio clock and underruns](../developer/viewer-presentation.md#audio-clock-and-underruns)).

## Cause

The Linux output (`crates/midnight-media-output/src/linux.rs`) counts a frame as played
only when the running device's delay shows it. After an underrun (XRUN) it waited up to
200 ms for the stopped device's status to prove that no sound was left after the ring,
and otherwise ended playback with the error above.

A probe opened ALSA's `default` device as the output does (48 kHz stereo S16,
nonblocking, 100 ms latency, start threshold one frame, stop threshold the whole ring),
wrote silence and sampled the status every 2 ms. On Ubuntu 26.04 with GNOME on Wayland
and PipeWire 1.6.2 as the default device, the plugin gave a 4800-frame ring and took one
1024-frame graph quantum per cycle:

| Moment | State | Delay | Available | In the ring |
| --- | --- | --- | --- | --- |
| 4800 frames written | RUNNING | 4800 | 0 | 4800 |
| after the first four cycles | RUNNING | ring + about 1000 to 2000, counting down | 1024 to 4096 | 3776 to 704 |
| fifth cycle | XRUN | 2048 | 4096 | 704 |
| 1.5 s later | XRUN | 2048 | 4096 | 704 |
| after `snd_pcm_prepare` | PREPARED | 2048 | 4800 | 0 |
| 4800 frames written again | RUNNING | 6848 | 0 | 4800 |

The plugin stops as soon as a cycle finds less than a quantum. The 704 frames left in
the ring never play, and until prepare the status repeats the last delay and
availability. The output's confirmation (the whole ring available and a delay of zero
after ring-only delays, or a negative delay) can never hold there. So every underrun
with sound left downstream ended playback after 200 ms, the drain at the end of every
movie with sound included. Small blocks with pauses (1920 frames, then 30, 60, 150 and
250 ms of nothing) stopped the same way, with 512 frames left in the ring.

The worker reports a failed video both as the media error and as the viewer's status
(`fail_video` in `crates/midnight-viewer-engine/src/worker_loop.rs`), and the status line
joined the two with " · ".

## Change

- `linux.rs`: an underrun takes the device's queue off the clock and prepares the device
  again at once. Its frames count as played only with the existing proof of an empty
  path (`DelayEvidence::exhausted`). Otherwise the clock keeps its last position, and the
  running delay after the restart passes the lost frames only when the next block
  plays. The 200 ms wait and its error are gone. The PCM calls go through a private
  `Pcm` trait: the libasound handle (`NativePcm`) implements it, and so does a model of
  the recorded plugin in the tests.
- `midnight-media-host`: the `DeviceClock` contract says that frames an underrun took
  off the device without proof are neither played nor queued.
- `crates/midnight-viewer-ui/ui/viewer.slint`: the status line shows a media error once
  when the viewer's status repeats it, and a playback status alone, without a trailing
  " · ", when the viewer has no other status.

## Tests

Builds ran in the project's Docker build image with private `CARGO_TARGET_DIR`s under
the worktree's `_build/`; the native runs started those binaries on the PipeWire host.

| Command | Result |
| --- | --- |
| `cargo test -p midnight-media-output -p midnight-media-host -p midnight-viewer-ui --lib` | 5 passed and 2 ignored (the native device tests below), 47 passed, 253 passed |
| `midnight_media_output-… default_device --ignored --test-threads=1` on the host, three runs | 2 passed in each: `default_device_pause_underrun_and_retirement` and `default_device_playback_continues_after_a_slow_start` |
| `cargo clippy -p midnight-media-output -p midnight-media-host -p midnight-viewer-ui --all-targets -- -D warnings`; `cargo fmt -p` the same packages `-- --check` | clean |

The model (`PipeWire` in `linux_tests.rs`) reports the recorded values at cycle
boundaries: 4800 after the first write, XRUN with delay 2048 and 4096 available after
five cycles, a stale 2048 after prepare and 6848 after the next write. Its
`heard` counter is the frame playing at each boundary, and the tests check that the
clock never passes it.

- `pipewire_underrun_keeps_the_clock_and_playback_continues`: the XRUN with 704 frames
  left keeps the clock at 2048 played frames with nothing queued, also 250 ms later;
  the next block is accepted, the clock passes the lost frames only once that block
  plays, and an underrun that a write finds first recovers the same way.
- `playback_continues_through_pipewire_underruns_and_drains_at_the_end`: `Playback`
  with a decoder that stalls after its first block and again mid-stream plays all
  twelve blocks, never ahead of the model's playing frame, and ends drained.
- `unconfirmed_underrun_retires_the_queue_without_counting_it` replaces the test of the
  former error.
- `default_device_pause_underrun_and_retirement` (native) now waits for a drained queue
  instead of every frame played, which PipeWire never reports, checks that the clock
  passes the first block once the second plays, and lets a write find an underrun.
- `default_device_playback_continues_after_a_slow_start` (native): `Playback` on the
  default device with one block, then 300 ms without sound, then nine blocks in time:
  no error, the clock never ahead of the time since the start, all ten blocks drained
  and the clock past 900 ms.
- `pressing_video_controls_keeps_the_video_area_and_controls_in_place` checks the exact
  status line: "Seeking…" alone, "Seeking… · " with a notice, a refused command's error
  alone, and a failed video's sentence once while the viewer's status repeats it.

Each new or changed test failed first against the former code. The two model tests
failed, one with "ALSA underrun left unconfirmed downstream media" when the XRUN was
sampled again after 250 ms, the other with 3 of 12 blocks delivered. Both native tests
failed on the host with the same error, the slow start after 0.33 s. The status-line
test failed on the trailing separator, and with those checks set aside, on the failed
video's sentence shown twice.

## End to end

The `viewers_video` acceptance scenario ran on the same host, from the suite's binary
built in Docker, with a private copy of the plugin packages of a debug build made the
same day. The scenario makes its H.264 movie with the host's `ffmpeg`, which this host
does not have. A stand-in `ffmpeg` on `PATH` made the movie with GStreamer 1.28 instead
(`videotestsrc`, `x264enc`, `avenc_ac3`, `subparse`, `matroskamux`): 1920 x 1080 H.264
High with B-frames, ten frames of each of the scenario's colours at 10 fps, two silent
AC-3 5.1 tracks and a subtitle track, in Matroska under the scenario's name `h264.mp4`.

- With this change: PASS in 12.6 s. The movie's first frame came 414 ms after F3; the
  seek to three quarters, Home and Play to the end showed no media error. PipeWire
  listed the suite's ALSA playback stream (S16LE, stereo, 48 kHz) during the run.
- With the parent commit's code, built the same way: FAIL, "F3 on h264.mp4: the viewer
  reports "ALSA underrun left unconfirmed downstream media"". The viewer's status held
  the same sentence, which the former status line repeats after the media error.

## Limits

- Only PipeWire's ALSA plugin on this host was recorded and run. Hardware (`hw:`)
  devices, PulseAudio's plugin and other distributions were not run; for them the
  unchanged confirmation rule and the unit tests apply.
- The end-to-end movie is two seconds of solid colour. Whether a long 1080p film
  underruns at its start on this host was not measured; the native slow-start test
  stands in for that start with 300 ms without sound.
- On PipeWire the last partial quantum of a movie's sound (under 22 ms) never plays: the
  plugin keeps it in the ring until the stream is prepared again. The clock does not
  count it.
