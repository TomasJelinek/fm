//! Worker-local playback demand. The host owns pacing and the playback clock.
use crate::media::{AudioTrack, Block, Frame, MAX_PCM_FRAMES, Metadata};
use std::{fmt, io, num::NonZeroU64};

pub const MAX_PULL_FRAMES: u8 = 4;

pub struct Request {
    pub id: NonZeroU64,
    pub command: Command,
}

pub enum Command {
    Pull {
        generation: NonZeroU64,
        frames: u8,
        pcm_frames: u16,
    },
    Seek {
        generation: NonZeroU64,
        position_micros: u64,
    },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Served,
    EndOfMedia,
    Superseded,
}

pub enum Event {
    Video {
        request: NonZeroU64,
        generation: NonZeroU64,
        frame: Frame,
    },
    Pcm {
        request: NonZeroU64,
        generation: NonZeroU64,
        block: Block,
    },
    Completed {
        request: NonZeroU64,
        generation: NonZeroU64,
        completion: Completion,
    },
}

/// Only a decoder failure is safe to report to the host as a terminal result
/// for a command. A broken transport cannot accept another event.
#[derive(Debug)]
pub enum Failure {
    Transport(io::Error),
    Protocol(&'static str),
    Decoder {
        request: NonZeroU64,
        generation: NonZeroU64,
        cause: io::Error,
    },
}

impl Failure {
    pub fn kind(&self) -> io::ErrorKind {
        match self {
            Self::Transport(cause) | Self::Decoder { cause, .. } => cause.kind(),
            Self::Protocol(_) => io::ErrorKind::InvalidData,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(cause) => write!(f, "Playback transport failed with {cause}"),
            Self::Protocol(message) => f.write_str(message),
            Self::Decoder {
                request,
                generation,
                cause,
            } => write!(
                f,
                "Playback request {request} in generation {generation} failed with {cause}"
            ),
        }
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(cause) | Self::Decoder { cause, .. } => Some(cause),
            Self::Protocol(_) => None,
        }
    }
}

/// An adapter on the locked-down worker thread. It forwards each frame before
/// the decoder advances and bounds its own framing and buffering.
pub trait Transport {
    fn receive(&mut self) -> io::Result<Option<Request>>;
    fn emit(&mut self, event: Event) -> io::Result<()>;
}

/// The decoder contract shared by the bundled FFmpeg profile and the platform decoders
/// (Media Foundation on Windows; the distribution's FFmpeg, then GStreamer, on Linux). One
/// worker thread drives it; each call may read the capture through the host broker.
pub trait PlaybackDecoder {
    /// Container duration when known and the original, oriented video dimensions
    /// (quarter turns swap the axes), at most 32 mebipixels.
    fn metadata(&self) -> Metadata;
    /// `Qualified` only when `pcm` produces the track.
    fn audio_track(&self) -> AudioTrack;
    /// The next presentation frame: oriented straight-alpha sRGB RGBA8 fitted within the
    /// output bounds, never enlarged; microseconds from the container start, nondecreasing
    /// until the next seek. Frames before the last seek target are never returned. `None` at
    /// the end stays until a seek. Any error is terminal.
    fn next_frame(&mut self) -> io::Result<Option<Frame>>;
    /// Reposition video and audio; the next frame is the first at or after the target.
    fn seek(&mut self, position_micros: u64) -> io::Result<()>;
    /// Up to `credit` (1 to `MAX_PCM_FRAMES`) 48 kHz stereo frames. `first_frame` counts from
    /// the last seek target and blocks are contiguous; gaps, and the tail up to the container
    /// duration, are silence. `None` once the audio and that tail have ended.
    fn pcm(&mut self, credit: u16) -> io::Result<Option<Block>>;
}

