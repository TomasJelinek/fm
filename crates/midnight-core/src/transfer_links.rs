//! App-private link Copy contracts; link representations never grant target access.

use crate::{
    Cancellation, CapturedTransferRequest, FsError, MetadataTransferRequest, TransferIdentity,
    TransferPublication, TransferRecovery,
};
use std::{ffi::OsStr, fmt, path::Path};

/// Immutable per-Copy choice. Legacy requests and each new draft reject links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CopyLinkPolicy {
    #[default]
    Reject,
    Preserve,
}

/// Reuses the existing intent, verification and required-metadata inputs.
#[derive(Clone, Debug)]
pub enum CopyOptionsInput {
    Native(MetadataTransferRequest),
    Captured(CapturedTransferRequest),
}

/// One explicit options request; Preserve with Move or required modified time
/// must be refused before preparation or writes. Admission freezes this policy.
#[derive(Clone, Debug)]
pub struct CopyOptionsRequest {
    pub input: CopyOptionsInput,
    pub links: CopyLinkPolicy,
}

/// Aggregate meaningful-payload allocation budget, not a native support or path
/// length claim. Native readers also validate their own format's checked lengths
/// before allocation and fail explicitly rather than truncating a representation.
pub const MAX_TRANSFER_LINK_PAYLOAD_BYTES: usize = 64 * 1024;

/// Positive declarations stay separate from display-only EntryKind values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferLinkKind {
    LinuxSymbolicLink,
    WindowsFileSymbolicLink,
    WindowsDirectorySymbolicLink,
    /// Only a separately qualified local junction; never a volume mount.
    WindowsJunction,
}

/// Lossless meaningful payload after native framing/offset validation. Native
/// reserved or uninitialized padding is excluded from representation equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferLinkFormat {
    LinuxSymlinkBytes,
    WindowsSymbolicLinkUtf16,
    WindowsJunctionUtf16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LinkPayload {
    Linux(Box<[u8]>),
    Windows {
        substitute: Box<[u16]>,
        print: Box<[u16]>,
    },
}

/// Bounded, immutable native representation. Compare the complete value, never
/// display text, resolved targets or sampled content. Relative targets retain
/// their exact bytes/code units; destination-relative meaning may differ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferLinkRepresentation {
    kind: TransferLinkKind,
    format: TransferLinkFormat,
    flags: u32,
    payload: LinkPayload,
}

impl TransferLinkRepresentation {
    /// Raw readlink bytes, without decoding, rebasing or opening the referent.
    pub fn linux_symbolic_link(target: &[u8]) -> Result<Self, FsError> {
        check_payload_len(target.len())?;
        if target.is_empty() || target.contains(&0) {
            return Err(invalid_representation(
                "empty or NUL-containing link target",
            ));
        }
        Ok(Self {
            kind: TransferLinkKind::LinuxSymbolicLink,
            format: TransferLinkFormat::LinuxSymlinkBytes,
            flags: 0,
            payload: LinkPayload::Linux(target.into()),
        })
    }

    /// Exact substitute/print names, excluding native terminators. Unpaired
    /// UTF-16 units are preserved. The adapter must first validate native tag,
    /// framing, offsets, alignment and the retained object's directory distinction.
    /// Only the native relative-target flag (bit 0) is currently declared.
    pub fn windows_symbolic_link(
        directory: bool,
        flags: u32,
        substitute: &[u16],
        print: &[u16],
    ) -> Result<Self, FsError> {
        if flags & !1 != 0 {
            return Err(super::transfer_unsupported(
                Path::new(""),
                "symbolic link flags are unsupported",
            ));
        }
        checked_windows_names(substitute, print)?;
        Ok(Self {
            kind: if directory {
                TransferLinkKind::WindowsDirectorySymbolicLink
            } else {
                TransferLinkKind::WindowsFileSymbolicLink
            },
            format: TransferLinkFormat::WindowsSymbolicLinkUtf16,
            flags,
            payload: LinkPayload::Windows {
                substitute: substitute.into(),
                print: print.into(),
            },
        })
    }

