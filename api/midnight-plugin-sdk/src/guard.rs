use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Mutex, MutexGuard},
};

/// Never let an unwinding panic, including its payload destructor, cross C.
/// This cannot recover from aborts, double panics during unwinding, or native UB.
pub(crate) fn contain<T>(work: impl FnOnce() -> T) -> Result<T, ()> {
    catch_unwind(AssertUnwindSafe(work)).map_err(|payload| {
        if let Err(nested) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
            // The second payload's destructor is unknown; do not recurse indefinitely.
            std::mem::forget(nested);
        }
    })
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // No user callbacks execute while SDK state locks are held.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
