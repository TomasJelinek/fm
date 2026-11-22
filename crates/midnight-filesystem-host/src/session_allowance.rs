//! The allowance of archives opened from native files for panels, tabs and viewers
//! (`MAX_ACTIVE`), with the hand-off of a replaced session's slot.
//!
//! Every counted slot is one archive session. A navigation that leaves its tab's archive
//! for another one names the session it replaces. While that session still counts, the
//! new open becomes its only successor and waits, without blocking a thread, until the
//! session's teardown ends: its slot then passes straight to the successor, so no other
//! open can take it and never more than the limit of workers run. A successor that gives
//! up (its navigation failed, was cancelled or unwound) is withdrawn when its
//! reservation is dropped; the replaced session's release then frees its slot as usual.
use super::MAX_ACTIVE;
use std::{
    fmt,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    task::{Context, Poll, Waker},
};

/// One counted archive session, as a refusal names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSession {
    /// The session's `ArchiveMount::id`.
    pub id: u64,
    /// The native archive file that the session reads.
    pub container: PathBuf,
}

/// Why one more archive from a native file is not opened: the allowance is full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeLimit {
    /// How many such archives may be open at once.
    pub limit: usize,
    /// The counted sessions, oldest first. A slot whose session is still starting counts
    /// but is not listed.
    pub sessions: Vec<NativeSession>,
}
impl fmt::Display for NativeLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.limit {
            2 => f.write_str("Two archives are already open; close one of them first"),
            limit => write!(
                f,
                "{limit} archives are already open; close one of them first"
            ),
        }
    }
}
impl std::error::Error for NativeLimit {}

/// The slots of archives opened from native files. Production uses `global`; a test may
/// count in an allowance of its own.
pub struct NativeAllowance {
    limit: usize,
    state: Mutex<State>,
}
impl fmt::Debug for NativeAllowance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAllowance")
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct State {
    next: u64,
    /// The slots of live `NativeSlot`s, oldest first.
    slots: Vec<Counted>,
    /// Registered successors. A granted one holds a counted slot that its reservation has
    /// not taken yet.
    successors: Vec<Successor>,
}
struct Counted {
    key: u64,
    session: Option<NativeSession>,
    /// The ticket of this slot's successor, if one waits for it.
    successor: Option<u64>,
}
struct Successor {
    ticket: u64,
    granted: bool,
    waker: Option<Waker>,
}
impl State {
    fn counted(&self) -> usize {
        self.slots.len() + self.successors.iter().filter(|s| s.granted).count()
    }
    fn take_key(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
    fn push_slot(&mut self) -> u64 {
        let key = self.take_key();
        self.slots.push(Counted {
            key,
            session: None,
            successor: None,
        });
        key
    }
    /// The wakers of every waiting successor, which may now take a free slot.
    fn waiting_wakers(&mut self) -> Vec<Waker> {
        self.successors
            .iter_mut()
            .filter(|successor| !successor.granted)
            .filter_map(|successor| successor.waker.take())
            .collect()
    }
    /// Remove a successor; a granted one returns its untaken slot.
    fn withdraw(&mut self, ticket: u64) -> Vec<Waker> {
        let Some(index) = self.successors.iter().position(|s| s.ticket == ticket) else {
            return Vec::new();
        };
        let successor = self.successors.remove(index);
        if successor.granted {
            return self.waiting_wakers();
        }
        for slot in &mut self.slots {
            if slot.successor == Some(ticket) {
                slot.successor = None;
            }
        }
        Vec::new()
    }
}

static GLOBAL: LazyLock<Arc<NativeAllowance>> = LazyLock::new(|| NativeAllowance::new(MAX_ACTIVE));

impl NativeAllowance {
    /// An allowance of `limit` slots, separate from the application's (`global`).
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            state: Mutex::new(State::default()),
        })
    }
    /// The application's allowance: `MAX_ACTIVE` archives opened from native files.
    pub fn global() -> Arc<Self> {
        GLOBAL.clone()
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The counted sessions, as a refusal lists them.
    pub fn refusal(&self) -> NativeLimit {
        self.refusal_in(&self.lock())
    }
    fn refusal_in(&self, state: &State) -> NativeLimit {
        NativeLimit {
            limit: self.limit,
            sessions: state
                .slots
                .iter()
                .filter_map(|slot| slot.session.clone())
                .collect(),
        }
    }
    /// How many slots count now: live sessions, opening ones and handed-off ones.
    pub fn counted(&self) -> usize {
        self.lock().counted()
    }
    /// How many opens wait for the slot of a session that they replace.
    pub fn successors(&self) -> usize {
        let state = self.lock();
        state.successors.iter().filter(|s| !s.granted).count()
    }
    /// A free slot now, or the refusal.
    pub fn try_reserve(self: &Arc<Self>) -> Result<NativeSlot, NativeLimit> {
        let mut state = self.lock();
        if state.counted() >= self.limit {
            return Err(self.refusal_in(&state));
        }
        let key = state.push_slot();
        Ok(self.slot(key))
    }
    /// A free slot, or the slot of the session `replaces` once its teardown ends. The
    /// wait applies only while that session counts and has no other successor; anything
    /// else is refused at once. Dropping the reservation withdraws it.
    pub fn reserve(self: &Arc<Self>, replaces: Option<u64>) -> Reservation {
        Reservation {
            allowance: self.clone(),
            replaces,
            ticket: None,
        }
    }
    fn slot(self: &Arc<Self>, key: u64) -> NativeSlot {
        NativeSlot {
            allowance: self.clone(),
            key,
        }
    }
    fn bind(&self, key: u64, session: NativeSession) {
        let mut state = self.lock();
        if let Some(slot) = state.slots.iter_mut().find(|slot| slot.key == key) {
            slot.session = Some(session);
        }
    }
    /// A slot's session ended: its successor takes the slot, or the slot is free.
    fn release(&self, key: u64) {
        let wakers = {
            let mut state = self.lock();
            let Some(index) = state.slots.iter().position(|slot| slot.key == key) else {
                return;
            };
            let released = state.slots.remove(index);
            let handed = released.successor.and_then(|ticket| {
                state
                    .successors
                    .iter()
                    .position(|successor| successor.ticket == ticket && !successor.granted)
            });
            match handed {
                Some(index) => {
                    let successor = &mut state.successors[index];
                    successor.granted = true;
                    successor.waker.take().into_iter().collect()
                }
                None => state.waiting_wakers(),
            }
        };
        wake(wakers);
    }
}