    /// Exact names of an already qualified local junction. The native adapter
    /// must validate framing/offsets and reject volume mounts or unsupported forms
    /// before calling this constructor. This type alone is no endpoint admission.
    pub fn windows_junction(substitute: &[u16], print: &[u16]) -> Result<Self, FsError> {
        checked_windows_names(substitute, print)?;
        Ok(Self {
            kind: TransferLinkKind::WindowsJunction,
            format: TransferLinkFormat::WindowsJunctionUtf16,
            flags: 0,
            payload: LinkPayload::Windows {
                substitute: substitute.into(),
                print: print.into(),
            },
        })
    }

    pub fn kind(&self) -> TransferLinkKind {
        self.kind
    }

    pub fn format(&self) -> TransferLinkFormat {
        self.format
    }

    pub fn meaningful_flags(&self) -> u32 {
        self.flags
    }

    pub fn payload_len(&self) -> usize {
        match &self.payload {
            LinkPayload::Linux(target) => target.len(),
            // The checked aggregate was accepted before either immutable allocation.
            LinkPayload::Windows { substitute, print } => (substitute.len() + print.len()) * 2,
        }
    }

    pub fn linux_target(&self) -> Option<&[u8]> {
        match &self.payload {
            LinkPayload::Linux(target) => Some(target),
            LinkPayload::Windows { .. } => None,
        }
    }

    pub fn windows_names(&self) -> Option<(&[u16], &[u16])> {
        match &self.payload {
            LinkPayload::Linux(_) => None,
            LinkPayload::Windows { substitute, print } => Some((substitute, print)),
        }
    }
}

fn check_payload_len(bytes: usize) -> Result<(), FsError> {
    if bytes > MAX_TRANSFER_LINK_PAYLOAD_BYTES {
        return Err(invalid_representation(
            "link representation exceeds its payload budget",
        ));
    }
    Ok(())
}

fn checked_windows_names(substitute: &[u16], print: &[u16]) -> Result<(), FsError> {
    let bytes = substitute
        .len()
        .checked_add(print.len())
        .and_then(|units| units.checked_mul(2))
        .ok_or_else(|| invalid_representation("link representation length overflow"))?;
    check_payload_len(bytes)?;
    if substitute.is_empty() || substitute.contains(&0) || print.contains(&0) {
        return Err(invalid_representation(
            "empty substitute name or NUL-containing link name",
        ));
    }
    Ok(())
}

fn invalid_representation(message: &str) -> FsError {
    FsError::from_io(
        "validate transfer link",
        Path::new(""),
        std::io::Error::new(std::io::ErrorKind::InvalidData, message),
    )
}

/// A failed construction retains any partially created stage for explicit worker
/// cleanup. An error with no stage certifies that no owned unpublished residue
/// remains. Retain source/provider ownership until the returned stage is retired;
/// a partial stage grants cleanup only and must never be published or retried.
#[must_use = "retire an owned partial link stage explicitly on a backend worker"]
pub struct TransferLinkStageError {
    pub error: FsError,
    pub stage: Option<Box<dyn StagedTransferLink>>,
}

impl fmt::Debug for TransferLinkStageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferLinkStageError")
            .field("error", &self.error)
            .field("has_stage", &self.stage.is_some())
            .finish()
    }
}

impl fmt::Display for TransferLinkStageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for TransferLinkStageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Truthful cleanup facts for the owned stage and its optional private directory.
/// Known residual paths stay visible; unknown identities are never invented.
/// Set cleanup_unconfirmed if absence or ownership cannot be verified, including
/// substituted slots/parents. These facts grant no further deletion authority.
#[derive(Clone, Debug)]
#[must_use = "report link cleanup residuals or unconfirmed cleanup"]
pub struct TransferLinkCleanupError {
    pub error: FsError,
    pub recovery: Vec<TransferRecovery>,
    pub cleanup_unconfirmed: bool,
}

impl fmt::Display for TransferLinkCleanupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for TransferLinkCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Distinct owned link stage, never regular-file content or Move authority.
/// All calls and final destruction belong on backend workers, with the source
/// and provider registration retained through actual cleanup/destruction.
pub trait StagedTransferLink: Send {
    /// Independently query the owned native link; never echo cached creation
    /// bytes or open the target. Validate framing and bound allocation first.
    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError>;

