# Native transfer service

These are internal Rust contracts in `midnight-core::transfer`, implemented by
the separate Windows and Linux adapters. They are not Rust trait objects exposed
across a plugin ABI. Archive extraction supplies a brokered `TransferSource` and
uses the same native destination and verification state machine.

## Verification and ownership

`VerifiedFileCopy` performs bounded 256 KiB steps: source read/stage write,
flush/sync, independent complete stage readback, then publication. Source and
readback SHA-256 digests and lengths must match. It revalidates the opened source
before accepting replacement consent and immediately before publishing. A failed,
cancelled or recovery-terminal copy cannot publish again. A late no-replace
collision leaves the verified stage available for a different name or explicit
replacement; the consumed source stream is not replayed.

All methods run on backend workers. UI receives bounded immutable progress and
versioned conflict metadata, and sends a choice with the exact job/version. The
queue currently runs serially on every storage class. More than one stream still
requires positive local SSD classification at both endpoints and shared budgets.

## Destination capabilities

| Contract | Meaning |
| --- | --- |
| `prepare_target` | Capture a native target identity and provider-owned revision for a conflict. The pin remains worker-owned. |
| `directory_merge` / `open_merge_directory` | Revalidate and open the consented ordinary directory without following a link. Never delete an existing tree. |
| `conditional_overwrite` | Strong expected-identity replacement. Currently false on both adapters; ordinary rename is not compare-and-swap. |
| `recoverable_replacement` | Separately selected replace-current-entry policy which preserves displaced objects in private recovery. |
| `prepare_replacement` | Bind an unpublished stage to a basename, target pin and explicit policy without namespace mutation. Also works after readback. |
| `publish_no_replace` | Publish exclusively. Ignores a previous replacement binding, so choosing another name never silently replaces its destination. |
| `publish_replace` | Publish only under the explicit bound policy; enforce its basename and revalidate the pin. |

Default optional methods return Unsupported. Windows currently advertises
recoverable replacement on local fixed NTFS; Linux qualifies local ext
filesystems. Network, removable and unknown replacement semantics are not
silently treated as equivalent. Other no-replace and merge operations retain
their own provider capability/error behavior.

## Conflict policies

Replace/Replace all keep originals. Merge/Merge all keep existing children and
do not authorize leaf replacement. Each all-policy is limited to its job; it
cannot enable a type collision or a source/target alias. All captured native
source roots are protected, including a later selected root that is also an
earlier destination. Destination ancestry checks prevent recursion into sources.

A target change while a decision is pending invalidates that decision. A newly
appearing destination at publication requires a fresh conflict, even after
Replace all. A stale job/version response cannot authorize a new conflict.
Skip, another name and Cancel remain available when replacement is unsupported.

## Publication and recovery

Windows captures the exact guarded old file into a private same-volume directory,
then publishes the verified stage without replacement. This has an explicit
namespace gap. A concurrent new destination wins; both the old file and the
verified stage are retained instead of deleting that winner or rolling back over
it. Linux links its verified anonymous inode into a private directory and uses
an exchange; it checks the displaced identity/revision and retains unexpected
objects, including links or trees, without following or removing them.

The recovery directory is `.midnight-recovery-<random>` beneath the destination
folder. Windows uses a protected owner-only DACL; Linux requires owned private
recovery and a parent protected against other-user name replacement. These are
not isolation from root/administrators or malicious processes of the same user.
An error after namespace mutation is a structured outcome, never a request to
delete recovery data during cleanup:

- `Published`: no-replace publication completed.
- `PublishedDurabilityUnconfirmed`: publication happened, but a durability check
  failed. Keep the target; this does not authorize Move source deletion.
- `Replaced`: verified destination published and original retained.
- `RecoveryRequired`: an actual mutation occurred, with an optional published
  path, retained recovery objects and the failure. This is not complete success.
  An unknown post-mutation identity remains `None`, never an invented identity.

Before mutation, an ordinary error leaves the existing target untouched. After
recovery ownership transfers, Drop closes handles but never removes retained
objects. No implicit rollback or recovery garbage collection is performed.

