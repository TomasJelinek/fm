//! Isolate the production browser pipe and raw protocol cost without launching Chromium.
use midnight_web_viewer_host::api;
use std::{io, time::Instant};
fn header() -> api::Header {
    api::Header {
        session: 1,
        document: 1,
        sequence: 1,
    }
}
fn child() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    for (width, height) in [(640, 480), (2187, 1467)] {
        let mut message = api::Message::Event(api::Event {
            header: header(),
            kind: api::EventKind::Frame(api::Frame {
                viewport_revision: 1,
                acknowledged_input: 0,
                width,
                height,
                rgba: vec![127; width as usize * height as usize * 4],
            }),
        });
        api::write_message(
            &mut output,
            &api::Message::Event(api::Event {
                header: header(),
                kind: api::EventKind::Ready,
            }),
        )?;
        for round in 0..12 {
            let _ = api::read_message(&mut input)?;
            if let api::Message::Event(event) = &mut message {
                event.header.sequence = round + 2;
            }
            api::write_message(&mut output, &message)?;
        }
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() == 1 {
        child()?;
        return Ok(());
    }
    let mut browser = midnight_worker_platform::browser::spawn(&std::env::current_exe()?)?;
    for _ in 0..2 {
        let ready = api::read_message(&mut browser.output)?;
        assert!(matches!(
            ready,
            api::Message::Event(api::Event {
                kind: api::EventKind::Ready,
                ..
            })
        ));
        for round in 0..12 {
            let started = Instant::now();
            api::write_message(
                &mut browser.input,
                &api::Message::Command(api::Command {
                    header: header(),
                    kind: api::CommandKind::Focus(true),
                }),
            )?;
            let message = api::read_message(&mut browser.output)?;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            let api::Message::Event(api::Event {
                kind: api::EventKind::Frame(frame),
                ..
            }) = message
            else {
                return Err("unexpected probe output".into());
            };
            println!(
                "pipe round={round} physical={}x{} transfer_ms={elapsed:.3}",
                frame.width, frame.height
            );
        }
    }
    browser.process.wait()?;
    Ok(())
}