pub fn run(
    decoder: &mut dyn PlaybackDecoder,
    mut current: NonZeroU64,
    transport: &mut dyn Transport,
) -> Result<(), Failure> {
    let mut previous = None;
    let mut video_ended = false;
    let mut audio_ended = false;
    while let Some(Request { id, command }) = transport.receive().map_err(Failure::Transport)? {
        if previous.is_some_and(|previous| id <= previous) {
            return Err(Failure::Protocol("Out-of-order playback request"));
        }
        previous = Some(id);
        let (generation, completion) = match command {
            Command::Close => return Ok(()),
            Command::Pull {
                generation,
                frames,
                pcm_frames,
            } => {
                if (frames == 0 && pcm_frames == 0)
                    || frames > MAX_PULL_FRAMES
                    || pcm_frames > MAX_PCM_FRAMES
                    || generation > current
                {
                    return Err(Failure::Protocol("Invalid playback credit or generation"));
                }
                let outcome = if generation < current {
                    Completion::Superseded
                } else {
                    if !video_ended {
                        video_ended = pull_video(decoder, transport, id, generation, frames)?;
                    }
                    if pcm_frames > 0 && !audio_ended {
                        let block = decoder.pcm(pcm_frames).map_err(|cause| Failure::Decoder {
                            request: id,
                            generation,
                            cause,
                        })?;
                        if let Some(block) = block {
                            transport
                                .emit(Event::Pcm {
                                    request: id,
                                    generation,
                                    block,
                                })
                                .map_err(Failure::Transport)?;
                        } else {
                            audio_ended = true;
                        }
                    }
                    if video_ended && (pcm_frames == 0 || audio_ended) {
                        Completion::EndOfMedia
                    } else {
                        Completion::Served
                    }
                };
                (generation, outcome)
            }
            Command::Seek {
                generation,
                position_micros,
            } => {
                let outcome = if generation <= current {
                    Completion::Superseded
                } else {
                    decoder
                        .seek(position_micros)
                        .map_err(|cause| Failure::Decoder {
                            request: id,
                            generation,
                            cause,
                        })?;
                    current = generation;
                    video_ended = false;
                    audio_ended = false;
                    Completion::Served
                };
                (generation, outcome)
            }
        };
        transport
            .emit(Event::Completed {
                request: id,
                generation,
                completion,
            })
            .map_err(Failure::Transport)?;
    }
    Ok(())
}

