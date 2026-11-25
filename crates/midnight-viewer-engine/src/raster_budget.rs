//! Shared admission for retained text pixels (prepared strips), independent of worker
//! lifetimes.
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU64, Ordering},
};

/// Prepared text strips: bytes per viewer and for all viewers together.
pub(crate) const MAX_STRIP_BYTES: usize = midnight_core::VIEWER_PREPARED_TEXT_BYTES;
pub(crate) const MAX_TOTAL_STRIP_BYTES: usize = midnight_core::VIEWER_PREPARED_TEXT_TOTAL_BYTES;

/// Shared by every worker opened through a Service, including replaced/closed windows.
/// A strip's bytes stay charged until its last pixel owner (a worker's cache, a frame, a
/// window's delivery) is dropped. A strip's `Weak<[u8]>` still keeps its allocation, pixels
/// included, until [`Budget::reap`] or the next reservation forgets it: whoever drops
/// strips reaps (`Worker::drop_strips`, the worker's close).
pub(crate) struct Budget {
    strips: Mutex<Vec<Strip>>,
    next_owner: AtomicU64,
    next_ticket: AtomicU64,
}

enum Strip {
    Reserved {
        owner: u64,
        bytes: usize,
        ticket: u64,
    },
    Live {
        owner: u64,
        bytes: usize,
        body: Weak<[u8]>,
        gutter: Option<Weak<[u8]>>,
    },
}

impl Strip {
    fn charge(&self) -> (u64, usize) {
        match self {
            Self::Reserved { owner, bytes, .. } | Self::Live { owner, bytes, .. } => {
                (*owner, *bytes)
            }
        }
    }

    fn released(&self) -> bool {
        matches!(self, Self::Live { body, gutter, .. }
            if body.strong_count() == 0
                && gutter.as_ref().is_none_or(|pixels| pixels.strong_count() == 0))
    }
}

/// A strip's bytes, held until its pixels are committed or the reservation is dropped.
pub(crate) struct StripReservation {
    budget: Arc<Budget>,
    ticket: u64,
    committed: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            strips: Mutex::new(Vec::new()),
            next_owner: AtomicU64::new(0),
            next_ticket: AtomicU64::new(0),
        }
    }
}

impl Budget {
    /// A worker keeps this identity across file, layout and camera resets.
    pub(crate) fn owner(&self) -> u64 {
        self.next_owner
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |owner| {
                owner.checked_add(1)
            })
            .expect("Retained raster worker identities exhausted.")
            + 1
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Strip>> {
        self.strips
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Forget strips whose pixels every owner dropped, when retirement finishes, even with
    /// no later open.
    pub(crate) fn reap(&self) {
        self.lock().retain(|strip| !strip.released());
    }

    /// Reserve `bytes` of prepared text strips for `owner`, unless that viewer's or all
    /// viewers' strips would exceed their budget. Strips whose pixels every owner dropped
    /// no longer count. Rejects saturated capacity without waiting for an owner.
    pub(crate) fn reserve_strip(
        self: &Arc<Self>,
        owner: u64,
        bytes: usize,
    ) -> Option<StripReservation> {
        let mut strips = self.lock();
        strips.retain(|strip| !strip.released());
        let (mine, all) = strips.iter().fold((0, 0), |(mine, all), strip| {
            let (charged, size) = strip.charge();
            (mine + if charged == owner { size } else { 0 }, all + size)
        });
        if owner == 0 || mine + bytes > MAX_STRIP_BYTES || all + bytes > MAX_TOTAL_STRIP_BYTES {
            return None;
        }
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        strips.push(Strip::Reserved {
            owner,
            bytes,
            ticket,
        });
        Some(StripReservation {
            budget: self.clone(),
            ticket,
            committed: false,
        })
    }

    /// Strips the budget tracks, released or not: each keeps its pixels' allocation.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> usize {
        self.lock().len()
    }

    /// Bytes of prepared text strips `owner`'s pixels still hold.
    #[cfg(test)]
    pub(crate) fn strip_bytes(&self, owner: u64) -> usize {
        let mut strips = self.lock();
        strips.retain(|strip| !strip.released());
        strips
            .iter()
            .filter(|strip| strip.charge().0 == owner)
            .map(|strip| strip.charge().1)
            .sum()
    }
}

impl StripReservation {
    /// Charge the reserved bytes to these pixels until both are dropped. Callers size the
    /// strip before allocating it; this catches accounting mistakes.
    pub(crate) fn commit(
        mut self,
        body: &Arc<[u8]>,
        gutter: Option<&Arc<[u8]>>,
    ) -> Result<(), &'static str> {
        let mut strips = self.budget.lock();
        let index =
            Self::find(&strips, self.ticket).ok_or("Prepared text strip reservation lost.")?;
        let (owner, bytes) = strips[index].charge();
        if body.len() + gutter.map_or(0, |pixels| pixels.len()) > bytes {
            return Err("Prepared text strip exceeds its reservation.");
        }
        strips[index] = Strip::Live {
            owner,
            bytes,
            body: Arc::downgrade(body),
            gutter: gutter.map(Arc::downgrade),
        };
        self.committed = true;
        Ok(())
    }

    /// Entries move when released ones are removed: find this reservation by its ticket.
    fn find(strips: &[Strip], ticket: u64) -> Option<usize> {
        strips.iter().position(
            |strip| matches!(strip, Strip::Reserved { ticket: held, .. } if *held == ticket),
        )
    }
}

impl Drop for StripReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut strips = self.budget.lock();
        if let Some(index) = Self::find(&strips, self.ticket) {
            strips.remove(index);
        }
    }
}

#[cfg(test)]
#[path = "raster_budget_tests.rs"]
mod tests;
