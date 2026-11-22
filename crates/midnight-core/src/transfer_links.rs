//! App-private link Copy contracts; link representations never grant target access.

use crate::{
    Cancellation, CapturedTransferRequest, FsError, MetadataTransferRequest, TransferIdentity,
    TransferPublication, TransferRecovery,
};
use std::{ffi::OsStr, fmt, path::Path};

/// Immutable link policy for the copy side of a transfer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CopyLinkPolicy {
    /// Legacy preparation rejects links.
    #[default]
    Reject,
    /// Stage and verify the exact supported link representation as a leaf, never
    /// following it. Copy recreates every link; Move moves the link itself (a rename
    /// on one volume; across volumes the link is recreated, then the original removed).
    Preserve,
    /// Materialize each qualified directory link as a directory with verified contents.
    /// A qualified target may lie on another filesystem than the link (each platform
    /// qualifies it; see TRANSFERS.md), but never on a system filesystem such as
    /// Linux's /proc.
    FollowDirectoryContents,
    /// Materialize each qualified directory link as an empty ordinary directory.
    EmptyDirectories,
    /// Copy what each link points to. A directory link is followed as with
    /// `FollowDirectoryContents` (cycles are refused the same way) and a link to a
    /// regular file is copied as that file's content, also where the target lies on
    /// another filesystem than the link. A link that cannot be followed (dangling,
    /// unreadable, pointing to a special file, into a system filesystem such as
    /// /proc, or back to an active ancestor) is recreated as a link, as `Preserve`
    /// does, and the source reports why through `TransferSource::link_not_followed`.
    /// Move copies the content the same way, verified like any copy, and then
    /// removes only the link itself: what it points to is never removed or changed.
    /// Move never renames a folder whole under this policy, so that the links in it
    /// are followed too.
    FollowLinks,
}

impl CopyLinkPolicy {
    /// Whether the source must project supported directory links as ordinary directories.
    pub fn materializes_directory_links(self) -> bool {
        matches!(
            self,
            Self::FollowDirectoryContents | Self::EmptyDirectories | Self::FollowLinks
        )
    }

    /// Whether directory links are followed into their contents, so followed
    /// referents take part in ancestry checks.
    pub fn follows_directory_links(self) -> bool {
        matches!(self, Self::FollowDirectoryContents | Self::FollowLinks)
    }

    /// Whether the source can yield link leaves that are recreated as links.
    pub fn recreates_links(self) -> bool {
        matches!(self, Self::Preserve | Self::FollowLinks)
    }
}

/// Reuses the existing intent, verification and required-metadata inputs.
#[derive(Clone, Debug)]
pub enum CopyOptionsInput {
    Native(MetadataTransferRequest),
    Captured(CapturedTransferRequest),
}

/// Explicit options frozen at admission. Preserve requires content-only Copy.
/// Directory materialization applies when contents are copied. Move requires
/// independent authority over the original link to finalize a materialized branch.
#[derive(Clone, Debug)]
pub struct CopyOptionsRequest {
    pub input: CopyOptionsInput,
    pub links: CopyLinkPolicy,
}

