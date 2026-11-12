//! Native-only archive creation/listing followed by bounded async publication.
use super::*;

pub(super) fn run(request: &Request, mounts: &Mounts) -> Result<ArchiveListing, FsError> {
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
                            "Compatible archive filesystem plugin is unavailable",
                        ),
                    )
                })?;
            let source = capture_source(request.capture.as_ref(), &path, None, &request.cancel)?;
            let raw = Mount::open_captured(package, format, &path, source, &request.cancel)?;
            let address = raw.address();
            let mount = mounts.adopt_reserved(raw, reserved, request.scope.clone());
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
                mounts.adopt(raw, request.scope.clone())?
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
                mounts.adopt_reserved(raw, reserved, request.scope.clone())
            };
            (location, mount)
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
    let nodes = mount.list(location.directory, &request.cancel)?;
    let mut entries: Vec<_> = nodes.into_iter().map(Entry::from_archive).collect();
    for entry in &mut entries {
        timestamps::localize(entry, request.locale.as_deref());
    }
    sort_entries_with_sizes(&mut entries, request.sort, |entry| {
        crate::archive_sizes::published::size(entry, sizes.as_deref())
    });
    mount.validate()?;
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
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
