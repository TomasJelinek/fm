//! Retained capacity, including nested Vec/String element arrays. Wire lengths
//! alone do not bound the three copies of a short-key, deeply nested identity.
//!
//! The 16 MiB charge bounds common catalog retained payload and container
//! capacities, with conservative bookkeeping overhead. It is not a total-plugin
//! memory bound: transient encoding/decoding, separately bounded SDK reply
//! buffers, adapter-native allocations, and OS/system-library allocators are
//! outside this counter. Removal batches shrink maps before their charges retire.
use super::{Adapter, Failure, Location, state::*};
use crate::adapter::{Record, Target};

pub(super) fn record_bytes(record: &Record) -> Result<usize, Failure> {
    record
        .target
        .key
        .capacity()
        .checked_add(record.display.capacity())
        .ok_or_else(limit)
}

fn location_bytes(location: &Location) -> Result<usize, Failure> {
    let arrays = location
        .trail
        .capacity()
        .checked_mul(size_of::<Record>())
        .ok_or_else(limit)?;
    location.trail.iter().try_fold(arrays, |bytes, record| {
        bytes.checked_add(record_bytes(record)?).ok_or_else(limit)
    })
}

fn identity_bytes(identity: &Identity) -> Result<usize, Failure> {
    let arrays = identity
        .capacity()
        .checked_mul(size_of::<Target>())
        .ok_or_else(limit)?;
    identity.iter().try_fold(arrays, |bytes, target| {
        bytes.checked_add(target.key.capacity()).ok_or_else(limit)
    })
}

pub(super) fn node_bytes(
    location: &Location,
    map_key: &Identity,
    node_key: &Identity,
) -> Result<usize, Failure> {
    // Measure both actual copies, including any spare capacity in nested vectors.
    let location = location_bytes(location)?;
    let node_key = identity_bytes(node_key)?;
    identity_bytes(map_key)?
        .checked_add(node_key)
        .and_then(|bytes| bytes.checked_add(512))
        .and_then(|bytes| bytes.checked_add(location))
        .ok_or_else(limit)
}

pub(super) fn directory_bytes(identity: &Identity) -> Result<usize, Failure> {
    identity_bytes(identity)?.checked_add(512).ok_or_else(limit)
}

pub(super) fn cursor_bytes<A: Adapter>() -> Result<usize, Failure> {
    // Include the complete inline cursor, whose Source size depends on A::Cursor,
    // plus Arc/map allocation and small retained collection/snapshot overhead.
    size_of::<Cursor<A>>().checked_add(512).ok_or_else(limit)
}
