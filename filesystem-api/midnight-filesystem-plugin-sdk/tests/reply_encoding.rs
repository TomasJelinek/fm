//! Reply encoding cost and the exact retained-capacity contract that reply budgets charge.
// Test-only counting allocator: every request is forwarded unchanged to `System`.
#![allow(unsafe_code)]

use midnight_filesystem_plugin_sdk::api::{
    self, Address, AddressEncoding, Compression, DEFAULT_REPLY_BYTES, Kind, MAX_BATCH, Metadata,
    NameEncoding, Node, Response, Timestamp, WireError,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
    time::Instant,
};

thread_local! {
    // Per-thread so concurrently running tests cannot disturb a measurement.
    static ALLOCATOR_CALLS: Cell<usize> = const { Cell::new(0) };
}
fn record_allocator_call() {
    let _ = ALLOCATOR_CALLS.try_with(|calls| calls.set(calls.get() + 1));
}

struct Counting;
// SAFETY: every method forwards its caller's unchanged arguments to the system allocator,
// which upholds the GlobalAlloc contract. The const-initialized, destructor-free
// thread-local counter neither allocates nor reenters this allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocator_call();
        // SAFETY: forwarded caller contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocator_call();
        // SAFETY: forwarded caller contract.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocator_call();
        // SAFETY: forwarded caller contract.
        unsafe { System.realloc(pointer, layout, size) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: forwarded caller contract.
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocator_calls<T>(action: impl FnOnce() -> T) -> (T, usize) {
    let before = ALLOCATOR_CALLS.with(Cell::get);
    let value = action();
    (value, ALLOCATOR_CALLS.with(Cell::get) - before)
}

fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// A full listing page whose entries carry absolute paths of `path_units` UTF-16 units.
fn page(path_units: usize) -> Response {
    let directory: String = "C:\\Users\\Example\\Documents\\project\\"
        .chars()
        .cycle()
        .take(path_units.saturating_sub(16))
        .collect();
    let entries = (1..=u64::from(MAX_BATCH))
        .map(|id| {
            let name = format!("entry {id:05}.txt");
            Node {
                id,
                address: Address {
                    encoding: AddressEncoding::WindowsUtf16Le,
                    bytes: wide(&format!("{directory}{name}")),
                },
                name_encoding: NameEncoding::WindowsUtf16Le,
                raw_name: wide(&name),
                display: name,
                kind: Kind::File,
                metadata: Metadata {
                    size: Some(id * 4096),
                    modified: Some(Timestamp {
                        seconds: 1_790_000_000,
                        nanos: 5,
                    }),
                    native_attributes: Some(32),
                    unix_mode: None,
                    compression: Compression::Uncompressed,
                },
            }
        })
        .collect();
    Response::Page {
        cursor: 1,
        page: 1,
        done: false,
        entries,
    }
}

fn content(length: usize) -> Response {
    Response::Content {
        bytes: vec![7; length],
        eof: false,
    }
}

#[test]
fn page_replies_use_one_exact_allocation_instead_of_one_per_field() {
    for path_units in [64, 2048] {
        let response = page(path_units);
        let (bytes, calls) = allocator_calls(|| {
            api::encode_response(&response, DEFAULT_REPLY_BYTES as usize).unwrap()
        });
        // About 2,200 field writes; the encoder measures the frame before allocating.
        assert_eq!(calls, 1, "allocator calls for {} bytes", bytes.len());
        assert_eq!(bytes.capacity(), bytes.len());
        assert_eq!(
            api::decode_response(&bytes, DEFAULT_REPLY_BYTES as usize).unwrap(),
            response
        );
    }
}

#[test]
fn encoded_replies_retain_exactly_their_length_up_to_the_budget() {
    // Content replies encode 19 framing bytes around the data.
    for (maximum, length) in [(4096, 4077), (4096, 2100), (4096, 0), (1 << 20, 256 * 1024)] {
        let response = Response::Content {
            bytes: vec![7; length],
            eof: length == 0,
        };
        let bytes = api::encode_response(&response, maximum).unwrap();
        assert_eq!(bytes.len(), length + 19);
        assert_eq!(bytes.capacity(), bytes.len(), "{maximum}/{length}");
    }
    assert_eq!(
        api::encode_response(&content(4078), 4096),
        Err(WireError::Limit)
    );
}

/// Before/after measurement for reply encoding; compare runs of the same build profile.
#[test]
#[ignore = "timing harness: cargo test --release --test reply_encoding -- --ignored --nocapture"]
fn measure_reply_encoding() {
    for (label, response) in [
        ("128-node page, 64-unit paths", page(64)),
        ("128-node page, 2048-unit paths", page(2048)),
        ("256 KiB content", content(256 * 1024)),
    ] {
        let maximum = DEFAULT_REPLY_BYTES as usize;
        let (bytes, calls) = allocator_calls(|| api::encode_response(&response, maximum).unwrap());
        // The fastest of several batches filters out unrelated machine load.
        let iterations = 500;
        let micros = (0..7)
            .map(|_| {
                let start = Instant::now();
                for _ in 0..iterations {
                    black_box(api::encode_response(black_box(&response), maximum).unwrap());
                }
                start.elapsed().as_secs_f64() * 1e6 / f64::from(iterations)
            })
            .fold(f64::INFINITY, f64::min);
        println!(
            "{label}: {} bytes, {calls} allocator calls, {micros:.2} us per encode (best batch)",
            bytes.len()
        );
    }
}
