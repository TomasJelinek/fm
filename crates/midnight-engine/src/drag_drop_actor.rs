use super::*;
use std::sync::{
    PoisonError,
    mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError},
};

#[cfg(test)]
#[path = "drag_drop_actor_tests.rs"]
mod tests;

/// Pump interval of an open session without a waker, whose native queue is serviced
/// without blocking.
const PUMP_INTERVAL: Duration = Duration::from_millis(10);
/// Longest idle wait of an open session with a waker. Native input and every controller
/// wake end it. Its expiry bounds the effect of a lost wakeup (a platform defect), and
/// how long a session stays open after its host lifetime was cancelled without a detach
/// (which ends no wait), at 0.2 idle wakeups per second.
const IDLE_WAIT: Duration = Duration::from_secs(5);

/// The open session's waker: installed by the actor, called by the controller.
type Installed = Arc<Mutex<Option<FileDragDropWaker>>>;

/// The controller's half of the actor's wakeup.
pub(super) struct Wake {
    signal: SyncSender<()>,
    installed: Installed,
}
/// The actor's half of its wakeup.
pub(super) struct Woken {
    signal: Receiver<()>,
    installed: Installed,
}
pub(super) fn wake() -> (Wake, Woken) {
    let (signal, receiver) = std::sync::mpsc::sync_channel(1);
    let installed = Installed::default();
    (
        Wake {
            signal,
            installed: installed.clone(),
        },
        Woken {
            signal: receiver,
            installed,
        },
    )
}
impl Wake {
    /// Signals each controller change the actor must observe (a host attached or
    /// detached, a queued start, shutdown) and ends the wait of its open session. A
    /// session the actor replaces meanwhile can still receive this call; the signal
    /// covers that case, because the actor takes it before each session wait.
    pub(super) fn wake(&self) {
        // A full channel already holds a signal the actor has not taken.
        let _ = self.signal.try_send(());
        // Called outside the lock; a waker stays harmless after its session was dropped.
        let waker = self
            .installed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(waker) = waker {
            waker();
        }
    }
}

/// The open native session. It never leaves the actor thread; only its waker is shared.
struct Native {
    session: Option<Box<dyn FileDragDropSession>>,
    installed: Installed,
    /// The open session has a waker, so its idle waits need no short timer.
    wakes: bool,
}
impl Native {
    fn open(&mut self, session: Box<dyn FileDragDropSession>) {
        let waker = session.waker();
        self.wakes = waker.is_some();
        *self
            .installed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = waker;
        self.session = Some(session);
    }
    fn close(&mut self) {
        *self
            .installed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        self.wakes = false;
        self.session = None;
    }
}

/// The native actor. It waits for its open session's native input and for the
/// controller's wake (see `Wake`); without a session, for the controller only.
pub(super) fn run(
    state: SharedState,
    service: Option<Arc<dyn FileDragDropService>>,
    mut commands: mpsc::Receiver<Start>,
    woken: Woken,
    events: mpsc::Sender<DragDropEvent>,
    cancel: Cancellation,
) {
    let Woken { signal, installed } = woken;
    let mut generation = None;
    let mut announced_ready = false;
    let mut native = Native {
        session: None,
        installed,
        wakes: false,
    };
    while !cancel.is_cancelled() {
        let host = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .host
            .clone()
            .filter(|host| !host.host.lifetime.is_cancelled());
        let current = host.as_ref().map(|host| host.host.generation);
        if current != generation {
            native.close();
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
                        if let Some(opened) = opened {
                            native.open(opened);
                        }
                        true
                    }
                    Err(error) => {
                        if !publish(
                            &events,
                            DragDropEvent::Failed { id: None, error },
                            &cancel,
                            &mut native,
                        ) {
                            break;
                        }
                        false
                    }
                };
                announced_ready = ready && native.session.is_some();
                if !publish(
                    &events,
                    DragDropEvent::Ready {
                        host_generation: host.host.generation,
                        supported: announced_ready,
                    },
                    &cancel,
                    &mut native,
                ) {
                    break;
                }
            }
        }
        if let Some(session) = &mut native.session
            && let Err(error) = session.service_pending()
        {
            native.close();
            if !publish(
                &events,
                DragDropEvent::Failed { id: None, error },
                &cancel,
                &mut native,
            ) {
                break;
            }
        }
        // The backpressure pump can also lose the session while delivering an earlier event.
        // Revoke readiness before the next start can consume the UI's pointer capture.
        if native.session.is_none() && announced_ready {
            announced_ready = false;
            if let Some(host_generation) = generation
                && !publish(
                    &events,
                    DragDropEvent::Ready {
                        host_generation,
                        supported: false,
                    },
                    &cancel,
                    &mut native,
                )
            {
                break;
            }
        }
        let command = match commands.try_recv() {
            Ok(command) => command,
            Err(mpsc::error::TryRecvError::Empty) => {
                let mut lost = None;
                let woken = match native.session.as_mut() {
                    // Without a session only a controller change can matter.
                    None => signal.recv().map_err(RecvTimeoutError::from),
                    Some(_) if !native.wakes => signal.recv_timeout(PUMP_INTERVAL),
                    // A change signalled while a replaced session's waker was installed
                    // is found here, before the wait.
                    Some(session) => match signal.try_recv() {
                        Err(TryRecvError::Empty) => {
                            lost = session.wait_pending(IDLE_WAIT).err();
                            Ok(())
                        }
                        taken => taken.map_err(|_| RecvTimeoutError::Disconnected),
                    },
                };
                #[cfg(test)]
                {
                    state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .actor_wakeups += 1;
                }
                // The controller is gone.
                if woken == Err(RecvTimeoutError::Disconnected) {
                    break;
                }
                if let Some(error) = lost {
                    native.close();
                    if !publish(
                        &events,
                        DragDropEvent::Failed { id: None, error },
                        &cancel,
                        &mut native,
                    ) {
                        break;
                    }
                }
                continue;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        };
        let result = if command.cancel.is_cancelled() || generation != Some(command.host) {
            Ok(DragFinish::Cancelled)
        } else if let Some(session) = &mut native.session {
            command
                .captured
                .as_ref()
                .map_or(Ok(()), |source| {
                    if source.resolve(&command.cancel)? == *command.files {
                        Ok(())
                    } else {
                        Err(error(
                            io::ErrorKind::InvalidInput,
                            "The captured drag sources changed.",
                        ))
                    }
                })
                .and_then(|()| session.start_drag(&command.files, &command.cancel))
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
        if !publish(&events, event, &cancel, &mut native) {
            break;
        }
    }
    // Session and its COM/X11 objects are destroyed on their creating thread.
    native.close();
}

fn publish(
    events: &mpsc::Sender<DragDropEvent>,
    mut event: DragDropEvent,
    cancel: &Cancellation,
    native: &mut Native,
) -> bool {
    while !cancel.is_cancelled() {
        match events.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(value)) => event = value,
        }
        if native
            .session
            .as_mut()
            .is_some_and(|session| session.service_pending().is_err())
        {
            native.close();
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
    false
}