Jobs shows the exact recovery count and the first 16 paths. Job history contains
the latest 16 completed jobs and is not persisted across restart; copy the job
details while available. The files themselves remain on disk until explicitly
removed. This is not a persistent recovery catalogue or an automatic Undo action.

## Bounded regular-file Recoverable Move

Explicit Move uses the same FIFO, admission, mutation exclusion, cancellation and
completion receipts as Copy. F6 retains the captured native selection and other
pane destination in a typed Move draft; Cut publishes native Move clipboard
intent. Pasting a Move clipboard opens confirmation. `ClipboardMoveRequest` keeps
an `Arc<FileClipboard>` so confirming or retrying never clones raw paths on the
UI; `CopyInput` unwraps/clones it only in blocking preparation. Ordinary Copy and
archive Copy keep their existing routes. Native formats retain raw paths; no
shell parses file names.

The first scope accepts at most 64 ordinary file roots, with a conservative 4 MiB
reservation for source/capture/stage/proof/receipt paths. The entire batch is
rejected before publication for a directory, link, virtual source, duplicate
native identity, unsupported recovery/proof capability or exceeded budget.
Preparation retains every native source capture authority and empty stage before
copying; failure cleans only its owned stages on a worker. Windows source DELETE
authority is obtained upfront, separately from Copy's unchanged read-only pin.
Protected source parents and qualified local NTFS/ext semantics are mandatory;
unsupported parents suggest Copy, never permission weakening or silent fallback.

Optional contracts default to Unsupported:

| Contract | Meaning |
| --- | --- |
| `prepare_move_source` | Retain the actual source reader plus opaque root capture authority, without changing source names. |
| `prepare_move_destination` | Qualify retained stage read and final-name proof support before copying. |
| `rewind_for_validation` | Rewind the existing source/stage file object and obtain a fresh handle-only revision guard; no old source pathname reopen. |
| `revalidate_publication` | Check held destination directory/final basename against the exact held stage identity, without following replacement links. |
| `TransferMoveCapture::capture` | Retain the root under its original raw basename in private sibling recovery. Structured Captured/Restored/RecoveryRequired facts describe actual mutation; no inode is unlinked. |

Only completely successful Published/Replaced copies yield `VerifiedFileProof`.
The user-requested Move order is per file: copy and independently verify one
file, validate its retained source/destination SHA-256 and required metadata,
capture that source, establish native writer quiescence, rehash both objects,
then explicitly remove that source before starting the next copy. Each full
proof pass uses one shared 256 KiB buffer; source and destination revisions from
the final reads remain guarded through the removal gate. Earlier destination
identities stay protected from later replacement, without retaining their files.

`TransferMoveCapture::preflight_finalization` refuses unsupported removal before
publication. `prepare_finalization` requires a matching successful capture and
returns an opaque `TransferMoveFinalizer`; its `revalidate` checks the retained
object/recovery binding, and `finalize` reports actual Removed/Retained facts.
Drop never removes recovery. The first removal scope is Windows local fixed NTFS, single-link native
regular files. Windows refuses readonly files and any named alternate
data stream, including empty streams that were not copied and verified.

Linux source finalization is currently Unsupported before publication, with an
explicit suggestion to use Copy. A read-lease probe detects existing writable
handles/mappings, but cannot exclude an old-name open already in progress when
capture renames the source. Holding a lease does not solve arbitrary pauses past
its forced-break timeout. Neither advisory locks nor a revision-to-unlink race
is silently accepted as writer exclusion. Existing Linux Copy, capture and proof
primitives remain; no reachable source-unlink implementation is enabled.

On Windows, a Read oplock probe on an asynchronous exact-object `ReOpenFile`
handle rejects preexisting writable mapped sections. Its pending IO is cancelled
and drained before buffers disappear; the original source handle continues to
deny write sharing. Removal uses normal exact-handle disposition with image
section checks, without POSIX semantics or readonly overrides. All application
source/proof/finalizer owners close before the next file. External readers can
delay storage reclamation; parent-entry power-loss durability is not promised.

