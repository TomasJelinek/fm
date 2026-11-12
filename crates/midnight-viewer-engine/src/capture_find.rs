//! Find launches capture only their retained result, through the installed native provider.
use crate::{NativeCapture, capture::Order};
use midnight_core::{Cancellation, EntryKind, ViewerLaunch};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub(crate) fn prepare(
    launch: &ViewerLaunch,
    closed: &Arc<AtomicBool>,
    capture: NativeCapture,
) -> Result<Order, String> {
    if launch.archive.is_some() || launch.tree.is_some() || launch.generated.is_some() {
        return Err("Conflicting viewer source identities.".into());
    }
    let source = launch.find.as_ref().ok_or("Missing Find source.")?;
    let entry = source.entry();
    if entry.kind != EntryKind::File {
        return Err("Only regular Find results can be viewed.".into());
    }
    if closed.load(Ordering::Acquire) {
        return Err("Viewer closed".into());
    }
    let cancel = Cancellation::observing(closed.clone());
    source
        .validate(&cancel)
        .map_err(|error| error.to_string())?;
    if closed.load(Ordering::Acquire) {
        return Err("Viewer closed".into());
    }
    let file = capture(&entry.path, closed)?;
    if !midnight_native_filesystem_host::matches_find_entry(entry, &file.stable_version()) {
        return Err("Find result identity or content revision changed before viewing.".into());
    }
    source
        .validate(&cancel)
        .map_err(|error| error.to_string())?;
    if closed.load(Ordering::Acquire) {
        return Err("Viewer closed".into());
    }
    let (_sender, ready) = mpsc::sync_channel(1);
    Ok(Order {
        files: vec![(entry.path.clone(), Some(Ok(file)))],
        current: 0,
        ready,
    })
}
