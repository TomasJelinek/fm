//! Archives that a navigation opens from native files: the installed package that opens
//! each one, and its session's slot in the allowance of such archives
//! (`NativeAllowance`), which the session that the tab let go of may hand over.
use super::*;
use midnight_filesystem_host::{Format, NativeLimit, Package};

/// An archive that a navigation opens from a native file.
pub(super) enum Target {
    /// Enter on an archive file in a native folder.
    Open {
        path: std::path::PathBuf,
        format: ArchiveFormat,
    },
    /// A saved address without a session: a restored tab, a Hot Path, Go to file or the
    /// recovery of a failed session.
    Saved(ArchiveAddress),
}
impl Target {
    pub(super) fn of(action: &ArchiveAction) -> Option<Self> {
        match action {
            ArchiveAction::Open { path, format } => Some(Self::Open {
                path: path.clone(),
                format: *format,
            }),
            ArchiveAction::List(location) if location.mount.is_none() => {
                Some(Self::Saved(location.address.clone()))
            }
            ArchiveAction::List(_) | ArchiveAction::OpenMember { .. } => None,
        }
    }
    fn path(&self) -> &Path {
        match self {
            Self::Open { path, .. } => path,
            Self::Saved(address) => &address.container,
        }
    }
    /// The navigation that opens this archive again.
    fn action(&self) -> ArchiveAction {
        match self {
            Self::Open { path, format } => ArchiveAction::Open {
                path: path.clone(),
                format: *format,
            },
            Self::Saved(address) => {
                ArchiveAction::List(ArchiveLocation::unavailable(address.clone()))
            }
        }
    }
    fn choose<'c>(&self, catalog: Option<&'c Catalog>) -> Option<(&'c Package, Format)> {
        match self {
            Self::Open { format, .. } => choose_new(catalog, *format),
            Self::Saved(address) => choose_saved(catalog, address),
        }
    }
}

/// The installed package and decoder for Enter on an archive file.
fn choose_new(catalog: Option<&Catalog>, format: ArchiveFormat) -> Option<(&Package, Format)> {
    catalog?.choose_format(format, None)
}

/// The installed package and decoder for a saved address: its own provider only.
fn choose_saved<'c>(
    catalog: Option<&'c Catalog>,
    address: &ArchiveAddress,
) -> Option<(&'c Package, Format)> {
    let catalog = catalog?;
    match address.format {
        Some(format) => catalog.choose_format(format, Some(&address.provider)),
        None => catalog.choose_provider(&address.container, Some(&address.provider)),
    }
}

/// How a navigation opens an archive from a native file.
#[derive(Clone)]
pub(super) enum Opener {
    /// The installed filesystem package captures and opens it (`Mount::open_captured_in`).
    Installed,
    /// Tests substitute the session of the archive at a path.
    #[cfg(test)]
    Test(TestOpen),
}
#[cfg(test)]
pub(super) type TestOpen =
    Arc<dyn Fn(Option<NativeSlot>, &Path) -> Result<Arc<dyn ArchiveMount>, FsError> + Send + Sync>;

impl Opener {
    /// Whether an installed package opens `target`. When none does, the navigation fails
    /// with the reason before it takes a slot.
    fn supports(&self, catalog: Option<&Catalog>, target: &Target) -> bool {
        match self {
            Self::Installed => target.choose(catalog).is_some(),
            #[cfg(test)]
            Self::Test(_) => true,
        }
    }
    /// Enter on an archive file: its new session, in `slot` when one was reserved.
    pub(super) fn open(
        &self,
        request: &Request,
        slot: Option<NativeSlot>,
        path: &Path,
        format: ArchiveFormat,
    ) -> Result<(Arc<dyn ArchiveMount>, ArchiveAddress), FsError> {
        match self {
            Self::Installed => {
                let (package, format) =
                    choose_new(request.catalog.get(), format).ok_or_else(|| {
                        FsError::from_io(
                            "open archive",
                            path,
                            std::io::Error::new(
                                std::io::ErrorKind::Unsupported,
                                unavailable(
                                    "Compatible archive filesystem plugin is unavailable",
                                    request.catalog.get(),
                                ),
                            ),
                        )
                    })?;
                let source = capture_source(request.capture.as_ref(), path, None, &request.cancel)?;
                let mount = open_mount(slot, package, format, path, source, &request.cancel)?;
                let address = mount.address();
                Ok((mount, address))
            }
            #[cfg(test)]
            Self::Test(open) => {
                let mount = open(slot, path)?;
                let address = ArchiveAddress {
                    provider: "test.native".into(),
                    format: Some(format),
                    container: path.to_owned(),
                    source_version: vec![1],
                    components: Vec::new(),
                };
                Ok((mount, address))
            }
        }
    }
    /// A saved address: its new session, in `slot` when one was reserved, and the source
    /// revision it opened.
    pub(super) fn restore(
        &self,
        request: &Request,
        slot: Option<NativeSlot>,
        address: &ArchiveAddress,
    ) -> Result<(Arc<dyn ArchiveMount>, Vec<u8>), FsError> {
        match self {
            Self::Installed => {
                let catalog = request.catalog.get();
                let (package, format) = choose_saved(catalog, address).ok_or_else(|| {
                    error(unavailable(
                        "Saved archive filesystem plugin is unavailable; the tab address is retained",
                        catalog,
                    ))
                })?;
                let source = capture_source(
                    request.capture.as_ref(),
                    &address.container,
                    Some(&address.source_version),
                    &request.cancel,
                )?;
                let mount = open_mount(
                    slot,
                    package,
                    format,
                    &address.container,
                    source,
                    &request.cancel,
                )?;
                let version = mount.address().source_version;
                Ok((mount, version))
            }
            #[cfg(test)]
            Self::Test(open) => Ok((
                open(slot, &address.container)?,
                address.source_version.clone(),
            )),
        }
    }
}

