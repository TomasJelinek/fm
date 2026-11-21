//! The decoder never receives paths, inherited files, network access or command arguments.
use midnight_filesystem_api::*;
use midnight_fs_archive::broker::Broker;
use midnight_fs_archive::{Archive, Error, Limits};
use std::{io, rc::Rc};

fn main() {
    // Both parent and child enforce containment. No untrusted bytes precede this handshake.
    let mut input = io::stdin();
    let mut output = io::stdout();
    if midnight_worker_sdk::enter(&mut input, &mut output).is_err() {
        return;
    }
    let _ = serve(Broker::new(input, output));
}

fn serve(broker: Broker) -> io::Result<()> {
    let mut archive = None;
    loop {
        // The broker rejects non-increasing host request IDs before returning.
        let message = broker.receive()?;
        let request = message.request();
        let result: Result<Message, Error> = match message {
            Message::Open { format, length, .. } if archive.is_none() => {
                let limits = Limits::default();
                match Archive::open_stream(
                    format,
                    broker.source(length),
                    length,
                    Rc::new(broker.clone()),
                    limits,
                ) {
                    Ok(value) => {
                        archive = Some(value);
                        Ok(Message::Opened { request, root: 1 })
                    }
                    Err(error) => Err(error),
                }
            }
            Message::List {
                directory, after, ..
            } => match archive.as_ref() {
                Some(archive) => {
                    archive
                        .page(directory, after)
                        .map(|(entries, more)| Message::Entries {
                            request,
                            more,
                            entries,
                        })
                }
                None => Err(unopened()),
            },
            Message::Read { entry, .. } => match archive.as_mut() {
                Some(archive) => archive.prepare(entry).map(|content| Message::ContentReady {
                    request,
                    entry,
                    stage: content.stage,
                    offset: content.offset,
                    length: content.length,
                }),
                None => Err(unopened()),
            },
            // The password moves into the archive, or is dropped (and zeroized) here.
            Message::Unlock { password, .. } => match archive.as_mut() {
                Some(archive) => archive
                    .unlock(password)
                    .map(|status| Message::UnlockReply { request, status }),
                None => Err(unopened()),
            },
            Message::Close { .. } => return Ok(()),
            _ => Err(Error {
                code: ErrorCode::Unsupported,
                message: "Unsupported read-only archive operation".into(),
            }),
        };
        broker.send(&result.unwrap_or_else(|error| Message::Failed {
            request,
            code: error.code,
            message: error.message.chars().take(1024).collect(),
        }))?;
    }
}
fn unopened() -> Error {
    Error {
        code: ErrorCode::Invalid,
        message: "Archive is not open".into(),
    }
}
