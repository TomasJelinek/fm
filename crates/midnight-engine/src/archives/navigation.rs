//! Native-only archive creation/listing followed by bounded async publication.
use super::*;

/// Staging progress of an archive inside an archive reaches the pane at most this often.
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

pub(super) fn run(
    request: &Request,
    mounts: &Mounts,
    outbox: &Outbox,
) -> Result<ArchiveListing, FsError> {
    let action = request
        .action
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or_else(|| error("Archive navigation request was already consumed"))?;
    let (mut location, mount) = match action {
        ArchiveAction::Open { path, format } => {
            let reserved = mounts.reserve()?;
            let (package, format) = request
                .catalog
                .get()
                .and_then(|c| c.choose_format(format, None))
                .ok_or_else(|| {
                    FsError::from_io(
                        "open archive",
                        &path,
                        std::io::Error::new(
                            std::io::ErrorKind::Unsupported,
                            unavailable(
                                "Compatible archive filesystem plugin is unavailable",
                                request.catalog.get(),
                            ),
                        ),
                    )
                })?;
            let source = capture_source(request.capture.as_ref(), &path, None, &request.cancel)?;
            let raw = Mount::open_captured(package, format, &path, source, &request.cancel)?;
            let address = raw.address();
            let mount = mounts.adopt_reserved(raw, reserved);
            (
                ArchiveLocation {
                    address,
                    mount: None,
                    directory: 1,
                    trail: vec![(
                        1,
                        path.file_name()
                            .unwrap_or(path.as_os_str())
                            .to_string_lossy()
                            .into_owned(),
                    )],
                },
                mount,
            )
        }
        ArchiveAction::List(mut location) => {
            let mount = if let Some(raw) = location.mount.take() {
                mounts.adopt(raw)?
            } else {
                let reserved = mounts.reserve()?;
                location = restore(
                    request.catalog.get(),
                    request.capture.as_ref(),
                    location.address,
                    &request.cancel,
                )?;
                let raw = location
                    .mount
                    .take()
                    .ok_or_else(|| error("Archive is unavailable"))?;
                mounts.adopt_reserved(raw, reserved)
            };
            (location, mount)
        }
        ArchiveAction::OpenMember { parent, format } => {
            open_member(request, mounts, outbox, parent, format)?
        }
    };
    let canonical: Arc<dyn ArchiveMount> = mount.clone();
    location.mount = Some(canonical.clone());
    mount.validate()?;
    // The raw alias is considered only for this exact privately retained source,
    // preserving maps published before first adoption. Numeric IDs are insufficient.
    let sizes =
        crate::archive_sizes::published::snapshot(&request.snapshots, request.pane, &canonical)
            .or_else(|| {
                crate::archive_sizes::published::snapshot(
                    &request.snapshots,
                    request.pane,
                    mount.source(),
                )
            });
    // A superseded listing of a shared mount ends between pages without stopping
    // the worker that other tabs, trees, viewers and jobs use (archive_shared.rs).
    // An archive whose names are encrypted (7z) opens empty: its pane asks for the
    // password (`ArchiveMount::listing_locked`) and lists it again once it is unlocked.
    let nodes = match mount.list(location.directory, &request.cancel) {
        Err(error) if is_archive_password_required(&error) && mount.listing_locked() => Vec::new(),
        result => result?,
    };
    let mut entries: Vec<_> = nodes.into_iter().map(Entry::from_archive).collect();
    let hide_hidden = request.snapshots.lock().unwrap_or_else(|e| e.into_inner())
        [request.pane.index()]
    .hide_hidden;
    let hidden = crate::hidden_entries::retain_visible(&mut entries, hide_hidden);
    for entry in &mut entries {
        timestamps::localize(entry, request.locale.as_deref());
        // Members named like archives open as archives inside this archive.
        if entry.kind == EntryKind::File {
            entry.archive_extensions =
                ArchiveExtensionHints::from_path(Path::new(&entry.display_name));
        }
    }
    // Name and extension order use the same backend locale keys as native listings.
    if let Some(locale) = request.locale.as_deref() {
        prepare_sort_keys(&mut entries, locale, &request.cancel)?;
    }
    sort_entries_with_sizes(&mut entries, request.sort, |entry| {
        crate::archive_sizes::published::size(entry, sizes.as_deref())
    });
    mount.validate()?;
    let index = Arc::new(DirectoryIndex {
        hidden,
        ..DirectoryIndex::from_entries(&entries)
    });
    let listing = ArchiveListing {
        location,
        entries: entries.into(),
        index,
    };
    remember_listing(
        &request.snapshots,
        request.pane,
        request.navigation,
        &listing,
        &request.cancel,
    );
    Ok(listing)
}

