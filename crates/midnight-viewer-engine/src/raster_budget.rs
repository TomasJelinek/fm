//! Shared admission for retained CPU raster allocations, independent of worker lifetimes.
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU64, Ordering},
};

pub(crate) const MAX_SNAPSHOTS: usize = 12;
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 6 * 1024 * 1024 * 4;
const MAX_PER_OWNER: usize = 3;

/// Shared by every worker opened through a Service, including replaced/closed windows.
/// This is the only weak-pixel registry: Weak<[u8]> itself retains the slice allocation.
pub(crate) struct Budget {
    slots: Mutex<[Slot; MAX_SNAPSHOTS]>,
    next_owner: AtomicU64,
}

enum Slot {
    Vacant,
    Reserved(u64),
    Live {
        owner: u64,
        body: Weak<[u8]>,
        gutter: Option<Weak<[u8]>>,
    },
}

impl Slot {
    fn owner(&self) -> Option<u64> {
        match self {
            Self::Vacant => None,
            Self::Reserved(owner) | Self::Live { owner, .. } => Some(*owner),
        }
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            slots: Mutex::new(std::array::from_fn(|_| Slot::Vacant)),
            next_owner: AtomicU64::new(0),
        }
    }
}

pub(crate) struct Reservation {
    budget: Arc<Budget>,
    index: usize,
    owner: u64,
    committed: bool,
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

    /// Reject saturated capacity without waiting for a pixel owner to release it.
    pub(crate) fn reserve_for(self: &Arc<Self>, owner: u64) -> Option<Reservation> {
        (owner != 0).then(|| self.admit(owner)).flatten()
    }

    #[cfg(test)]
    fn reserve(self: &Arc<Self>) -> Option<Reservation> {
        self.admit(0)
    }

    /// Release expired Arc slice allocations when retirement finishes, even with no later open.
    pub(crate) fn reap(&self) {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        Self::prune(&mut slots);
    }

    fn prune(slots: &mut [Slot; MAX_SNAPSHOTS]) {
        for slot in slots.iter_mut() {
            if let Slot::Live { body, gutter, .. } = slot
                && body.strong_count() == 0
                && gutter
                    .as_ref()
                    .is_none_or(|pixels| pixels.strong_count() == 0)
            {
                // Drop both weak references before making the allocation slot reusable.
                *slot = Slot::Vacant;
            }
        }
    }

    fn admit(self: &Arc<Self>, owner: u64) -> Option<Reservation> {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        Self::prune(&mut slots);
        if owner != 0
            && slots
                .iter()
                .filter(|slot| slot.owner() == Some(owner))
                .count()
                >= MAX_PER_OWNER
        {
            return None;
        }
        let index = slots.iter().position(|slot| matches!(slot, Slot::Vacant))?;
        slots[index] = Slot::Reserved(owner);
        Some(Reservation {
            budget: self.clone(),
            index,
            owner,
            committed: false,
        })
    }
}

impl Reservation {
    /// Retain the slot until both independently cloned pixel allocations are reclaimed.
    /// Callers also size-check geometry before allocation; this catches accounting mistakes.
    pub(crate) fn commit(
        mut self,
        body: &Arc<[u8]>,
        gutter: Option<&Arc<[u8]>>,
    ) -> Result<(), &'static str> {
        body.len()
            .checked_add(gutter.map_or(0, |pixels| pixels.len()))
            .filter(|bytes| *bytes <= MAX_SNAPSHOT_BYTES)
            .ok_or("Retained body and gutter exceed the raster snapshot budget.")?;
        let mut slots = self
            .budget
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        debug_assert!(matches!(slots[self.index], Slot::Reserved(owner) if owner == self.owner));
        slots[self.index] = Slot::Live {
            owner: self.owner,
            body: Arc::downgrade(body),
            gutter: gutter.map(Arc::downgrade),
        };
        self.committed = true;
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut slots = self
            .budget
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        debug_assert!(matches!(slots[self.index], Slot::Reserved(owner) if owner == self.owner));
        slots[self.index] = Slot::Vacant;
    }
}

#[cfg(test)]
#[path = "raster_budget_tests.rs"]
mod tests;
