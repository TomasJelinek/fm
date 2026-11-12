# Native inline-tree Delete verification — 2026-11-12

## Implemented boundary

Native tree Delete now has an immutable typed capture (`TreeDeleteRequest`,
`TreeDeleteCapture`, `CapturedDeleteSources`) and `PrepareCapturedDelete` admission.
The exact acknowledged projection, selection allocation/revision, request ID and
optional focused occurrence determine targets on the tree worker. A nonempty
selection wins over focus; parent/context-only/foreign/stale rows grant no authority.

The worker checks every selected provider before deduplication. Unsupported/virtual
sources reject the entire capture. Selected descendants of selected ordinary
folders are suppressed using occurrence parent IDs, preserving topmost display
order. Distinct hard-link names remain distinct namespace targets. A covering
folder must still prepare as an ordinary Directory, otherwise preparation fails.

Native paths come from retained provider targets. Display/source-directory text is
only presentation and refresh context. Ordinary platform MutationService preparation
pins the remaining native identities before preview. The captured mode, stamp,
request and count are frozen at operation admission and checked around preparation;
wrong counts or changed source contracts fail before confirmation.

The existing operation worker owns confirmation, Trash/permanent mode, FIFO gates,
Copy predecessors, pause/cancel, discovery, errors and progress. The existing flat
PrepareOperation checks still require its exact native listing and immediate-parent
entries. There is no new Windows/Linux deletion API, shell deletion command, Trash
to permanent fallback, or changed native mutation policy. Rename remains a separate
unimplemented tree consumer.

## Lifetime and mutable-subtree contract

The four shared tree capture permits, common 4096-target/4 MiB captured-context
limits, two preparation workers and 32-operation queue bound remain in force.
Consumer ancestry sets are independently bounded by the admitted target count.
Native targets are declared before the retained capture in Job, so pins close before
provider registrations and capture permits are released. Rejected/replayed inputs,
queue-input disposal and cancellation use backend retirement, including an async-abort
ownership guard for those inputs. The existing forced-shutdown ownership gap for
already prepared/running jobs remains separately tracked as LIFE-01 in the
[backlog](../IMPLEMENTATION_BACKLOG.md); this work does not claim that gap is fixed.

Capture is not a filesystem snapshot. Native identity starts at ordinary preparation.
The selected topmost roots are pinned; their recursively visited descendants are
not frozen between capture, confirmation and execution. A child moved out of the
selected tree is no longer recursively removed; a replacement directory link is
removed as a link and its target is not traversed. A one-time ancestry inspection
would not strengthen that execution-time guarantee, so no such cosmetic proof was
introduced.

## Peer-review resolution

Initial review inferred inline link expansion from NativeProvider::list accepting
DirectoryLink. The attempted real-session regression disproved that premise:
DirectoryTree::request/disclosure reject link expansion, attach_container requires
File, and core TreeProjection rejects DirectoryLink disclosure. No current
production overlap bug was demonstrated.

The real-session test now asserts that existing expansion fence. A separate pure
ancestry-policy test protects future provider shapes: an explicitly selected ancestor
and descendant separated by a link/non-directory boundary must be rejected before
native preparation, with a message to delete the targets separately. Ordinary
folder ancestry still deduplicates. The defensive policy does not enable link
expansion. Independent review otherwise found no actionable capture/admission,
correlation, scope-check or target-before-provider retirement defect.

## Evidence

All logs are retained under `_build/qa/tree-delete-20261112`.

- `capture-red.log`: deliberately unsupported capture stub: 6 expected failures,
  1 existing admission-guard pass.
- `operation-red.log`: implemented capture passed; missing operation preparation
  produced 2 expected missing-preview failures, 9 passes.
- `backend-green.log`: 14 focused Windows tests passed, including real native
  nested Unicode folder deletion after confirmation, cancellation before confirmation,
  and selected junction removal preserving the target.
- `link-boundary-red.log`: 19 additional/previous tests passed. The sole failed test
  attempted an impossible current link expansion; this was a fixture/premise error,
  not evidence of a live Delete defect. The corrected test asserts the fence.
- `ancestry-policy-red.log`: pure defensive policy regression failed as expected;
  ordinary ancestry passed (1 failed, 1 passed). The policy fix is now implemented.

Final focused qualification:

- `integrated-tree-units.log`: all 23 Windows backend tests and all 8 presenter
  tree Delete tests passed after the defensive ancestry policy fix.
- `native-ui-green.log`: the real Slint-to-Engine native Delete test passed. It
  confirms deletion and preserves the other expanded branch through reconciliation.
  The earlier `native-ui.log` failure used the Loading numeric disclosure value as
  an Expanded expectation; its fixture now asserts the expanded boolean and a
  retained sibling row. Native deletion and content assertions had already passed.
- `linux-backend-green.log`: all 23 tests passed on native Ubuntu-24.04/WSL,
  including symlink target preservation and post-preview identity/link replacements.
  `linux-backend-first.log` preserves the earlier 22-pass/1-failure run: its shared
  fixture incorrectly required Windows DirectoryLink for a Linux no-follow Symlink.
  The corrected assertion accepts both native link representations; no production
  deletion change was needed.
- `linux-clippy.log`: strict core/engine library Clippy passed with `-D warnings`.
  The first lint run (`linux-clippy-first.log`) found duplicate inclusion of the
  retirement helper. It now exists once as the crate-level worker_retirement module,
  imported by Copy and mutation operations; the helper's behavior is unchanged.
- Owned Rust sources passed LF/100 KB checks and `git diff --check`.

The final broader Windows batch passed all 2,335 enabled affected unit tests, with
27 existing ignored tests and no failures/filtered cases (`full-affected-units-green.log`).
The engine portion passed 856 tests with 7 existing ignored cases. All 26 selected
integration checks passed (`final-integrations.log`); strict all-target Clippy passed
(`final-clippy.log`), and `debug-build.jsonl` ends with build-finished success=true.
These are the affected package union and selected integrations, not every independent
plugin/workspace target; see the [integration report](2026-11-12-tree-delete-integration.md).

The earlier 13 diagnostic-storage failures came from a protected TEMP spool owned
by CodexSandboxOffline being used by the authorized tjelinek runner. All 13 unchanged
cases passed with a fresh same-user TEMP. No ACL, product code, security policy or
assertion was weakened; the old protected directory was left untouched.

Final shared test-fixture follow-up: Windows all-target lint found the same platform
link-test helper included twice. Root moved its cfg-selected module to one engine
crate-level native_test_links module; both test consumers now import it. The actual
fixture behavior and assertions are unchanged. Native Linux qualification after
that wiring change passed all 23 tree Delete tests (`shared-fixture-linux-tree-delete.log`),
all 3 native folder-size tests (`shared-fixture-linux-folder-sizes.log`), and strict
engine `--lib --tests` Clippy with `-D warnings` (`shared-fixture-linux-clippy.log`).
This follow-up used a fresh `/tmp/fm-shared-links.*/_build/tmp` fixture root.

All destructive fixtures use generated test-owned directories under QA TEMP or a
native Linux temporary `_build` directory. The final Linux runner uses its own
random `/tmp/fm-tree-delete.*/_build/tmp` directory (recorded in linux-fixture-root.txt).
Actual Trash was not invoked; both immutable modes are checked through the
mutation-service boundary. No commit was made by this implementation agent.