/// An archive inside an archive: the enclosing session stages the member completely and
/// verified (memory under the staging budget, else a private spool file), cancelled with
/// this navigation, and a session of its own reads that staged copy as its source. The
/// staged copy no longer needs the enclosing worker; the enclosing folder stays this
/// mount's parent for going up.
fn open_member(
    request: &Request,
    mounts: &Mounts,
    outbox: &Outbox,
    parent: ArchiveParent,
    format: ArchiveFormat,
) -> Result<(ArchiveLocation, Arc<ownership::GuardedMount>), FsError> {
    let ArchiveParent {
        location: mut folder,
        member,
        name,
    } = parent;
    let mut path = folder.display_text();
    if !path.ends_with(std::path::MAIN_SEPARATOR) {
        path.push(std::path::MAIN_SEPARATOR);
    }
    path.push_str(&name);
    let path = std::path::PathBuf::from(path);
    // Errors of the enclosing session name the member, not the enclosing archive.
    let named = |mut error: FsError| {
        error.path = path.clone();
        error
    };
    let fail = |kind: std::io::ErrorKind, message: String| {
        FsError::from_io("open archive", &path, std::io::Error::new(kind, message))
    };
    let reserved = mounts.reserve()?;
    let enclosing = folder.mount.take().ok_or_else(|| {
        fail(
            std::io::ErrorKind::NotFound,
            "The archive that contains this archive is no longer open. Open it again.".into(),
        )
    })?;
    let enclosing: Arc<dyn ArchiveMount> = mounts.adopt(enclosing)?;
    folder.mount = Some(enclosing.clone());
    if member.mount != enclosing.id() {
        return Err(fail(
            std::io::ErrorKind::InvalidInput,
            "Archive member belongs to another archive session".into(),
        ));
    }
    if folder.depth() >= MAX_ARCHIVE_DEPTH {
        return Err(fail(
            std::io::ErrorKind::Unsupported,
            archive_depth_message(),
        ));
    }
    let (package, format) = request
        .catalog
        .get()
        .and_then(|catalog| catalog.choose_format(format, None))
        .ok_or_else(|| {
            fail(
                std::io::ErrorKind::Unsupported,
                unavailable(
                    "Compatible archive filesystem plugin is unavailable",
                    request.catalog.get(),
                ),
            )
        })?;
    let reported = std::cell::Cell::new(std::time::Instant::now());
    let progress = |staged: u64| {
        let now = std::time::Instant::now();
        if now.duration_since(reported.get()) >= PROGRESS_INTERVAL {
            reported.set(now);
            offer(
                outbox,
                Event::ArchiveProgress {
                    pane: request.pane,
                    navigation: request.navigation,
                    staged,
                },
            );
        }
    };
    let content = enclosing
        .stage_member(member.entry, &|| request.cancel.is_cancelled(), &progress)
        .map_err(named)?;
    drop(enclosing);
    let raw = Mount::open_nested(
        package,
        format,
        ArchiveParent {
            location: folder,
            member,
            name: name.clone(),
        },
        content,
        &request.cancel,
    )?;
    let address = raw.address();
    let mount = mounts.adopt_reserved(raw, reserved);
    Ok((
        ArchiveLocation {
            address,
            mount: None,
            directory: 1,
            trail: vec![(1, name)],
        },
        mount,
    ))
}

/// Progress is advisory: a full UI queue drops it rather than holding the staging.
fn offer(outbox: &Outbox, event: Event) {
    if outbox.sender.try_send(event).is_ok() {
        (outbox.wakeup)();
    }
}

pub(super) async fn publish(
    snapshots: &Snapshots,
    outbox: &Outbox,
    pane: PaneId,
    navigation: NavigationId,
    restore_native_on_failure: bool,
    result: Result<ArchiveListing, FsError>,
) {
    {
        let mut states = snapshots.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut states[pane.index()];
        if state.archive_navigation == Some(navigation)
            && result.is_err()
            && let Some(notify) = &state.tree_notify
        {
            notify.notify(navigation);
        }
        if state.archive_navigation == Some(navigation)
            && result.is_err()
            && restore_native_on_failure
            && state.archive.is_none()
        {
            // Failed native Open preserves its native pane; saved provider tabs
            // remain unavailable with native actions disabled.
            state.archive_navigation = None;
        }
    }
    outbox
        .send(Event::ArchiveListed {
            pane,
            navigation,
            result,
        })
        .await;
}

#[cfg(test)]
#[path = "nested_tests.rs"]
mod nested_tests;
