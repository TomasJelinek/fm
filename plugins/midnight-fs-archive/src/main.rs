//! The decoder never receives paths, inherited files, network access or command arguments.
use midnight_filesystem_api::*;
use midnight_fs_archive::{Archive, Error, Limits};
use std::io::{self, Read, Write};

fn main() {
    // Both parent and child enforce containment. No untrusted bytes precede this handshake.
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    if midnight_worker_sdk::enter(&mut input, &mut output).is_err() {
        return;
    }
    let _ = serve(&mut input, &mut output);
}

fn serve(input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    let mut archive = None;
    let mut last_request = 0;
    loop {
        let message = read_message(input)?;
        let request = message.request();
        if request <= last_request {
            return Err(io::ErrorKind::InvalidData.into());
        }
        last_request = request;
        let result: Result<Message, Error> = match message {
            Message::Open { format, length, .. } if archive.is_none() => {
                let limits = Limits::default();
                if length > limits.max_source {
                    Err(Error {
                        code: ErrorCode::Limit,
                        message: "Archive exceeds the 64 MiB source budget".into(),
                    })
                } else {
                    let mut bytes = Vec::with_capacity(length as usize);
                    while bytes.len() < length as usize {
                        let count = ((length as usize - bytes.len()).min(MAX_READ as usize)) as u32;
                        write_message(
                            output,
                            &Message::SourceRead {
                                request,
                                offset: bytes.len() as u64,
                                length: count,
                            },
                        )?;
                        match read_message(input)? {
                            Message::SourceData {
                                request: reply,
                                bytes: data,
                            } if reply == request && data.len() == count as usize => {
                                bytes.extend(data)
                            }
                            Message::Failed { .. } => return Ok(()),
                            _ => return Err(io::ErrorKind::InvalidData.into()),
                        }
                    }
                    match Archive::open(format, bytes, limits) {
                        Ok(value) => {
                            archive = Some(value);
                            Ok(Message::Opened { request, root: 1 })
                        }
                        Err(error) => Err(error),
                    }
                }
            }
            Message::List {
                directory, after, ..
            } => match archive.as_ref() {
                Some(archive) => archive.list(directory, after).and_then(|entries| {
                    let mut bytes = 32usize;
                    let entries: Vec<_> = entries
                        .into_iter()
                        .take_while(|entry| {
                            bytes =
                                bytes.saturating_add(entry.name.len() + entry.display.len() + 40);
                            bytes <= MAX_FRAME
                        })
                        .collect();
                    let more = if let Some(last) = entries.last() {
                        !archive.list(directory, last.id)?.is_empty()
                    } else {
                        false
                    };
                    Ok(Message::Entries {
                        request,
                        more,
                        entries,
                    })
                }),
                None => Err(unopened()),
            },
            Message::Read {
                entry,
                offset,
                length,
                ..
            } => match archive.as_mut() {
                Some(archive) => {
                    archive
                        .read(entry, offset, length)
                        .map(|(bytes, eof)| Message::Content {
                            request,
                            bytes,
                            eof,
                        })
                }
                None => Err(unopened()),
            },
            Message::Close { .. } => return Ok(()),
            _ => Err(Error {
                code: ErrorCode::Unsupported,
                message: "Unsupported read-only archive operation".into(),
            }),
        };
        write_message(
            output,
            &result.unwrap_or_else(|error| Message::Failed {
                request,
                code: error.code,
                message: error.message.chars().take(1024).collect(),
            }),
        )?;
    }
}
fn unopened() -> Error {
    Error {
        code: ErrorCode::Invalid,
        message: "Archive is not open".into(),
    }
}