A skip retains that file. Failure or cancellation stops unfinished Move work;
earlier successful moves remain completed, and later source names stay in place.
A failure after capture retains the original in recovery. No destination rollback
is attempted. Progress reports actual removal/delete-pending/durability counts,
remaining recovery objects, first16 raw recovery mappings and at most64 source
parents for refresh. `.midnight-recovery-<random>/<original raw basename>` retains
local recovery context, without an automatic recovery catalogue or journal.
Empty recovery directories may remain. Physical freed space is never measured or
promised: open readers, mappings and filesystem allocation can delay reclamation.

Same-device Move still performs the full content proof instead of a rename-only
shortcut. Directories, hardlinks, links, virtual providers and broader metadata
remain separate unfinished scope. No phase promises an atomic snapshot or batch
rollback under arbitrary concurrent mutation.

## Required modified time

Existing requests and constructors remain `ContentOnly`. An explicit
`Command::EnqueueTransferWithMetadata(Box<MetadataTransferRequest>)` selects
`TransferMetadataPolicy::RequireModifiedTime` for native paths, captured panel
selection or shared clipboard input, with an explicit Copy/RecoverableMove intent.
The first slice accepts at most 64 distinct native regular-file roots within the
existing 4 MiB proof-path reservation. Directories, links, virtual sources and
unsupported providers are refused without a policy downgrade. The UI option is
described separately in [transfer options](TRANSFER_OPTIONS.md).

All source metadata pins and destination staging capabilities are prepared before
content writes. Source pins capture a consistent native modified time and
revision before any source data read, retaining the existing file object through
`Arc<File>`. This adds no source write authority or pathname reopen. Native read
errors propagate; continuously changing metadata is refused after bounded
capture attempts. Existing stages and constructors retain their original rights;
Windows requests WRITE_ATTRIBUTES only on newly owned required-metadata stages.

The existing source SHA-256 and full independent destination readback complete
first. The source metadata pin is revalidated; modified time is set on the exact
unpublished stage, synchronized and read back exactly, then the source pin is
checked again. Readback has no rounding tolerance. A metadata failure cannot
publish that file or create a Move proof. Before publication, both the source pin
and stage time are checked again; validation errors are terminal. A late conflict
reuses this same verified stage through existing replacement preparation.

Preflight establishes kinds and capabilities, not that every source timestamp is
representable by every destination. Value-specific destination precision failures
are discovered after data verification on unpublished stages. Earlier completed
files may remain published; a failure at this stage prevents removal of that source
and stops subsequent Move work. There is no silent switch to content-only copying.

Move proofs retain the metadata pin and expected modified time alongside their
existing source/stage/digest/length. Full source/destination hashes, fresh native
read guards and exact final-name checks remain mandatory. Expected source and
destination times are checked during each proof and before captures. After our
source rename, the old revision token is not compared to its changed ctime;
fresh proof guards and the retained source-handle timestamp read are used.
Published targets are never corrected to conceal a mismatch. A mismatch after
capture reports an error and retains the original bytes in recovery.

Windows preserves native 100 ns values and explicitly refuses reserved/unsettable
FILETIME values; Linux preserves native nanoseconds, including supported negative
epoch values, with access time left to its existing policy. Timestamp readback is
verified after native synchronization, and native tests also check the final
closed file. Current native filesystem coverage is NTFS on Windows and ext on
Linux; this does not broaden remote commit or power-loss guarantees. Native
version checks are still not immutable snapshots.

## Current limits and next stage

Copy preserves verified content, with explicit required modified time as above.
Link copying and remaining times/attributes,
permissions, ACLs, extended attributes and alternate streams need their declared
metadata capability matrix. Existing source link/special entries are refused.
Recursive directory Move remains a separate stage: root metadata cannot prove
descendant content/namespace closure. It requires a complete bounded child proof
manifest and another reviewed barrier before root capture. Permanent source
reclamation is also separate; skips, failure, cancellation or recovery/durability uncertainty cannot satisfy
that barrier. Nothing in this API permanently deletes a source inode.
