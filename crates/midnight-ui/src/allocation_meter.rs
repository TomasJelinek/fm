//! Test-only allocation counting for this crate's unit tests: the test binary's global
//! allocator forwards every call unchanged to the system allocator and counts the
//! allocations a thread makes while it measures (`count`). Other test threads, which run
//! in parallel, are never counted.
#![allow(
    unsafe_code,
    reason = "a GlobalAlloc implementation is unsafe by definition"
)]
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct Meter;

#[global_allocator]
static ALLOCATOR: Meter = Meter;

thread_local! {
    /// Allocations of this thread while it measures; `None` while it does not.
    static COUNTED: Cell<Option<usize>> = const { Cell::new(None) };
}

fn counted() {
    // `try_with`: a thread being torn down still allocates and frees.
    let _ = COUNTED.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

// SAFETY: every method forwards its layout and pointer unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counter is a const-initialized thread local
// without a destructor, so touching it never allocates.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        counted();
        // SAFETY: the caller's layout, as `GlobalAlloc::alloc` requires.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        counted();
        // SAFETY: as `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` came from this allocator, that is from `System`, with `layout`.
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        counted();
        // SAFETY: as `dealloc`, with the caller's new size.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

/// Run `measured` and return its result with the allocations (and reallocations) this
/// thread made meanwhile.
pub(crate) fn count<T>(measured: impl FnOnce() -> T) -> (T, usize) {
    COUNTED.with(|count| count.set(Some(0)));
    let result = measured();
    let allocations = COUNTED.with(|count| count.replace(None)).unwrap_or(0);
    (result, allocations)
}