fn open_mount(
    slot: Option<NativeSlot>,
    package: &Package,
    format: Format,
    path: &Path,
    source: midnight_plugin_host::CapturedFile,
    cancel: &Cancellation,
) -> Result<Arc<Mount>, FsError> {
    match slot {
        Some(slot) => Mount::open_captured_in(slot, package, format, path, source, cancel),
        None => Mount::open_captured(package, format, path, source, cancel),
    }
}

/// Why a navigation did not get its slot: its error, and when every slot counts, the
/// counted sessions and the navigation that opens the archive again.
pub(super) struct Refusal {
    pub(super) error: FsError,
    pub(super) full: Option<Box<(Vec<ArchiveSession>, ArchiveAction)>>,
}
impl Refusal {
    fn failed(error: FsError) -> Self {
        Self { error, full: None }
    }
}

/// Before a navigation opens an archive from a native file, the pane lets go of the root
/// of its last archive listing, which holds that archive's session, and the new session
/// gets a slot. A free slot is taken at once. When every slot counts and `replaces`
/// names a session that its holders let go of, the navigation waits, without blocking a
/// thread, until that session's teardown ends and its slot passes to it. The wait ends
/// with the navigation's cancellation and after `HANDOFF_DEADLINE`, and dropping it
/// withdraws it. Anything else is refused with the sessions that count.
pub(super) async fn slot(
    request: &Request,
    target: &Target,
    replaces: Option<u64>,
    allowance: &Arc<NativeAllowance>,
    superseded: &Notify,
    scope: &TaskLifetime,
) -> Result<(), Refusal> {
    let root = {
        let mut states = request.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut states[request.pane.index()];
        (state.archive_navigation == Some(request.navigation) && !request.cancel.is_cancelled())
            .then(|| state.take_archive_tree_root())
            .flatten()
    };
    if let Some(root) = root {
        // Its rows and session are destroyed off the executor.
        let _ = tokio::task::spawn_blocking(scope.blocking(move || drop(root))).await;
    }
    if !request.opener.supports(request.catalog.get(), target) {
        return Ok(());
    }
    let refused = |limit: NativeLimit| {
        let sessions = limit
            .sessions
            .iter()
            .map(|session| ArchiveSession {
                id: session.id,
                container: session.container.clone(),
            })
            .collect();
        Refusal {
            error: FsError::from_io("open archive", target.path(), std::io::Error::other(limit)),
            full: Some(Box::new((sessions, target.action()))),
        }
    };
    let reservation = allowance.reserve(replaces);
    tokio::pin!(reservation);
    let deadline = tokio::time::sleep(HANDOFF_DEADLINE);
    tokio::pin!(deadline);
    loop {
        // Registered before the check, so that a cancellation cannot be missed.
        let changed = superseded.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if request.cancel.is_cancelled() {
            return Err(Refusal::failed(FsError::from_io(
                "open archive",
                target.path(),
                std::io::ErrorKind::Interrupted.into(),
            )));
        }
        tokio::select! {
            reserved = &mut reservation => {
                let slot = reserved.map_err(refused)?;
                *request.native.lock().unwrap_or_else(|e| e.into_inner()) = Some(slot);
                return Ok(());
            }
            () = &mut deadline => return Err(refused(allowance.refusal())),
            () = &mut changed => {}
        }
    }
}
