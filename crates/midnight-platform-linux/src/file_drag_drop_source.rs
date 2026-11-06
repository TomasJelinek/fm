//! Copy-only XDND source: pointer ownership, bounded peer negotiation and cleanup.
use super::*;
use std::thread;
use x11rb::protocol::xproto::{GrabMode, GrabStatus, KeyButMask};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Route {
    window: Window,
    recipient: Window,
    version: u32,
}

pub(super) struct Outgoing {
    pub bytes: Arc<[u8]>,
    pub escape: Vec<u8>,
    pub cancelled: bool,
    route: Option<Route>,
    accepted: bool,
    awaiting: bool,
    finished: Option<bool>,
    dropped: bool,
    last_point: Option<(i16, i16)>,
    updated: Instant,
}

impl Session {
    pub(super) fn source_message(&mut self, event: &ClientMessageEvent, data: [u32; 5]) -> bool {
        if event.window != self.proxy {
            return false;
        }
        let Some(outgoing) = self.outgoing.as_mut() else {
            return false;
        };
        let Some(route) = outgoing.route else {
            return false;
        };
        if data[0] != route.window {
            return false;
        }
        if event.type_ == self.atoms.status {
            if outgoing.awaiting && !outgoing.dropped {
                outgoing.accepted = data[1] & 1 != 0 && data[4] == self.atoms.copy;
                outgoing.awaiting = false;
            }
            true
        } else if event.type_ == self.atoms.finished {
            if outgoing.dropped {
                outgoing.finished =
                    Some(route.version < 5 || (data[1] & 1 != 0 && data[2] == self.atoms.copy));
            }
            true
        } else {
            false
        }
    }

    fn route_at_pointer(&self) -> Result<Option<Route>, FsError> {
        let mut window = self.root;
        // Only top-level XDND-aware windows qualify; descend through WM frames.
        for _ in 0..64 {
            let reply = self
                .connection
                .query_pointer(window)
                .map_err(failure)?
                .reply()
                .map_err(failure)?;
            if !reply.same_screen {
                return Ok(None);
            }
            if window != self.root && window != self.proxy {
                let proxy = single_property(
                    &self.connection,
                    window,
                    self.atoms.proxy,
                    AtomEnum::WINDOW.into(),
                )
                .ok()
                .flatten();
                let recipient = proxy
                    .filter(|proxy| {
                        *proxy != NONE
                            && single_property(
                                &self.connection,
                                *proxy,
                                self.atoms.proxy,
                                AtomEnum::WINDOW.into(),
                            )
                            .ok()
                            .flatten()
                                == Some(*proxy)
                    })
                    .unwrap_or(window);
                if let Some(version) = single_property(
                    &self.connection,
                    recipient,
                    self.atoms.aware,
                    AtomEnum::ATOM.into(),
                )
                .ok()
                .flatten()
                .filter(|v| *v >= 3)
                {
                    return Ok(Some(Route {
                        window,
                        recipient,
                        version: version.min(5),
                    }));
                }
            }
            if reply.child == NONE || reply.child == window {
                break;
            }
            window = reply.child;
        }
        Ok(None)
    }

    fn escape_keys(&self) -> Result<Vec<u8>, FsError> {
        let first = self.connection.setup().min_keycode;
        let count = self
            .connection
            .setup()
            .max_keycode
            .saturating_sub(first)
            .saturating_add(1);
        let reply = self
            .connection
            .get_keyboard_mapping(first, count)
            .map_err(failure)?
            .reply()
            .map_err(failure)?;
        if reply.keysyms_per_keycode == 0 {
            return Err(failure("The X11 keyboard mapping is empty."));
        }
        Ok(reply
            .keysyms
            .chunks(reply.keysyms_per_keycode as usize)
            .enumerate()
            .filter_map(|(index, keysyms)| keysyms.contains(&0xff1b).then_some(first + index as u8))
            .collect())
    }

