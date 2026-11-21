//! Native roots keep independent metadata pins until their first active use.
use super::*;

struct Active {
    source: Box<dyn TransferSource>,
    capture: Option<Arc<dyn TransferMoveCapture>>,
}

struct State {
    // All native owners precede the service registration that implements them.
    active: Mutex<Option<Active>>,
    pin: Arc<dyn TransferNamespacePin>,
    entry: TransferEntry,
    original_identity: Option<TransferIdentity>,
    authority: Authority,
    retry: bool,
    service: Arc<dyn TransferService>,
}

pub(super) fn defer(
    service: Arc<dyn TransferService>,
    authority: Authority,
    source: Box<dyn TransferSource>,
    capture: Option<Arc<dyn TransferMoveCapture>>,
    cancel: &Cancellation,
) -> Result<PinnedSource, FsError> {
    source.revalidate(cancel)?;
    if let Some(capture) = &capture {
        capture.revalidate(cancel)?;
        if !same_entry(source.entry(), capture.entry()) {
            return Err(changed(&source.entry().item.path));
        }
    }
    let pin = source.native_namespace_scope(cancel)?;
    let qualified = match &pin {
        Some(pin)
            if source.native_identity() == Some(source.entry().identity)
                && source.native_original_identity() == Some(source.entry().identity) =>
        {
            matches!(pin.refresh(cancel)?, TransferNamespaceProbe::Known(scopes)
                if !scopes.is_empty() && scopes.iter().any(|scope| scope.object == Some(source.entry().identity)))
        }
        _ => false,
    };
    if !qualified {
        // Geometry alone is not a reopening capability. Keep an unqualified
        // provider's exact ready owner; that route receives opaque exclusion.
        return Ok(PinnedSource {
            source: SharedSource::new(source),
            capture,
            link_capture: None,
        });
    }
    let entry = source.entry().clone();
    let original_identity = source.native_original_identity();
    let retry = source.supports_read_retry();
    let moved = capture.is_some();
    drop(source);
    drop(capture);
    let state = Arc::new(State {
        active: Mutex::new(None),
        pin: pin.expect("qualified pin"),
        entry,
        original_identity,
        authority,
        retry,
        service,
    });
    Ok(PinnedSource {
        source: SharedSource::new(Box::new(Source(state.clone()))),
        capture: moved.then(|| Arc::new(Capture(state)) as Arc<dyn TransferMoveCapture>),
        link_capture: None,
    })
}

fn changed(path: &Path) -> FsError {
    error(
        path,
        io::ErrorKind::InvalidData,
        "Planned native source changed its original binding, identity or revision.",
    )
}
impl State {
    fn lock(&self) -> Result<MutexGuard<'_, Option<Active>>, FsError> {
        self.active
            .lock()
            .map_err(|_| changed(&self.entry.item.path))
    }
    fn validate_pin(&self, cancel: &Cancellation) -> Result<(), FsError> {
        check_cancel(&self.entry.item.path, cancel)?;
        match self.pin.refresh(cancel)? {
            TransferNamespaceProbe::Known(scopes)
                if !scopes.is_empty()
                    && scopes
                        .iter()
                        .any(|scope| scope.object == Some(self.entry.identity)) =>
            {
                Ok(())
            }
            _ => Err(changed(&self.entry.item.path)),
        }
    }
    fn open(&self, cancel: &Cancellation) -> Result<Active, FsError> {
        self.validate_pin(cancel)?;
        // The raw path is only a lookup hint. Both the retained original pin
        // and the new provider's binding/revision checks must still agree.
        let active = match self.authority {
            Authority::Copy(links) => Active {
                source: self.service.prepare_transfer_source_with_policy(
                    &self.entry.item.path,
                    links,
                    TRANSFER_REVISIONS,
                    cancel,
                )?,
                capture: None,
            },
            Authority::Move => {
                let prepared = prepare_ordinary_move_source(
                    self.service.as_ref(),
                    &self.entry.item.path,
                    cancel,
                )?;
                Active {
                    source: prepared.source,
                    capture: Some(prepared.capture),
                }
            }
            Authority::MoveWithLinks(links) => {
                let prepared = self.service.prepare_move_source_with_policy(
                    &self.entry.item.path,
                    links,
                    TRANSFER_REVISIONS,
                    cancel,
                )?;
                let PreparedTransferMoveEntry::Ordinary(prepared) = prepared else {
                    return Err(changed(&self.entry.item.path));
                };
                Active {
                    source: prepared.source,
                    capture: Some(prepared.capture),
                }
            }
        };
        active.source.revalidate(cancel)?;
        if active.source.native_identity() != Some(self.entry.identity)
            || active.source.native_original_identity() != self.original_identity
            || !same_entry(active.source.entry(), &self.entry)
        {
            return Err(changed(&self.entry.item.path));
        }
        if let Some(capture) = &active.capture {
            capture.revalidate(cancel)?;
            if !same_entry(capture.entry(), &self.entry) {
                return Err(changed(&self.entry.item.path));
            }
        }
        self.validate_pin(cancel)?;
        Ok(active)
    }
    fn with_active<T>(
        &self,
        cancel: &Cancellation,
        operation: impl FnOnce(&mut Active) -> Result<T, FsError>,
    ) -> Result<T, FsError> {
        let mut state = self.lock()?;
        if state.is_none() {
            *state = Some(self.open(cancel)?);
        }
        operation(state.as_mut().expect("active native source"))
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let state = self.lock()?;
        if let Some(active) = state.as_ref() {
            active.source.revalidate(cancel)?;
            Ok(())
        } else {
            self.validate_pin(cancel)
        }
    }
    fn capture<T>(
        &self,
        cancel: &Cancellation,
        operation: impl FnOnce(&dyn TransferMoveCapture) -> Result<T, FsError>,
    ) -> Result<T, FsError> {
        self.with_active(cancel, |active| {
            operation(
                active
                    .capture
                    .as_deref()
                    .ok_or_else(|| changed(&self.entry.item.path))?,
            )
        })
    }
}

