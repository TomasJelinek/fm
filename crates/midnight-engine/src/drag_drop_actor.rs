use super::*;

pub(super) fn run(
    state: SharedState,
    service: Option<Arc<dyn FileDragDropService>>,
    mut commands: mpsc::Receiver<Start>,
    events: mpsc::Sender<DragDropEvent>,
    cancel: Cancellation,
) {
    let mut generation = None;
    let mut announced_ready = false;
    let mut session: Option<Box<dyn FileDragDropSession>> = None;
    while !cancel.is_cancelled() {
        let host = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .host
            .clone()
            .filter(|host| !host.host.lifetime.is_cancelled());
        let current = host.as_ref().map(|host| host.host.generation);
        if current != generation {
            session = None;
            announced_ready = false;
            generation = current;
            if let Some(host) = host {
                let result = if host.host.native == NativeDragWindow::Unsupported {
                    Ok(None)
                } else {
                    service
                        .as_ref()
                        .ok_or_else(|| {
                            error(
                                io::ErrorKind::Unsupported,
                                "Native file dragging is unavailable.",
                            )
                        })
                        .and_then(|service| {
                            service.open_session(host.host.clone(), host.targets, host.sink)
                        })
                        .map(Some)
                };
                let ready = match result {
                    Ok(opened) => {
                        session = opened;
                        true
                    }
                    Err(error) => {
                        if !publish(
                            &events,
                            DragDropEvent::Failed { id: None, error },
                            &cancel,
                            &mut session,
                        ) {
                            break;
                        }
                        false
                    }
                };
                announced_ready = ready && session.is_some();
                if !publish(
                    &events,
                    DragDropEvent::Ready {
                        host_generation: host.host.generation,
                        supported: announced_ready,
                    },
                    &cancel,
                    &mut session,
                ) {
                    break;
                }
            }
        }
        if let Some(native) = &mut session
            && let Err(error) = native.service_pending()
        {
            session = None;
            if !publish(
                &events,
                DragDropEvent::Failed { id: None, error },
                &cancel,
                &mut session,
            ) {
                break;
            }
        }
        // The backpressure pump can also lose the session while delivering an earlier event.
        // Revoke readiness before the next start can consume the UI's pointer capture.
        if session.is_none() && announced_ready {
            announced_ready = false;
            if let Some(host_generation) = generation
                && !publish(
                    &events,
                    DragDropEvent::Ready {
                        host_generation,
                        supported: false,
                    },
                    &cancel,
                    &mut session,
                )
            {
                break;
            }
        }
        let command = match commands.try_recv() {
            Ok(command) => command,
            Err(mpsc::error::TryRecvError::Empty) => {
                std::thread::park_timeout(Duration::from_millis(10));
                continue;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        };
        let result = if command.cancel.is_cancelled() || generation != Some(command.host) {
            Ok(DragFinish::Cancelled)
        } else if let Some(native) = &mut session {
            native.start_drag(&command.files, &command.cancel)
        } else {
            Ok(DragFinish::Unavailable)
        };
        {
            let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
            if state
                .source
                .as_ref()
                .is_some_and(|source| source.id == command.id)
            {
                if matches!(result, Ok(DragFinish::Unavailable)) {
                    state.source.as_mut().unwrap().running = false;
                } else {
                    state.source = None;
                }
            }
        }
        let event = match result {
            Ok(outcome) => DragDropEvent::Finished {
                id: command.id,
                outcome,
            },
            Err(error) => DragDropEvent::Failed {
                id: Some(command.id),
                error,
            },
        };
        if !publish(&events, event, &cancel, &mut session) {
            break;
        }
    }
    // Session and its COM/X11 objects are destroyed on their creating thread.
}

fn publish(
    events: &mpsc::Sender<DragDropEvent>,
    mut event: DragDropEvent,
    cancel: &Cancellation,
    session: &mut Option<Box<dyn FileDragDropSession>>,
) -> bool {
    while !cancel.is_cancelled() {
        match events.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(value)) => event = value,
        }
        if session
            .as_mut()
            .is_some_and(|native| native.service_pending().is_err())
        {
            *session = None;
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
    false
}
