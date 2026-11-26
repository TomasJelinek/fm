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
            let slot = request.take_native_slot();
            let reserved = mounts.reserve()?;
            let (raw, address) = request.opener.open(request, slot, &path, format)?;
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
            if let Some(raw) = location.mount.take() {
                (location, mounts.adopt(raw)?)
            } else {
                let slot = request.take_native_slot();
                let reserved = mounts.reserve()?;
                let saved = location.address;
                let mut outermost = saved.clone();
                outermost.nested.clear();
                location =
                    restore(request, slot, outermost).map_err(|e| level_error(&saved, 0, e))?;
                let raw = location
                    .mount
                    .take()
                    .ok_or_else(|| error("Archive is unavailable"))?;
                let mount = mounts.adopt_reserved(raw, reserved);
                restore_nested(request, mounts, outbox, location, mount, &saved)?
            }
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
        crate::archive_sizes::published::sort_size(entry, sizes.as_deref())
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

/// The archives inside the archive that a saved address names (`ArchiveAddress::nested`),
/// opened from `location`, its outermost folder, each down to its saved folder. A member
/// or folder that needs a password ends the walk where it stands: that folder is listed,
/// and the pane asks for the password there and continues (`archive_resume` in the UI).
fn restore_nested(
    request: &Request,
    mounts: &Mounts,
    outbox: &Outbox,
    mut location: ArchiveLocation,
    mut mount: Arc<ownership::GuardedMount>,
    saved: &ArchiveAddress,
) -> Result<(ArchiveLocation, Arc<ownership::GuardedMount>), FsError> {
    if location.trail.len() != saved.components.len() + 1 {
        // The outermost archive's names are locked: the walk stopped at its root.
        return Ok((location, mount));
    }
    for (index, level) in saved.nested.iter().enumerate() {
        let failed = |error| level_error(saved, index + 1, error);
        let enclosing: Arc<dyn ArchiveMount> = mount.clone();
        let node = find_member(
            &*enclosing,
            location.directory,
            &level.member,
            &request.cancel,
        )
        .map_err(failed)?;
        location.mount = Some(enclosing);
        let parent = ArchiveParent {
            location: location.clone(),
            member: node.id,
            name: node.display_name,
            raw_name: level.member.clone(),
        };
        let (mut next, next_mount) =
            match open_member(request, mounts, outbox, parent, level.format) {
                Ok(opened) => opened,
                // An encrypted member: the folder that holds it is shown, and opening the
                // member there asks for its archive's password.
                Err(error) if is_archive_password_required(&error) => return Ok((location, mount)),
                Err(error) => return Err(failed(error)),
            };
        let complete =
            descend(&*next_mount, &mut next, &level.components, &request.cancel).map_err(failed)?;
        location = next;
        mount = next_mount;
        if !complete {
            break;
        }
    }
    Ok((location, mount))
}

/// The member of `directory` whose raw name is `name`, which opens as an archive.
fn find_member(
    mount: &dyn ArchiveMount,
    directory: u64,
    name: &[u8],
    cancel: &Cancellation,
) -> Result<ArchiveNode, FsError> {
    let mut matches = mount
        .list(directory, cancel)?
        .into_iter()
        .filter(|node| node.kind != EntryKind::Directory && node.raw_name.as_ref() == name);
    let node = matches.next().ok_or_else(|| {
        FsError::from_io(
            "open archive",
            Path::new(""),
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "The archive is no longer in the folder that held it",
            ),
        )
    })?;
    if matches.next().is_some() {
        return Err(error("Saved archive member locator is ambiguous"));
    }
    Ok(node)
}

/// Walk `components` down from the shown folder of `location`, a folder of `mount`.
/// Returns false when the archive's names need its password: the walk stops there.
pub(super) fn descend(
    mount: &dyn ArchiveMount,
    location: &mut ArchiveLocation,
    components: &[Vec<u8>],
    cancel: &Cancellation,
) -> Result<bool, FsError> {
    for component in components {
        let nodes = match mount.list(location.directory, cancel) {
            Err(error) if is_archive_password_required(&error) && mount.listing_locked() => {
                return Ok(false);
            }
            result => result?,
        };
        let mut matches = nodes.into_iter().filter(|node| {
            node.kind == EntryKind::Directory && node.raw_name.as_ref() == component.as_slice()
        });
        let node = matches
            .next()
            .ok_or_else(|| error("Saved archive folder is unavailable"))?;
        if matches.next().is_some() {
            return Err(error("Saved archive folder locator is ambiguous"));
        }
        location.directory = node.id.entry;
        location.trail.push((node.id.entry, node.display_name));
        location.address.components.push(component.clone());
    }
    Ok(true)
}

/// A saved location inside archives inside an archive names the level that does not open
/// again, counted from the archive opened from the native file. `level` 0 is that archive.
pub(super) fn level_error(saved: &ArchiveAddress, level: usize, mut failure: FsError) -> FsError {
    if saved.nested.is_empty() || failure.kind == std::io::ErrorKind::Interrupted {
        return failure;
    }
    let name = match level
        .checked_sub(1)
        .and_then(|index| saved.nested.get(index))
    {
        Some(nested) => String::from_utf8_lossy(&nested.member).into_owned(),
        None => saved
            .container
            .file_name()
            .unwrap_or(saved.container.as_os_str())
            .to_string_lossy()
            .into_owned(),
    };
    failure.message = format!(
        "Archive {} of {} on the saved path ({name}) cannot be opened: {}",
        level + 1,
        saved.nested.len() + 1,
        failure.message
    );
    failure
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
        raw_name,
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
            raw_name,
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
#[cfg(test)]
#[path = "restore_nested_tests.rs"]
mod restore_nested_tests;