fn wake(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}

/// One counted slot. Dropping it ends the slot's session in the allowance: the session's
/// successor takes the slot, or it is free.
#[must_use = "a slot is released when dropped"]
pub struct NativeSlot {
    allowance: Arc<NativeAllowance>,
    key: u64,
}
impl NativeSlot {
    /// Name the session that holds this slot: refusals list it, and a later open may
    /// replace it (`NativeAllowance::reserve`).
    pub fn bind(&self, id: u64, container: &Path) {
        self.allowance.bind(
            self.key,
            NativeSession {
                id,
                container: container.to_owned(),
            },
        );
    }
}
impl fmt::Debug for NativeSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSlot")
            .field("key", &self.key)
            .finish()
    }
}
impl Drop for NativeSlot {
    fn drop(&mut self) {
        self.allowance.release(self.key);
    }
}

/// A pending `NativeAllowance::reserve`. It never blocks: a successor is woken when the
/// replaced session's slot passes to it or when a slot is free.
#[must_use = "a reservation does nothing unless polled"]
pub struct Reservation {
    allowance: Arc<NativeAllowance>,
    replaces: Option<u64>,
    ticket: Option<u64>,
}
impl fmt::Debug for Reservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reservation")
            .field("replaces", &self.replaces)
            .field("waiting", &self.ticket.is_some())
            .finish()
    }
}
impl Future for Reservation {
    type Output = Result<NativeSlot, NativeLimit>;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let allowance = this.allowance.clone();
        let mut state = allowance.lock();
        if let Some(ticket) = this.ticket {
            let Some(index) = state.successors.iter().position(|s| s.ticket == ticket) else {
                this.ticket = None;
                return Poll::Ready(Err(allowance.refusal_in(&state)));
            };
            if state.successors[index].granted {
                state.successors.remove(index);
                this.ticket = None;
                let key = state.push_slot();
                return Poll::Ready(Ok(allowance.slot(key)));
            }
            if state.counted() < allowance.limit {
                // Another session ended first: take its free slot instead.
                let wakers = state.withdraw(ticket);
                this.ticket = None;
                let key = state.push_slot();
                drop(state);
                wake(wakers);
                return Poll::Ready(Ok(allowance.slot(key)));
            }
            state.successors[index].waker = Some(context.waker().clone());
            return Poll::Pending;
        }
        if state.counted() < allowance.limit {
            let key = state.push_slot();
            return Poll::Ready(Ok(allowance.slot(key)));
        }
        let replaced = this.replaces.and_then(|id| {
            state.slots.iter().position(|slot| {
                slot.successor.is_none() && slot.session.as_ref().is_some_and(|s| s.id == id)
            })
        });
        let Some(index) = replaced else {
            return Poll::Ready(Err(allowance.refusal_in(&state)));
        };
        let ticket = state.take_key();
        state.slots[index].successor = Some(ticket);
        state.successors.push(Successor {
            ticket,
            granted: false,
            waker: Some(context.waker().clone()),
        });
        this.ticket = Some(ticket);
        Poll::Pending
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            let wakers = self.allowance.lock().withdraw(ticket);
            wake(wakers);
        }
    }
}

#[cfg(test)]
#[path = "session_allowance_tests.rs"]
mod tests;
