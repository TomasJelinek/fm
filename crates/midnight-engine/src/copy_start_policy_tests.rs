//! Admission transports the explicit choice through wrappers and preparation retries.
use super::*;

fn paths(policy: TransferStartPolicy) -> CopyRequest {
    let root = std::env::current_dir().unwrap();
    CopyRequest {
        start_policy: policy,
        verification: TransferVerification::FullSha256,
        id: OperationId(1),
        clipboard: FileClipboard {
            paths: vec![root.join("policy-source")],
            intent: FileClipboardIntent::Copy,
        },
        destination: root.join("policy-destination"),
    }
}

fn selected(policy: TransferStartPolicy) -> SelectedCopyRequest {
    let native = paths(policy);
    SelectedCopyRequest {
        start_policy: policy,
        verification: native.verification,
        id: native.id,
        files: SelectedFiles {
            view: Arc::new(DirectoryIndex::from_entries(&[])),
            selection: Arc::default(),
            fallback: native.clipboard.paths.into_iter().next(),
        },
        destination: native.destination,
    }
}

fn clipboard(policy: TransferStartPolicy) -> ClipboardMoveRequest {
    let mut native = paths(policy);
    native.clipboard.intent = FileClipboardIntent::Move;
    ClipboardMoveRequest {
        start_policy: policy,
        verification: native.verification,
        id: native.id,
        clipboard: Arc::new(native.clipboard),
        destination: native.destination,
    }
}

#[derive(Debug)]
struct Capture(TransferIntent);
impl CapturedTransferSources for Capture {
    fn intent(&self) -> TransferIntent {
        self.0
    }
    fn root_count(&self) -> usize {
        1
    }
    fn metadata_available(&self) -> bool {
        false
    }
    fn prepare(
        &self,
        _service: &dyn TransferService,
        _cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        panic!("transporting a start policy must not prepare providers")
    }
}

fn captured(policy: TransferStartPolicy) -> CapturedTransferRequest {
    let native = paths(policy);
    CapturedTransferRequest {
        start_policy: policy,
        verification: native.verification,
        id: native.id,
        sources: Arc::new(Capture(TransferIntent::Copy)),
        destination: native.destination,
        metadata: TransferMetadataPolicy::ContentOnly,
    }
}

fn archive(policy: TransferStartPolicy) -> ArchiveCopyRequest {
    let native = paths(policy);
    ArchiveCopyRequest {
        start_policy: policy,
        verification: native.verification,
        id: native.id,
        location: ArchiveLocation {
            mount: None,
            address: ArchiveAddress {
                provider: "policy-fixture".into(),
                format: None,
                container: native.clipboard.paths[0].clone(),
                source_version: vec![1],
                components: Vec::new(),
            },
            directory: 1,
            trail: Vec::new(),
        },
        entries: Arc::from([]),
        view: Arc::new(DirectoryIndex::from_entries(&[])),
        selected: Arc::default(),
        fallback: None,
        destination: native.destination,
    }
}

#[test]
fn every_leaf_route_preserves_its_start_policy_when_cloned_for_retry() {
    for policy in [
        TransferStartPolicy::Immediate,
        TransferStartPolicy::WaitForEarlier,
    ] {
        let inputs = [
            CopyInput::from(paths(policy)),
            CopyInput::from(selected(policy)),
            CopyInput::move_clipboard(clipboard(policy)),
            CopyInput::from(archive(policy)),
            CopyInput::from(captured(policy)),
        ];
        for input in inputs {
            assert_eq!(input.start_policy, policy);
            assert_eq!(input.clone().start_policy, policy);
        }
    }
}

#[test]
fn move_metadata_and_link_options_preserve_the_nested_policy() {
    let policy = TransferStartPolicy::WaitForEarlier;
    let inputs = [
        MetadataTransferInput::Paths(paths(policy)),
        MetadataTransferInput::Selected(selected(policy)),
        MetadataTransferInput::Clipboard(clipboard(policy)),
    ];
    for input in inputs {
        let input = CopyInput::options(CopyOptionsRequest {
            input: CopyOptionsInput::Native(MetadataTransferRequest {
                input,
                intent: TransferIntent::Move,
                metadata: TransferMetadataPolicy::RequireModifiedTime,
            }),
            links: CopyLinkPolicy::Reject,
        });
        assert_eq!(input.start_policy, policy);
        assert_eq!(input.intent, TransferIntent::Move);
        assert_eq!(input.metadata, TransferMetadataPolicy::RequireModifiedTime);
    }
    let input = CopyInput::options(CopyOptionsRequest {
        input: CopyOptionsInput::Captured(captured(policy)),
        links: CopyLinkPolicy::Reject,
    });
    assert_eq!(input.start_policy, policy);
}

#[test]
fn directory_link_move_options_reach_native_preparation_and_legacy_captures_refuse() {
    let start = TransferStartPolicy::WaitForEarlier;
    for links in [
        CopyLinkPolicy::FollowDirectoryContents,
        CopyLinkPolicy::EmptyDirectories,
    ] {
        for metadata in [
            TransferMetadataPolicy::ContentOnly,
            TransferMetadataPolicy::RequireModifiedTime,
        ] {
            let mut captured = captured(start);
            captured.sources = Arc::new(Capture(TransferIntent::Move));
            captured.metadata = metadata;
            for input in [
                CopyOptionsInput::Native(MetadataTransferRequest {
                    input: MetadataTransferInput::Paths(paths(start)),
                    intent: TransferIntent::Move,
                    metadata,
                }),
                CopyOptionsInput::Captured(captured),
            ] {
                let is_captured = matches!(&input, CopyOptionsInput::Captured(_));
                let mut input = CopyInput::options(CopyOptionsRequest { input, links });
                input.initialize_captured_intent();
                input.validate().unwrap();
                assert_eq!(input.clone().links, links);
                if metadata == TransferMetadataPolicy::ContentOnly {
                    let service = midnight_platform::LocalFileSystem::new(None);
                    let resolved = input.resolve(&service, &Cancellation::default());
                    if is_captured {
                        assert_eq!(resolved.err().unwrap().kind, io::ErrorKind::Unsupported);
                    } else {
                        assert!(matches!(resolved.unwrap(), Resolved::NativeMove(_)));
                    }
                }
            }
        }
    }
}

#[test]
fn native_preparation_reconstruction_does_not_replace_the_chosen_policy() {
    let service = midnight_platform::LocalFileSystem::new(None);
    for moved in [false, true] {
        let request = paths(TransferStartPolicy::WaitForEarlier);
        let input = if moved {
            CopyInput::move_paths(request)
        } else {
            CopyInput::from(request)
        };
        let resolved = input.resolve(&service, &Cancellation::default()).unwrap();
        let request = match resolved {
            Resolved::Native(request) if !moved => request,
            Resolved::NativeMove(request) if moved => request,
            _ => panic!("native transport changed its intent"),
        };
        assert_eq!(request.start_policy, TransferStartPolicy::WaitForEarlier);
    }
}
