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
The copy buffer is released; each proof retains exact source and stage, immutable
SHA-256/length and publication identity without retaining a byte buffer. Earlier
proof targets are protected against replacement by later files in the same job.
Any skip, failure, cancellation, partial recovery or required-durability failure
blocks all source capture.

After every copy completes, one shared 256 KiB buffer rereads all held sources and
all held destinations independently against their original digest/length. Fresh
native revisions and final-name bindings supplement these hashes. All capture
authorities are revalidated; only this private whole-batch worker barrier permits
capture, one root per step. Before each capture, remaining authorities and all
destination bindings are checked. Another complete proof pass then reads the
same original handles despite their old names disappearing. A late writer's new
bytes remain in source recovery and a digest mismatch reports partial/recovery
status; verified destinations are never erased to emulate rollback.

Cancellation before capture leaves every source name unchanged. Cancellation
after one capture retains that original and leaves remaining source names in
place. Progress records actual captured-name and recovery counts, first 16 raw
source/recovery/identity mappings and at most 64 captured source parents to
refresh. The `.midnight-recovery-<random>/<original raw basename>` layout preserves
local source-name context after restart, but no persistent recovery catalogue or
automatic restoration is implemented. Originals remain on disk; source space is
not reclaimed and permanent deletion is never claimed.

Native revisions are not snapshots: same-size updates in one timestamp quantum
can retain both modification/change indicators. Full hashes remain required.
Linux no-replace capture plus identity inspection/restoration is not compare-ID
unlink. Same-device Move never shortcuts the full verification barrier.

## Current limits and next stage

Copy preserves verified content. Link copying and complete times/attributes,
permissions, ACLs, extended attributes and alternate streams need their declared
metadata capability matrix. Existing source link/special entries are refused.
Recursive directory Move remains a separate stage: root metadata cannot prove
descendant content/namespace closure. It requires a complete bounded child proof
manifest and another reviewed barrier before root capture. Permanent source
reclamation is also separate; skips, failure, cancellation or recovery/durability uncertainty cannot satisfy
that barrier. Nothing in this API permanently deletes a source inode.
