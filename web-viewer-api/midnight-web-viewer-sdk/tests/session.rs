use midnight_web_viewer_api::*;
use midnight_web_viewer_sdk::*;

fn header(sequence: u64) -> Header {
    Header {
        session: 8,
        document: 3,
        sequence,
    }
}
fn viewport(revision: u64) -> Viewport {
    Viewport {
        revision,
        width: 2,
        height: 1,
        scale: 1.0,
    }
}
fn command(sequence: u64, kind: CommandKind) -> Message {
    Message::Command(Command {
        header: header(sequence),
        kind,
    })
}
fn event(sequence: u64, kind: EventKind) -> Message {
    Message::Event(Event {
        header: header(sequence),
        kind,
    })
}
fn open() -> Message {
    command(
        1,
        CommandKind::Open {
            source: Resource {
                id: 11,
                length: 4,
                mime: "text/html".into(),
            },
            viewport: viewport(1),
            policy: Policy::default(),
        },
    )
}
fn frame(sequence: u64, revision: u64) -> Event {
    Event {
        header: header(sequence),
        kind: EventKind::Frame(Frame {
            viewport_revision: revision,
            acknowledged_input: 1,
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        }),
    }
}

#[test]
fn session_rejects_stale_generation_sequence_and_paints_without_mutating_state() {
    let mut session = Session::new(8, 3).unwrap();
    session.accept(&open()).unwrap();
    session.accept(&event(1, EventKind::Ready)).unwrap();
    session
        .accept(&command(2, CommandKind::Resize(viewport(2))))
        .unwrap();
    assert!(session.accept(&Message::Event(frame(2, 1))).is_err());
    let current = Message::Event(frame(2, 2));
    let mut stale = current.clone();
    if let Message::Event(event) = &mut stale {
        event.header.document = 2;
    }
    assert!(session.accept(&stale).is_err());
    session.accept(&current).unwrap();
    assert!(session.accept(&current).is_err());
    assert!(session.accept(&command(2, CommandKind::Copy)).is_err());
    session.accept(&command(3, CommandKind::Copy)).unwrap();
}

#[test]
fn session_rejects_future_ack_and_closing_frames_and_ends_ownership() {
    let mut session = Session::new(8, 3).unwrap();
    assert!(session.accept(&command(1, CommandKind::Copy)).is_err());
    session.accept(&open()).unwrap();
    session.accept(&event(1, EventKind::Ready)).unwrap();
    let mut future = frame(2, 1);
    if let EventKind::Frame(frame) = &mut future.kind {
        frame.acknowledged_input = 3;
    }
    assert!(session.accept(&Message::Event(future)).is_err());
    session.accept(&command(2, CommandKind::Close)).unwrap();
    assert!(session.accept(&Message::Event(frame(2, 1))).is_err());
    session.accept(&event(2, EventKind::Closed)).unwrap();
    assert!(session.is_closed());
    assert!(session.accept(&command(3, CommandKind::Copy)).is_err());
}

fn queued(sequence: u64, kind: CommandKind) -> Command {
    Command {
        header: header(sequence),
        kind,
    }
}
fn movement(sequence: u64) -> Command {
    queued(
        sequence,
        CommandKind::Pointer(Pointer {
            kind: PointerKind::Move,
            button: PointerButton::None,
            x: sequence as f64,
            y: 1.0,
            modifiers: Modifiers::default(),
            click_count: 0,
        }),
    )
}

#[test]
fn command_queue_only_coalesces_adjacent_movement_and_returns_displaced_ownership() {
    let mut queue = CommandQueue::new(2).unwrap();
    assert!(queue.push(movement(1)).unwrap().is_none());
    assert_eq!(queue.push(movement(2)).unwrap(), Some(movement(1)));
    queue.push(queued(3, CommandKind::Copy)).unwrap();
    assert_eq!(queue.push(movement(4)).unwrap_err(), movement(4));
    assert_eq!(queue.pop(), Some(movement(2)));
    assert_eq!(queue.pop(), Some(queued(3, CommandKind::Copy)));
    assert!(queue.pop().is_none());
}

#[test]
fn frame_window_requires_exact_ack_and_returns_owned_frame_on_pressure() {
    let mut frames = FrameWindow::new(1).unwrap();
    frames.retain(frame(2, 1)).unwrap();
    assert_eq!(frames.retain(frame(3, 1)).unwrap_err(), frame(3, 1));
    assert!(frames.acknowledge(3).is_none());
    assert_eq!(frames.acknowledge(2), Some(frame(2, 1)));
    assert!(frames.acknowledge(2).is_none());
    frames.retain(frame(3, 1)).unwrap();
    assert_eq!(frames.clear(), vec![frame(3, 1)]);
}

#[test]
fn generic_transport_handles_back_to_back_packets_and_preserves_io_ownership() {
    let mut bytes = Vec::new();
    write_message(&mut bytes, &open()).unwrap();
    write_message(&mut bytes, &event(1, EventKind::Ready)).unwrap();
    let mut transport = Transport::new(bytes.as_slice(), Vec::new());
    assert_eq!(transport.receive().unwrap(), open());
    assert_eq!(transport.receive().unwrap(), event(1, EventKind::Ready));
    transport.send(&command(2, CommandKind::Close)).unwrap();
    let (_, output) = transport.into_inner();
    assert_eq!(
        read_message(&mut output.as_slice()).unwrap(),
        command(2, CommandKind::Close)
    );
}
