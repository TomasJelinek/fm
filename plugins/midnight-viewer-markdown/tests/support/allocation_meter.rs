#![allow(unsafe_code)]
//! Process-global heap meter for one test binary. Every test of a metered
//! binary must hold [`exclusive`] for its whole body, so parallel test threads
//! cannot add their allocations to another test's peak.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static EXCLUSIVE: Mutex<()> = Mutex::new(());
#[global_allocator]
static ALLOCATOR: Meter = Meter;
fn added(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}
// Test-binary accounting forwards every layout and pointer unchanged to System.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            added(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe {
            System.dealloc(pointer, layout);
        }
    }
    unsafe fn realloc(&self, pointer: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, old, size) };
        if !next.is_null() {
            // A moving reallocation briefly holds both blocks: count the new
            // one before releasing the old one so that peak is observed.
            added(size);
            LIVE.fetch_sub(old.size(), Ordering::Relaxed);
        }
        next
    }
}
/// Serialize the tests of one metered binary.
pub fn exclusive() -> MutexGuard<'static, ()> {
    EXCLUSIVE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}
pub fn start() -> usize {
    let live = LIVE.load(Ordering::Relaxed);
    PEAK.store(live, Ordering::Relaxed);
    live
}
pub fn peak_since(start: usize) -> usize {
    PEAK.load(Ordering::Relaxed).saturating_sub(start)
}