fn pull_video(
    decoder: &mut dyn PlaybackDecoder,
    transport: &mut dyn Transport,
    request: NonZeroU64,
    generation: NonZeroU64,
    frames: u8,
) -> Result<bool, Failure> {
    for _ in 0..frames {
        let Some(frame) = decoder.next_frame().map_err(|cause| Failure::Decoder {
            request,
            generation,
            cause,
        })?
        else {
            return Ok(true);
        };
        transport
            .emit(Event::Video {
                request,
                generation,
                frame,
            })
            .map_err(Failure::Transport)?;
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn tag(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap()
    }

    #[derive(Default)]
    struct Decoder {
        times: VecDeque<i64>,
        seeks: Vec<u64>,
        reads: usize,
        fail_read: Option<usize>,
        fail_seek: bool,
    }

    impl PlaybackDecoder for Decoder {
        fn metadata(&self) -> Metadata {
            Metadata {
                duration_micros: None,
                width: 2,
                height: 1,
            }
        }

        fn audio_track(&self) -> AudioTrack {
            AudioTrack::Absent
        }

        fn pcm(&mut self, _: u16) -> io::Result<Option<Block>> {
            Ok(None)
        }

        fn next_frame(&mut self) -> io::Result<Option<Frame>> {
            self.reads += 1;
            if self.fail_read == Some(self.reads) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "broker denied read",
                ));
            }
            Ok(self.times.pop_front().map(|presentation_micros| Frame {
                presentation_micros,
                width: 2,
                height: 1,
                rgba: vec![0; 8],
            }))
        }

        fn seek(&mut self, position_micros: u64) -> io::Result<()> {
            if self.fail_seek {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "broker interrupted seek",
                ));
            }
            self.seeks.push(position_micros);
            self.times = [position_micros as i64].into();
            Ok(())
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Output {
        Frame(u64, u64, i64),
        Completion(u64, u64, Completion),
    }

    #[derive(Default)]
    struct Pipe {
        input: VecDeque<Request>,
        output: Vec<Output>,
        fail_emit: bool,
        emit_attempts: usize,
    }

    impl Transport for Pipe {
        fn receive(&mut self) -> io::Result<Option<Request>> {
            Ok(self.input.pop_front())
        }

        fn emit(&mut self, event: Event) -> io::Result<()> {
            self.emit_attempts += 1;
            if self.fail_emit {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            self.output.push(match event {
                Event::Video {
                    request,
                    generation,
                    frame,
                } => Output::Frame(request.get(), generation.get(), frame.presentation_micros),
                Event::Pcm { .. } => panic!("test decoder has no PCM"),
                Event::Completed {
                    request,
                    generation,
                    completion,
                } => Output::Completion(request.get(), generation.get(), completion),
            });
            Ok(())
        }
    }

    fn pull(id: u64, generation: u64, frames: u8) -> Request {
        Request {
            id: tag(id),
            command: Command::Pull {
                generation: tag(generation),
                frames,
                pcm_frames: 0,
            },
        }
    }

    #[test]
    fn seek_starts_a_new_generation_and_stale_pulls_never_decode() {
        let mut decoder = Decoder {
            times: [0, 40_000].into(),
            ..Decoder::default()
        };
        let mut pipe = Pipe {
            input: [
                pull(1, 10, 3),
                Request {
                    id: tag(2),
                    command: Command::Seek {
                        generation: tag(11),
                        position_micros: 120_000,
                    },
                },
                pull(3, 10, 1),
                pull(4, 11, 2),
                Request {
                    id: tag(5),
                    command: Command::Close,
                },
                pull(6, 11, 1),
            ]
            .into(),
            ..Pipe::default()
        };
        run(&mut decoder, tag(10), &mut pipe).unwrap();
        assert_eq!(decoder.seeks, [120_000]);
        assert_eq!(decoder.reads, 5);
        assert_eq!(
            pipe.output,
            [
                Output::Frame(1, 10, 0),
                Output::Frame(1, 10, 40_000),
                Output::Completion(1, 10, Completion::EndOfMedia),
                Output::Completion(2, 11, Completion::Served),
                Output::Completion(3, 10, Completion::Superseded),
                Output::Frame(4, 11, 120_000),
                Output::Completion(4, 11, Completion::EndOfMedia),
            ]
        );
        assert_eq!(pipe.input.len(), 1);
    }

    #[test]
    fn bad_credit_and_future_generation_fail_before_decoding() {
        for request in [pull(1, 10, 0), pull(1, 10, 5), pull(1, 11, 1)] {
            let mut decoder = Decoder {
                times: [0].into(),
                ..Decoder::default()
            };
            let mut pipe = Pipe {
                input: [request].into(),
                ..Pipe::default()
            };
            assert_eq!(
                run(&mut decoder, tag(10), &mut pipe).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(decoder.reads, 0);
            assert!(pipe.output.is_empty());
        }
    }

    #[test]
    fn duplicate_request_cannot_decode_the_same_credit_twice() {
        let mut decoder = Decoder {
            times: [0, 40_000].into(),
            ..Decoder::default()
        };
        let mut pipe = Pipe {
            input: [pull(1, 10, 1), pull(1, 10, 1)].into(),
            ..Pipe::default()
        };
        assert_eq!(
            run(&mut decoder, tag(10), &mut pipe).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(decoder.reads, 1);
        assert_eq!(
            pipe.output,
            [
                Output::Frame(1, 10, 0),
                Output::Completion(1, 10, Completion::Served),
            ]
        );
    }

    #[test]
    fn failed_partial_pull_keeps_its_request_and_generation() {
        let mut decoder = Decoder {
            times: [0].into(),
            fail_read: Some(2),
            ..Decoder::default()
        };
        let mut pipe = Pipe {
            input: [pull(17, 10, 2)].into(),
            ..Pipe::default()
        };
        let Err(Failure::Decoder {
            request,
            generation,
            cause,
        }) = run(&mut decoder, tag(10), &mut pipe)
        else {
            panic!("decoder failure lost its command identity");
        };
        assert_eq!((request, generation), (tag(17), tag(10)));
        assert_eq!(cause.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(cause.to_string(), "broker denied read");
        assert_eq!(pipe.output, [Output::Frame(17, 10, 0)]);
    }

    #[test]
    fn failed_seek_reports_the_requested_generation() {
        let mut decoder = Decoder {
            fail_seek: true,
            ..Decoder::default()
        };
        let mut pipe = Pipe {
            input: [Request {
                id: tag(9),
                command: Command::Seek {
                    generation: tag(11),
                    position_micros: 80_000,
                },
            }]
            .into(),
            ..Pipe::default()
        };
        let Err(Failure::Decoder {
            request,
            generation,
            cause,
        }) = run(&mut decoder, tag(10), &mut pipe)
        else {
            panic!("seek failure lost its command identity");
        };
        assert_eq!((request, generation), (tag(9), tag(11)));
        assert_eq!(cause.kind(), io::ErrorKind::Interrupted);
        assert!(pipe.output.is_empty());
    }

    #[test]
    fn broken_transport_is_never_retried_as_a_decoder_failure() {
        let mut decoder = Decoder {
            times: [0, 40_000].into(),
            ..Decoder::default()
        };
        let mut pipe = Pipe {
            input: [pull(8, 10, 2)].into(),
            fail_emit: true,
            ..Pipe::default()
        };
        let Err(Failure::Transport(cause)) = run(&mut decoder, tag(10), &mut pipe) else {
            panic!("the session must stop when output cannot be sent");
        };
        assert_eq!(cause.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(decoder.reads, 1);
        assert_eq!(pipe.emit_attempts, 1);
        assert!(pipe.output.is_empty());
    }
}