    pub(super) fn run_source(
        &mut self,
        files: &FileClipboard,
        cancel: &Cancellation,
    ) -> Result<DragFinish, FsError> {
        files.validate()?;
        if files.intent != FileClipboardIntent::Copy {
            return Err(failure("Native dragging supports Copy only."));
        }
        if cancel.is_cancelled() || self.host.lifetime.is_cancelled() {
            return Ok(DragFinish::Cancelled);
        }
        let pointer = self
            .connection
            .query_pointer(self.root)
            .map_err(failure)?
            .reply()
            .map_err(failure)?;
        if !pointer.same_screen || !pointer.mask.contains(KeyButMask::BUTTON1) {
            return Ok(DragFinish::Cancelled);
        }
        let bytes = crate::file_clipboard::encode_uris(files)?.into();
        let escape = self.escape_keys()?;
        let handoff = Instant::now();
        let status = loop {
            let status = self
                .connection
                .grab_pointer(
                    false,
                    self.window,
                    EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    NONE,
                    NONE,
                    CURRENT_TIME,
                )
                .map_err(failure)?
                .reply()
                .map_err(failure)?
                .status;
            if status != GrabStatus::ALREADY_GRABBED
                || handoff.elapsed() >= Duration::from_millis(150)
            {
                break status;
            }
            // XFlush on the UI connection sends its ungrab without blocking.
            // The server may service this connection first; retry on the actor.
            let pointer = self
                .connection
                .query_pointer(self.root)
                .map_err(failure)?
                .reply()
                .map_err(failure)?;
            if cancel.is_cancelled()
                || self.host.lifetime.is_cancelled()
                || !pointer.mask.contains(KeyButMask::BUTTON1)
            {
                return Ok(DragFinish::Cancelled);
            }
            thread::sleep(Duration::from_millis(5));
        };
        if status != GrabStatus::SUCCESS {
            return Ok(DragFinish::Unavailable);
        }
        self.grabbed_pointer = true;
        let grabbed = self
            .connection
            .grab_keyboard(
                false,
                self.window,
                CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .map_err(failure)?
            .reply()
            .map_err(failure)?;
        if grabbed.status != GrabStatus::SUCCESS {
            return Ok(DragFinish::Unavailable);
        }
        self.grabbed_keyboard = true;
        // Preparation, serialization and grab round trips can outlast the user's
        // gesture. A release before ownership is established cannot start a drag.
        let pointer = self
            .connection
            .query_pointer(self.root)
            .map_err(failure)?
            .reply()
            .map_err(failure)?;
        if cancel.is_cancelled()
            || self.host.lifetime.is_cancelled()
            || !pointer.mask.contains(KeyButMask::BUTTON1)
        {
            return Ok(DragFinish::Cancelled);
        }
        self.connection
            .change_property32(
                PropMode::REPLACE,
                self.proxy,
                self.atoms.action_list,
                AtomEnum::ATOM,
                &[self.atoms.copy],
            )
            .map_err(failure)?
            .check()
            .map_err(failure)?;
        self.connection
            .set_selection_owner(self.proxy, self.atoms.selection, CURRENT_TIME)
            .map_err(failure)?
            .check()
            .map_err(failure)?;
        if self
            .connection
            .get_selection_owner(self.atoms.selection)
            .map_err(failure)?
            .reply()
            .map_err(failure)?
            .owner
            != self.proxy
        {
            return Ok(DragFinish::Unavailable);
        }
        self.outgoing = Some(Outgoing {
            bytes,
            escape,
            cancelled: false,
            route: None,
            accepted: false,
            awaiting: false,
            finished: None,
            dropped: false,
            last_point: None,
            updated: Instant::now(),
        });
        let began = Instant::now();
        let mut released = false;
        loop {
            if cancel.is_cancelled()
                || self.host.lifetime.is_cancelled()
                || began.elapsed() > Duration::from_secs(120)
            {
                return Ok(DragFinish::Cancelled);
            }
            self.service_pending()?;
            let outgoing = self.outgoing.as_ref().unwrap();
            if outgoing.cancelled {
                return Ok(DragFinish::Cancelled);
            }
            if let Some(accepted) = outgoing.finished {
                return Ok(if accepted {
                    DragFinish::Copied
                } else {
                    DragFinish::Cancelled
                });
            }
            if outgoing.dropped {
                if outgoing.updated.elapsed() >= TRANSFER_TIMEOUT {
                    return Ok(DragFinish::Cancelled);
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            let pointer = self
                .connection
                .query_pointer(self.root)
                .map_err(failure)?
                .reply()
                .map_err(failure)?;
            released |= !pointer.mask.contains(KeyButMask::BUTTON1);
            let point = (pointer.root_x, pointer.root_y);
            let route = self.route_at_pointer()?;
            if route != self.outgoing.as_ref().unwrap().route {
                if let Some(old) = self.outgoing.as_ref().unwrap().route {
                    self.message(
                        old.recipient,
                        old.window,
                        self.atoms.leave,
                        [self.proxy, 0, 0, 0, 0],
                    )?;
                }
                let outgoing = self.outgoing.as_mut().unwrap();
                outgoing.route = route;
                outgoing.accepted = false;
                outgoing.awaiting = false;
                outgoing.last_point = None;
                if let Some(route) = route {
                    self.message(
                        route.recipient,
                        route.window,
                        self.atoms.enter,
                        [self.proxy, route.version << 24, self.atoms.uri, 0, 0],
                    )?;
                }
            }
            let Some(route) = route else {
                if released {
                    return Ok(DragFinish::Cancelled);
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            };
            let outgoing = self.outgoing.as_mut().unwrap();
            if outgoing.awaiting {
                if outgoing.updated.elapsed() >= TRANSFER_TIMEOUT {
                    return Ok(DragFinish::Cancelled);
                }
            } else if outgoing.last_point != Some(point)
                || (!released && outgoing.updated.elapsed() > Duration::from_millis(100))
            {
                outgoing.last_point = Some(point);
                outgoing.awaiting = true;
                outgoing.accepted = false;
                outgoing.updated = Instant::now();
                let packed = (u32::from(point.0 as u16) << 16) | u32::from(point.1 as u16);
                self.message(
                    route.recipient,
                    route.window,
                    self.atoms.position,
                    [self.proxy, 0, packed, self.last_time, self.atoms.copy],
                )?;
            } else if released {
                if !outgoing.accepted {
                    return Ok(DragFinish::Cancelled);
                }
                outgoing.dropped = true;
                outgoing.updated = Instant::now();
                self.message(
                    route.recipient,
                    route.window,
                    self.atoms.drop,
                    [self.proxy, 0, self.last_time, 0, 0],
                )?;
                self.release_grabs();
            }
            self.connection.flush().map_err(failure)?;
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn release_grabs(&mut self) {
        if self.grabbed_pointer {
            let _ = self.connection.ungrab_pointer(CURRENT_TIME);
            self.grabbed_pointer = false;
        }
        if self.grabbed_keyboard {
            let _ = self.connection.ungrab_keyboard(CURRENT_TIME);
            self.grabbed_keyboard = false;
        }
    }

    pub(super) fn clear_source(&mut self) {
        if let Some(outgoing) = self.outgoing.take() {
            if !outgoing.dropped
                && let Some(route) = outgoing.route
            {
                let _ = self.message(
                    route.recipient,
                    route.window,
                    self.atoms.leave,
                    [self.proxy, 0, 0, 0, 0],
                );
            }
            if self
                .connection
                .get_selection_owner(self.atoms.selection)
                .ok()
                .and_then(|r| r.reply().ok())
                .is_some_and(|r| r.owner == self.proxy)
            {
                let _ =
                    self.connection
                        .set_selection_owner(NONE, self.atoms.selection, CURRENT_TIME);
            }
        }
        self.release_grabs();
        self.transfers.clear();
        let _ = self.connection.flush();
    }
}