    /// Synchronize the owned representation and relevant native namespace state.
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError>;

    /// Immediately before publication, independently check the owned stage slot,
    /// retained parent and exact current representation against expected. Return
    /// the stage identity for comparison with the later owned publication. Fail
    /// rather than adopting a substituted stage or parent, even with equal data.
    fn revalidate_stage(
        &self,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError>;

    /// Publish only the revalidated owned object, without replacing any existing
    /// entry (including dangling links). Err certifies no namespace mutation by
    /// this call. After any mutation, return TransferPublication facts even if
    /// cancellation, durability or validation fails; never hide publication.
    /// Replaced is not a permitted outcome for this no-replace capability.
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError>;

    /// Check the final name, retained parent, exact owned identity and raw current
    /// representation. Never adopt a winner. Failure cannot authorize deleting
    /// a published entry or erase an earlier structured publication outcome.
    fn revalidate_publication(
        &self,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError>;

    /// Explicitly remove only the owned unpublished stage and empty owned private
    /// directory after verifying their bindings. Never follow targets, recurse,
    /// delete an unknown entry, or remove a published object. Success certifies
    /// no owned unpublished residual; failure reports known residual/unconfirmed
    /// facts and ends automatic retry. On failure the provider must latch refusal:
    /// neither another cleanup call nor Drop may silently retry namespace deletion.
    /// Destruction releases retained pins only; it must never delete a substituted
    /// entry. Drop cannot replace this explicit reported result.
    fn cleanup(&mut self) -> Result<(), TransferLinkCleanupError>;
}

// Contract tests were authored before the candidate implementation. Execution is
// intentionally deferred to the Build owner; no result is claimed here.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CapturedTransferSources, EntryKind, EntryMetadata, MutationItem, PreparedCapturedTransfer,
        StagedTransferFile, TransferDirectory, TransferEntry, TransferIdentity, TransferIntent,
        TransferService, TransferSource,
    };
    use std::{
        collections::VecDeque,
        ffi::OsStr,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    #[test]
    fn link_policy_defaults_to_reject() {
        assert_eq!(CopyLinkPolicy::default(), CopyLinkPolicy::Reject);
    }

    #[test]
    fn linux_representation_preserves_relative_non_utf8_target_bytes() {
        let target = b"../raw-\xff/../missing";
        let representation = TransferLinkRepresentation::linux_symbolic_link(target).unwrap();
        assert_eq!(representation.kind(), TransferLinkKind::LinuxSymbolicLink);
        assert_eq!(
            representation.format(),
            TransferLinkFormat::LinuxSymlinkBytes
        );
        assert_eq!(representation.meaningful_flags(), 0);
        assert_eq!(representation.linux_target(), Some(target.as_slice()));
        assert_eq!(representation.payload_len(), target.len());
        assert!(representation.windows_names().is_none());
    }

    #[test]
    fn linux_representation_rejects_empty_nul_and_excess_budget_without_truncation() {
        assert!(TransferLinkRepresentation::linux_symbolic_link(b"").is_err());
        assert!(TransferLinkRepresentation::linux_symbolic_link(b"one\0two").is_err());
        let target = vec![b'x'; MAX_TRANSFER_LINK_PAYLOAD_BYTES + 1];
        assert!(TransferLinkRepresentation::linux_symbolic_link(&target).is_err());
        let target = &target[..MAX_TRANSFER_LINK_PAYLOAD_BYTES];
        let representation = TransferLinkRepresentation::linux_symbolic_link(target).unwrap();
        assert_eq!(representation.linux_target(), Some(target));
    }

    #[test]
    fn windows_representation_preserves_names_unpaired_units_flags_and_directory_kind() {
        let substitute = [b'.' as u16, b'.' as u16, b'\\' as u16, 0xd800];
        let print = [0xdc00, b'x' as u16];
        let file = TransferLinkRepresentation::windows_symbolic_link(false, 1, &substitute, &print)
            .unwrap();
        let directory =
            TransferLinkRepresentation::windows_symbolic_link(true, 1, &substitute, &print)
                .unwrap();
        assert_eq!(file.kind(), TransferLinkKind::WindowsFileSymbolicLink);
        assert_eq!(
            directory.kind(),
            TransferLinkKind::WindowsDirectorySymbolicLink
        );
        assert_eq!(file.format(), TransferLinkFormat::WindowsSymbolicLinkUtf16);
        assert_eq!(file.meaningful_flags(), 1);
        assert_eq!(
            file.windows_names(),
            Some((substitute.as_slice(), print.as_slice()))
        );
        assert_eq!(file.payload_len(), 12);
        assert!(file.linux_target().is_none());
        assert_ne!(file, directory);
    }

    #[test]
    fn windows_representation_rejects_unknown_flags_empty_substitute_and_nul() {
        assert!(
            TransferLinkRepresentation::windows_symbolic_link(false, 2, &[b'x' as u16], &[])
                .is_err()
        );
        assert!(TransferLinkRepresentation::windows_symbolic_link(false, 0, &[], &[]).is_err());
        assert!(TransferLinkRepresentation::windows_symbolic_link(false, 0, &[0], &[]).is_err());
        assert!(TransferLinkRepresentation::windows_junction(&[b'x' as u16], &[0]).is_err());
    }

    #[test]
    fn windows_payload_budget_counts_both_names_as_bytes() {
        let substitute = vec![b'x' as u16; MAX_TRANSFER_LINK_PAYLOAD_BYTES / 2 - 1];
        let print = [b'p' as u16];
        let representation =
            TransferLinkRepresentation::windows_junction(&substitute, &print).unwrap();
        assert_eq!(
            representation.payload_len(),
            MAX_TRANSFER_LINK_PAYLOAD_BYTES
        );
        assert_eq!(representation.kind(), TransferLinkKind::WindowsJunction);
        assert_eq!(
            representation.format(),
            TransferLinkFormat::WindowsJunctionUtf16
        );
        assert_eq!(representation.meaningful_flags(), 0);
        assert!(TransferLinkRepresentation::windows_junction(&substitute, &[1, 2]).is_err());
    }

    #[test]
    fn representation_equality_detects_raw_payload_flags_and_native_kind_changes() {
        let linux = TransferLinkRepresentation::linux_symbolic_link(b"../target").unwrap();
        assert_ne!(
            linux,
            TransferLinkRepresentation::linux_symbolic_link(b"target").unwrap()
        );
        let substitute = [b'x' as u16];
        let file =
            TransferLinkRepresentation::windows_symbolic_link(false, 0, &substitute, &[]).unwrap();
        assert_ne!(
            file,
            TransferLinkRepresentation::windows_symbolic_link(false, 1, &substitute, &[]).unwrap()
        );
        assert_ne!(
            file,
            TransferLinkRepresentation::windows_symbolic_link(false, 0, &substitute, &[1]).unwrap()
        );
        assert_ne!(
            file,
            TransferLinkRepresentation::windows_junction(&substitute, &[]).unwrap()
        );
    }

    #[test]
    fn default_service_reject_delegates_but_preserve_never_opens_legacy_source() {
        let service = LegacyService::default();
        let cancel = Cancellation::default();
        let path = Path::new("captured-link");
        service
            .prepare_transfer_source_with_links(path, CopyLinkPolicy::Reject, &cancel)
            .unwrap();
        assert_eq!(service.calls.load(Ordering::Relaxed), 1);
        let error = service
            .prepare_transfer_source_with_links(path, CopyLinkPolicy::Preserve, &cancel)
            .err()
            .unwrap();
        assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
        assert_eq!(service.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn default_capture_reject_delegates_but_preserve_never_prepares_legacy_roots() {
        let sources = LegacyCaptured::default();
        let service = LegacyService::default();
        let cancel = Cancellation::default();
        sources
            .prepare_with_links(&service, CopyLinkPolicy::Reject, &cancel)
            .unwrap();
        assert_eq!(sources.calls.load(Ordering::Relaxed), 1);
        let error = sources
            .prepare_with_links(&service, CopyLinkPolicy::Preserve, &cancel)
            .err()
            .unwrap();
        assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
        assert_eq!(sources.calls.load(Ordering::Relaxed), 1);
        assert_eq!(service.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn default_source_representation_is_unsupported_without_content_reads() {
        let source = legacy_source(Path::new("captured-link"));
        let error = source
            .link_representation(&Cancellation::default())
            .unwrap_err();
        assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
    }

    #[test]
    fn default_directory_rejects_link_admission_and_has_no_partial_stage() {
        let directory = LegacyDirectory;
        let representation = TransferLinkRepresentation::linux_symbolic_link(b"target").unwrap();
        let cancel = Cancellation::default();
        let error = directory.admit_link(&representation, &cancel).unwrap_err();
        assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
        let failure = directory
            .stage_link(&representation, &cancel)
            .err()
            .unwrap();
        assert_eq!(failure.error.kind, std::io::ErrorKind::Unsupported);
        assert!(failure.stage.is_none());
    }

    #[derive(Default)]
    struct LegacyService {
        calls: AtomicUsize,
    }

    impl TransferService for LegacyService {
        fn prepare_transfer_source(
            &self,
            path: &Path,
            _cancel: &Cancellation,
        ) -> Result<Box<dyn TransferSource>, FsError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(legacy_source(path)))
        }

        fn prepare_transfer_destination(
            &self,
            path: &Path,
            _cancel: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            Err(FsError::from_io(
                "test destination",
                path,
                std::io::ErrorKind::Unsupported.into(),
            ))
        }
    }

    #[derive(Debug, Default)]
    struct LegacyCaptured {
        calls: AtomicUsize,
    }

    impl CapturedTransferSources for LegacyCaptured {
        fn intent(&self) -> TransferIntent {
            TransferIntent::Copy
        }

        fn root_count(&self) -> usize {
            0
        }

        fn metadata_available(&self) -> bool {
            false
        }

        fn prepare(
            &self,
            _service: &dyn TransferService,
            _cancel: &Cancellation,
        ) -> Result<PreparedCapturedTransfer, FsError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(PreparedCapturedTransfer::Copy(VecDeque::new()))
        }
    }

    struct LegacySource {
        entry: TransferEntry,
    }

    fn legacy_source(path: &Path) -> LegacySource {
        LegacySource {
            entry: TransferEntry {
                item: MutationItem {
                    path: PathBuf::from(path),
                    kind: EntryKind::Symlink,
                    metadata: EntryMetadata::default(),
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [1; 16],
                },
            },
        }
    }

    impl TransferSource for LegacySource {
        fn entry(&self) -> &TransferEntry {
            &self.entry
        }

        fn revalidate(&self, _cancel: &Cancellation) -> Result<(), FsError> {
            Ok(())
        }

        fn read_chunk(
            &mut self,
            _buffer: &mut [u8],
            _cancel: &Cancellation,
        ) -> Result<usize, FsError> {
            panic!("link representation must not invoke content reads")
        }

        fn next_child(
            &mut self,
            _cancel: &Cancellation,
        ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
            panic!("link representation must not enumerate a referent")
        }
    }

    struct LegacyDirectory;

    impl TransferDirectory for LegacyDirectory {
        fn path(&self) -> &Path {
            Path::new("destination")
        }

        fn identity(&self) -> TransferIdentity {
            TransferIdentity {
                device: 2,
                object: [2; 16],
            }
        }

        fn revalidate(&self, _cancel: &Cancellation) -> Result<(), FsError> {
            Ok(())
        }

        fn inspect(
            &self,
            _name: &OsStr,
            _cancel: &Cancellation,
        ) -> Result<Option<TransferEntry>, FsError> {
            panic!("unsupported link admission must not inspect a target")
        }

        fn reject_source_ancestor(
            &self,
            _source: TransferIdentity,
            _cancel: &Cancellation,
        ) -> Result<(), FsError> {
            panic!("unsupported link admission must not inspect source ancestry")
        }

        fn create_directory(
            &self,
            _name: &OsStr,
            _cancel: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            panic!("unsupported link admission must not create a directory")
        }

        fn stage_file(
            &self,
            _cancel: &Cancellation,
        ) -> Result<Box<dyn StagedTransferFile>, FsError> {
            panic!("link staging must not fall back to content staging")
        }
    }
}
