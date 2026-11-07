# Recoverable Copy and Directory Merge

> Implement with `superpowers:executing-plans`, coordinated native adapter work
> and independent review. Authorized by the user's implement-all goal.

**Goal:** Complete practical native overwrite and directory merge while retaining
independent checksum verification and preserving every displaced object.

**Architecture:** Extend the existing verified-copy state machine and versioned
conflicts. Native adapters own target pins, protected recovery objects and commit
facts; UI carries choices only. Recovery is a separate explicitly selected policy,
not a silent fallback from strong conditional replacement.

**Spec:** Main design section 25 and `2026-11-07-completion-goal.md`.
Detailed native feasibility report: `_build/goal/transfers/contract-review.md`;
frozen working signatures: `_build/goal/transfers/api.md`.

## Global constraints

SHA-256 source/readback, 256 KiB transfer steps, backend-only non-UI work,
raw native paths, no target deletion before verification, no implicit link
following, protected recovery ownership and truthful publication/partial outcomes.
This increment never deletes sources; the following Move stage adds its complete
batch verification/removal barrier. Existing no-replace behavior remains default.

## Review focus

- Target replacement after the last check must preserve the unexpected object.
- A verified no-replace stage that encounters a late collision must be reusable
  for a newly consented replacement without rereading a consumed source.
- Rename/Skip/Cancel cannot retain stale target authority or delete recovery data.
- Merge and replace-all scopes are independent and limited to the current job.
- Source=target aliases, changed target pins, queue backpressure and stale responses
  must not authorize unintended writes.

### Task 1: Native primitives and contract

Owner: `goal_transfer_contract`; core `transfer.rs`, per-OS `transfer*` and native
tests. Add optional target pins/capabilities, explicit merge, stage replacement
preparation/commit and structured recovery receipts. Linux exchanges a verified
stage with the final entry and retains displaced data; Windows captures the
guarded original into private recovery then publishes no-replace, with a declared
namespace gap. No automatic rollback over a concurrent new file.

- [ ] Add native tests for ordinary merge, stale pins, original retention, immediate
  commit races, cancellation, explicit Unsupported and stage-path substitution.
- [ ] Implement and verify Windows and Linux adapters with owned fixtures.

### Task 2: Verified-copy orchestration

Owner: root; `midnight-platform-common/src/transfer.rs` and `transfer_tests.rs`.
Add `prepare_replacement` delegating to the stage and `publish_replace` sharing
the same verified/source-revalidated gate as ordinary publication.

- [ ] Tests first: replacement never invoked before full independent readback;
  corruption/cancellation/source change prevent invocation; partial recovery
  facts survive without allowing another publish or losing owned recovery.
- [ ] Implement the shared gate without copying the hash/readback state machine.
- [ ] Run focused common transfer tests and strict Clippy.

### Task 3: Versioned choices and resumable traversal

Owner: root; core `copy_operation.rs`, engine `copy_worker.rs`, focused helper/test
files under the same directory. Keep native pins only on the worker. Add explicit
recoverable Replace/Replace all and Merge/Merge all choices, capability/type/alias
checks, fresh conflicts after stale target detection, and bounded recovery display
samples with an exact recovery count.

- [ ] Test per-job scope, independent merge/replace choices, old response refusal,
  changed targets, late publish collision, same-file aliases and no source removal.
- [ ] Integrate native receipts; never classify RecoveryRequired as full success.
- [ ] Run existing queue/copy/archive tests plus new cases.

### Task 4: Conflict UI and delivery

Owner: root; `copy-controls.slint`, `copy_presentation.rs`, presenter `copies.rs`,
app copy bindings and Jobs detail. Explain retained originals in the overwrite
choice, show separate source/target metadata and supported actions, preserve
deferred decisions and compact/movable keyboard-accessible dialogs.

- [ ] Test choice routing and actual Enter/Escape/focus without loading user settings.
- [ ] Document native guarantees, recovery locations and remaining Move work.
- [ ] Review, run relevant Windows/Linux checks, build/deploy/register release,
  verify actual UI, and commit/push the working increment before subsequent stages.
