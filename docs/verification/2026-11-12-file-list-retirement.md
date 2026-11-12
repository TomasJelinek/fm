# Make File List native ownership

Baseline: `b3fa92e`. Scope: Make File List actor, output and generator ownership;
shared startup integration is part of the enclosing LIFE-01 batch. This note does
not claim that every engine actor has been migrated.

## Reproduced failures

The pre-fix Windows test build ran three deterministic regressions: **0 passed,
3 failed**, 947 filtered, 0.05 s. Evidence:
`_build/qa/file-list-lifetime-20261112/windows-red.log`.

- A real native destination target retained after a conflict was finally dropped
  on the executor on Cancel, replacement, shutdown and Controller Drop. The test
  observed all four target destructors and checked the original file bytes.
- Confirmation was enqueued, then cancelled and aborted before its first poll.
  Its retained native destination pin was finally dropped on the executor.
- A real generated temporary Spool completed on the sole blocking worker. A later
  worker marker proved completion without consuming the first JoinHandle. Dropping
  that unconsumed result on the caller closed its Spool on the caller.

The Spool observation is a test-only field after the real native Spool; it does
not substitute a fake generated source for the native resource.

## Ownership and bound

The actor now enforces **six retained logical generations/outputs**. This is an
explicit admission limit, separate from four visible jobs and two active workers.
Completed Viewer/Clipboard output may outlive its job or originating window, so
those earlier limits alone did not bound native output ownership.

Before cancelling or replacing an accepted generation, admission acquires its
logical token and two cleanup leases. One lease guards Job, the other is reserved
for its sole generated Spool. Job and generated content share the logical token;
confirmation reuses the existing source. Both payloads keep their token through
actual native destruction, including unwinding from one destructor panic. This
requires **12 startup retirement slots and no additional waiter**. Services use
the existing TaskLifetime anchor rather than an extra cleanup slot.

The payload order is intentional:

- Job owns pending destination pins, generated content and captured size state
  before its service anchor and logical token. Captured native size state can
  retain warning-report storage and is included in this guard.
- A pending output owns its target/directory pins before its generated source;
  the destination pin owns the target before the directory.
- Generated content owns the sealed Spool before the logical token. The guarded
  content has no Job backreference, so external output creates no ownership cycle.

Confirmation leaves Pending in the guarded Job until native execution begins.
No raw pending native pin crosses an unpolled async handoff. Both the outer task
and blocking closure retain the original service anchor. The worker admission
permit is shared by native execution and async delivery: cancellation cannot free
it while native work remains active, and a completed send cannot accumulate
outside the two-worker bound. Controller Drop cancels and releases guarded jobs.
The fixed retirement worker handles final cleanup without requiring a live Tokio
runtime; there is no thread per Drop, inline executor cleanup or unbounded queue.

If six accepted generations/outputs remain retained, admission preserves the old
generation and all accepted output. It reports: "File list outputs are still in
use. Close an output and try again." It does not close a user output to admit work.
Verified append/replacement, explicit confirmation, cancellation, source identity,
public/plugin interfaces and output byte limits are unchanged.

## Archive size binding

Peer review found a separate pre-adoption seam: size binding cloned a native
archive mount under the Snapshots lock, then dropped the temporary clone on the
dispatcher after a second lookup. Concurrent root retirement could make it the
last native owner before guarded Job construction.

`Captured::bind` now calls `Published::snapshot_for` with the mount borrowed under
the original Snapshots lock. The helper takes no lock and creates no strong mount
owner; it preserves weak pointer identity and the numeric mount check. It returns
only the immutable size snapshot. There is no new nested lock order. A semantic
regression retains the accepted size map across root retirement and rejects a
replacement mount with the same numeric id; it does not assert Arc owner counts.

## Qualification

Additional regressions exercise six externally retained outputs after job
cancellation, refusal without replacing the prior generation or closing output,
re-admission after final output cleanup, and admission/lease retention during
blocked and panicking generated-content cleanup. Existing File List tests cover
output bytes, verified append/replacement, cancellation and size-cache behavior.

The frozen implementation passed its first coherent Linux File List run: **30
passed, 0 failed**, 923 filtered, 0.19 s. The suite includes the three original RED
cases and the additional admission, unwind and archive-binding regressions.
Evidence: `_build/qa/network-provider-lifetime-20261112/linux-final-lib-file_list.log`.
Its test artifact SHA-256 is
`6c9522d8a4bc91a5819d6f44372c45245e5f30198d9ba4560a4f959d8bd03722`.

The enclosing Windows gate passed the complete engine library suite: **956
passed, 7 existing ignored, 0 failed**, 6.55 s, including the 30 File List cases.
Seven directly affected integration targets passed **46 tests**. Strict engine
all-target Clippy passed in 19.77 s. Logs:
`_build/qa/remaining-native-results-20261112/windows-final-tests.log` and
`windows-clippy.log`. The app debug build passed in 1m20s (`debug-build.log`);
its pre-existing third-party femtovg warnings are separate from strict engine lint.
Linux strict engine all-target Clippy also passed in 1m32s; the driver completed
with status 0. Evidence: `network-provider-lifetime-20261112/linux-final-clippy.log`
and `linux-final-status.txt` under `_build/qa/`. Its seven affected integration
targets likewise passed 46 tests.

Independent source review accepted the six-owner/two-lease bound, Job/Spool field
ordering, native-plus-delivery permit lifetime and borrowed archive lookup. The
review reported no remaining actionable issue and did not run the tests.