struct Source(Arc<State>);
impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.0.entry
    }
    fn native_original_identity(&self) -> Option<TransferIdentity> {
        self.0.original_identity
    }
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        check_cancel(&self.0.entry.item.path, cancel)?;
        Ok(Some(self.0.pin.clone()))
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        let state = self.0.lock()?;
        if let Some(active) = state.as_ref() {
            return active.source.discover(cancel, visit);
        }
        // Discovery owns an ephemeral validated pair, never installs it in active.
        let ephemeral = self.0.open(cancel)?;
        ephemeral.source.discover(cancel, visit)
    }
    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError> {
        let state = self.0.lock()?;
        if let Some(active) = state.as_ref() {
            return active.source.link_representation(cancel);
        }
        self.0.open(cancel)?.source.link_representation(cancel)
    }
    fn link_not_followed(&self) -> Option<String> {
        let state = self.0.lock().ok()?;
        if let Some(active) = state.as_ref() {
            return active.source.link_not_followed();
        }
        self.0
            .open(&Cancellation::default())
            .ok()?
            .source
            .link_not_followed()
    }
    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0
            .with_active(cancel, |active| active.source.open_child(name, cancel))
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.0
            .with_active(cancel, |active| active.source.next_child(cancel))
    }
    fn next_child_or_failure(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<TransferChild>, FsError> {
        self.0
            .with_active(cancel, |active| active.source.next_child_or_failure(cancel))
    }
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        self.0
            .with_active(cancel, |active| active.source.next_move_child(cancel))
    }
    fn next_move_child_with_links(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMoveEntry>, FsError> {
        self.0.with_active(cancel, |active| {
            active.source.next_move_child_with_links(links, cancel)
        })
    }
    fn next_move_child_or_failure(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<TransferMoveChild>, FsError> {
        self.0.with_active(cancel, |active| {
            active.source.next_move_child_or_failure(links, cancel)
        })
    }
    fn prepare_move_child(
        &self,
        name: &OsStr,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMoveEntry, FsError> {
        self.0.with_active(cancel, |active| {
            active.source.prepare_move_child(name, links, cancel)
        })
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.0
            .with_active(cancel, |active| active.source.prepare_modified_time(cancel))
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.0
            .with_active(cancel, |active| active.source.rewind_for_validation(cancel))
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.0.with_active(cancel, |active| {
            active.source.seek_for_validation(offset, cancel)
        })
    }
    fn open_for_reading(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0
            .with_active(cancel, |active| active.source.open_for_reading(cancel))
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.0
            .with_active(cancel, |active| active.source.read_chunk(buffer, cancel))
    }
    fn supports_read_retry(&self) -> bool {
        self.0.retry
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.with_active(cancel, |active| {
            active.source.retry_read_from(offset, cancel)
        })
    }
}

struct Capture(Arc<State>);
impl TransferMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        &self.0.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let state = self.0.lock()?;
        match state.as_ref().and_then(|active| active.capture.as_ref()) {
            Some(capture) => capture.revalidate(cancel),
            None => self.0.validate_pin(cancel),
        }
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        // This is per-active-entry execution, not root planning. Native providers
        // can retain a writer-exclusion lease here through copy and finalization.
        self.0
            .capture(cancel, |capture| capture.preflight_finalization(cancel))
    }
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        self.0.capture(cancel, |capture| {
            capture.try_native_move(destination, name, replace, cancel)
        })
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        self.0
            .capture(cancel, |capture| capture.remove_empty_directory(cancel))
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        self.0
            .capture(cancel, |capture| capture.prepare_finalization(cancel))
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.0.capture(cancel, |capture| capture.capture(cancel))
    }
    fn restore(&self) -> Result<(), FsError> {
        let state = self.0.lock()?;
        match state.as_ref().and_then(|active| active.capture.as_ref()) {
            Some(capture) => capture.restore(),
            None => Ok(()),
        }
    }
    fn restoration_completed(&self) -> bool {
        self.0.lock().is_ok_and(|state| {
            state
                .as_ref()
                .and_then(|active| active.capture.as_ref())
                .is_some_and(|capture| capture.restoration_completed())
        })
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        self.0
            .lock()
            .ok()?
            .as_ref()?
            .capture
            .as_ref()?
            .take_stage_cleanup()
    }
}