/// The link a destination will create for a source link (`TransferDirectory::plan_link`):
/// the source's exact representation, or another form the destination substitutes for
/// one it cannot create (a Windows junction for a directory symbolic link that this
/// process may not create), with the reason for the job details. The stage is verified
/// against this representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferLinkPlan {
    pub representation: TransferLinkRepresentation,
    /// Why `representation` differs from the source's, in words for the job details.
    pub note: Option<String>,
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

    /// Whether this link's relative text, resolved name by name from a folder `depth`
    /// levels below a reference folder, leads above that folder, as `../x` does from the
    /// reference folder itself. Only the text decides, never what exists; absolute texts
    /// (and junctions, which are always absolute) never do.
    pub fn relative_text_leaves(&self, depth: usize) -> bool {
        // Each element is a name: `..` goes up, `.` and empty names stay.
        fn leaves<'a>(depth: usize, names: impl Iterator<Item = &'a [u8]>) -> bool {
            let mut depth = depth;
            for name in names {
                match name {
                    b".." => match depth.checked_sub(1) {
                        Some(up) => depth = up,
                        None => return true,
                    },
                    b"." | b"" => {}
                    _ => depth += 1,
                }
            }
            false
        }
        match &self.payload {
            LinkPayload::Linux(target) => {
                target.first() != Some(&b'/') && leaves(depth, target.split(|byte| *byte == b'/'))
            }
            LinkPayload::Windows { substitute, .. } => {
                let symbolic = matches!(
                    self.kind,
                    TransferLinkKind::WindowsFileSymbolicLink
                        | TransferLinkKind::WindowsDirectorySymbolicLink
                );
                if !symbolic || self.flags & 1 == 0 {
                    return false;
                }
                // Only `.` and `..` matter; any other name is one level down.
                let names: Vec<&[u8]> = substitute
                    .split(|unit| *unit == u16::from(b'\\') || *unit == u16::from(b'/'))
                    .map(|name| match name {
                        [] => b"".as_slice(),
                        [dot] if *dot == u16::from(b'.') => b".".as_slice(),
                        [first, second]
                            if *first == u16::from(b'.') && *second == u16::from(b'.') =>
                        {
                            b"..".as_slice()
                        }
                        _ => b"name".as_slice(),
                    })
                    .collect();
                leaves(depth, names.into_iter())
            }
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

    /// Bind the unpublished stage to replace the consented target under `policy`,
    /// without changing the namespace, as `StagedTransferFile::prepare_replacement`
    /// does. A later publication with another name never replaces.
    fn prepare_replacement(
        &mut self,
        _name: &OsStr,
        _expected: std::sync::Arc<dyn crate::TransferTarget>,
        _policy: crate::TransferReplacementPolicy,
        _cancel: &Cancellation,
    ) -> Result<(), FsError> {
        Err(super::transfer_unsupported(
            Path::new(""),
            "link replacement is unsupported",
        ))
    }

    /// Publish the revalidated owned link over the bound target, keeping the
    /// displaced entry in private recovery. Same outcome rules as
    /// `StagedTransferFile::publish_replace`: Err certifies that no user entry moved.
    fn publish_replace(
        &mut self,
        _name: &OsStr,
        _cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        Err(super::transfer_unsupported(
            Path::new(""),
            "link replacement is unsupported",
        ))
    }

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
    fn follow_links_materializes_follows_and_recreates_what_it_cannot_follow() {
        let policy = CopyLinkPolicy::FollowLinks;
        assert!(policy.materializes_directory_links());
        assert!(policy.follows_directory_links());
        assert!(policy.recreates_links());
        assert!(CopyLinkPolicy::Preserve.recreates_links());
        assert!(!CopyLinkPolicy::Preserve.materializes_directory_links());
        assert!(!CopyLinkPolicy::EmptyDirectories.follows_directory_links());
        for legacy in [
            CopyLinkPolicy::Reject,
            CopyLinkPolicy::FollowDirectoryContents,
            CopyLinkPolicy::EmptyDirectories,
        ] {
            assert!(!legacy.recreates_links(), "{legacy:?}");
        }
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
    fn relative_text_leaves_only_when_its_dot_dots_climb_above_the_reference_folder() {
        let linux = |text: &[u8]| TransferLinkRepresentation::linux_symbolic_link(text).unwrap();
        // From the reference folder itself (depth 0), any climb leaves it.
        assert!(linux(b"../x").relative_text_leaves(0));
        assert!(!linux(b"x/../y").relative_text_leaves(0));
        assert!(linux(b"x/../../y").relative_text_leaves(0));
        // One folder below it, one climb stays inside, two leave.
        assert!(!linux(b"../sibling").relative_text_leaves(1));
        assert!(linux(b"../../outside").relative_text_leaves(1));
        assert!(!linux(b"./a//b/../../c").relative_text_leaves(0));
        // An absolute text is not relative, wherever it points.
        assert!(!linux(b"/etc/../..").relative_text_leaves(0));
        let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
        let relative = TransferLinkRepresentation::windows_symbolic_link(
            false,
            1,
            &wide(r"..\..\x"),
            &wide(r"..\..\x"),
        )
        .unwrap();
        assert!(relative.relative_text_leaves(1));
        assert!(!relative.relative_text_leaves(2));
        // Forward slashes separate names in a relative Windows text as well.
        let slashes =
            TransferLinkRepresentation::windows_symbolic_link(true, 1, &wide("../x"), &wide(""))
                .unwrap();
        assert!(slashes.relative_text_leaves(0));
        // Absolute symbolic links and junctions never count as relative.
        let absolute = TransferLinkRepresentation::windows_symbolic_link(
            true,
            0,
            &wide(r"\??\C:\..\x"),
            &wide(r"C:\..\x"),
        )
        .unwrap();
        assert!(!absolute.relative_text_leaves(0));
        let junction =
            TransferLinkRepresentation::windows_junction(&wide(r"\??\C:\x"), &wide(r"C:\x"))
                .unwrap();
        assert!(!junction.relative_text_leaves(0));
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
    fn default_service_reject_delegates_but_explicit_link_options_fail_closed() {
        let service = LegacyService::default();
        let cancel = Cancellation::default();
        let path = Path::new("captured-link");
        service
            .prepare_transfer_source_with_links(path, CopyLinkPolicy::Reject, &cancel)
            .unwrap();
        assert_eq!(service.calls.load(Ordering::Relaxed), 1);
        for policy in [
            CopyLinkPolicy::Preserve,
            CopyLinkPolicy::FollowDirectoryContents,
            CopyLinkPolicy::EmptyDirectories,
            CopyLinkPolicy::FollowLinks,
        ] {
            let error = service
                .prepare_transfer_source_with_links(path, policy, &cancel)
                .err()
                .unwrap();
            assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
        }
        assert_eq!(service.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn default_capture_reject_delegates_but_explicit_link_options_fail_closed() {
        let sources = LegacyCaptured::default();
        let service = LegacyService::default();
        let cancel = Cancellation::default();
        sources
            .prepare_with_links(&service, CopyLinkPolicy::Reject, &cancel)
            .unwrap();
        assert_eq!(sources.calls.load(Ordering::Relaxed), 1);
        for policy in [
            CopyLinkPolicy::Preserve,
            CopyLinkPolicy::FollowDirectoryContents,
            CopyLinkPolicy::EmptyDirectories,
            CopyLinkPolicy::FollowLinks,
        ] {
            let error = sources
                .prepare_with_links(&service, policy, &cancel)
                .err()
                .unwrap();
            assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
        }
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
