# Move from inline directory trees

F6 must move the actual selected native occurrences in an expanded panel,
including nested files and directories. An empty selection uses its focused
ordinary occurrence. Parent `..`, filter-only context ancestors, links/special
files and unsupported providers cannot gain destructive authority. Archive Move
remains unavailable; archive display paths must never become native paths.

This is the next integration of the user's accepted tree and ordinary Move
requirements. Each file is copied, fully verified and removed before the next
file; completed changes remain on later error/cancel. No user-facing source
recovery catalogue or whole-batch rollback is introduced.

## Approach and contracts

Generalize the existing exact tree Copy capture into an intent-bound transfer.
This is an implementation choice within the approved requirements. Reusing
display paths as a flat selection would discard provider/occurrence identity.
A separate Move pipeline would duplicate the already qualified verification,
conflict and native removal logic. Both are unsuitable.

`TreeTransferRequest` replaces `TreeCopyRequest` and adds `intent: TransferIntent`.
`TreeCommand::Transfer` returns `TreeEvent::Transfer` with `TreeTransferCapture`.
The capture contains cached intent, root count, metadata availability and an
opaque `Arc<dyn CapturedTransferSources>`. That trait exposes cached `intent()`,
`root_count()` and `metadata_available()`, and worker-only `prepare(service,
cancel) -> Result<PreparedCapturedTransfer, FsError>`. The prepared enum contains
either Copy source roots or Move roots with `PreparedTransferMove` authority.
`CapturedTransferRequest` is confirmed through `Command::EnqueueCapturedTransfer`;
its operation type comes from the retained sources, never a mutable UI override.
The returned prepared variant must match that bound intent before destination
preparation or mutation.

Capture uses the acknowledged immutable view/selection and keeps the existing
four live capture permits and 4096-target/4 MiB capture budgets. Confirmation
uses the ordinary 32-job FIFO admission and shared mutation gate. The existing
Move admission/traversal budgets remain unchanged. They are resource budgets,
not path/file-length limits. Copy behavior and optional supported native exact
modified time are preserved.

Before accepting a Move capture, inspect every typed target for native support.
Preparation rechecks the whole capture before any source is prepared. A mixed
native/archive batch fails wholly. Native preparation uses the retained service's
Move authority, not its read-only Copy authority. Native identity is first pinned
during ordinary worker preparation and retained across Retry: a draft captures
locations/provider ownership, not an immutable inode or filesystem snapshot.

Extract the current native Move preparation so both ordinary paths and prepared
captured roots share validation and job construction. Retain all existing kind,
source/capture identity, no-follow, alias, selected ancestry, destination ancestry,
proof, metadata and finalization checks before writes. Do not add a rename-only
shortcut that bypasses the user's full checksum requirement.

## Ownership and completion

Every prepared source and Move capture retains the provider lease. Descendant
sources/captures inherit that lease but never emit selected-root completion.
Finalizers and detached stage-cleanup tickets retain the lease separately, without
keeping source/capture handles alive: the worker intentionally closes those
handles before stage cleanup. Normal cleanup, rejected sends, stale results,
cancelled drafts and async shutdown keep the existing backend retirement contract.

The shared successful-root occurrence feedback remains `TreeCopyOccurrence` /
`CompletedCopySource::Tree` on the existing Copy/Move event channel. Completion
waits for this root's actual verified removal and directory postorder cleanup.
Skipped, failed or cancelled roots stay marked while their source session remains
visible; navigation/refresh retires that scope as already documented. Existing
ordered Remove, reprojection and session guards remain shared with Copy.

## UI and acceptance

Reuse the existing Move destination dialog, metadata option, queue, conflicts,
Retry/Skip/Cancel and progress presentation. F5 stays Copy; F6 cannot silently
become Copy. Draft navigation and refused command sends retain the original
capture and intent. Known archive panels report unsupported Move immediately;
backend checks also protect future mixed-provider tree layouts.

Test the real F6 callback/dialog/confirmation path, stale results, pending
selection, navigation, queue refusal and Copy/Move intent separation. Backend
tests must exercise nested native roots and files, exact output bytes and
per-file removal ordering, skip/cancel/error retention, optional metadata,
overlap/destination-under-source, duplicate aliases, Retry source replacement,
native/archive mixtures and lease cleanup. Use owned fixtures only. Run existing
Copy/Move/tree suites on Windows and Linux and rebuild/smoke the debug app.

No new third-party dependency or plugin ABI is needed. Native UI bridge tests use
the existing internal platform crate as a dev-dependency. Keep all non-UI work on backend
workers, platform implementations separate, lossless native/provider identities,
100-column rustfmt and handwritten source files at most 100000 bytes. Preserve
the unrelated dirty working tree. Broader provider mutation, clipboard and tree
attachment work remains in the application goal.

## Reconciliation acceptance gate

Integration review identified that the ordinary path-matched terminal refresh
misses nested source parents, while whole-pane refresh discards an adopted tree.
F6 is not ready to ship until the
[native in-session reconciliation design](2026-11-10-tree-transfer-reconciliation.md)
is implemented and tested. It covers root/nested/partial transfers, preserves
surviving marks and expansions, and supplies current backend-owned root action
snapshots so newly discovered files retain F3 and Enter. The initial admission
snapshot and destructive authority remain unchanged.
