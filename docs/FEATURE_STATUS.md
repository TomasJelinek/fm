# Feature status for 0.3.0

This is an early preview. A design document is not evidence that a feature is available.
The resumed full-feature goal, role assignments, requirements/evidence backlog and
corrections to stale stage references are tracked in [Active work](ACTIVE_WORK.md).
Implementation presence, recorded targeted results and native/release qualification
are separate states; the latter is not implied by a checked plan or green subset.

## Current integration checkpoint — 18 November 2026

Main and the viewer work branch each have independent verification records. See
[main's deep review and the viewer work-branch checkpoint](ACTIVE_WORK.md#current-checkpoint)
for their test scope and open gates. The results below describe the source or
artifacts used in each run. The merged source still needs its own verification.
Repeated HTML Find and native Source validation remain open.

Private ABI 1.4 locator API v2 compiled without diagnostics. Cargo output
records 47 passing test bodies, including all 14 new locator cases. The initial
launcher returned 125 after observing a child still present and saved no raw
Cargo exit. Independent checks later confirmed that the exact child, Cargo and
supervisor identities had exited. [API run outcome](../_build/verification/viewer-refresh-20261117/build/locator-listing-wire-v2-validation/observed-outcome.json) ·
[Cargo output](../_build/verification/viewer-refresh-20261117/build/locator-listing-wire-v2-validation/cargo.stdout.log)

The matching private SDK v1 slice compiled cleanly and recorded Cargo and
wrapper exits of 0. All 58 library tests passed, including nine locator cases.
All 100 source checks showed zero drift. The two runs cover the locator API
and SDK cursor mechanics. The host patch is in progress, and installed provider
and unlimited tree UI acceptance remain open. [SDK exit receipt](../_build/verification/viewer-refresh-20261117/build/locator-listing-sdk-v1-validation/actual-exit.json) ·
[SDK run handoff](../_build/verification/viewer-refresh-20261117/build/locator-listing-sdk-v1-validation/handoff.json)

In the private filename color migration, three baseline tests failed solely
because the loaded document and text rules used the old gray. After the in-place
migration, all 27 `filename_colors` tests passed. The tests verify the loaded
rules and preservation of customized colors. The two tested files are now
integrated into the canonical working branch. Their hashes match the private
passing inputs (`a26e1266` for production and `413c88e3` for tests). Formatting
and diff checks passed at integration. The running and distributed executable
still predates the migration. Installation and visual acceptance remain open.
[Independent color migration review](../_build/verification/viewer-refresh-20261117/root-document-color-migration-v1-actual-red3-green27-review.json) ·
[Source integration record](../_build/verification/viewer-refresh-20261117/coordination.md)

The private Source frontier repair rebuilt with zero warnings or errors. All 12
headless tests passed, and the same real installed-provider case passed once. The
preserved baseline had failed at its final assertion with position generation 2
where it required 1. [Headless results](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-repair-v1-headless/HANDOFF.json) ·
[Installed-provider pass](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-repair-v1-native/handoff.json) ·
[Baseline failure](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-provider-red-v1-native/handoff.json)

The separate private corpus run reached C++ ACK 21 and failed waiting for ACK 22
at target `0.56375`, before rapid C++ or the Lua and YAML cases. Both pages
applied and completed. The first window `[42563, 43236)` began at mapped position
`0.56451339`, and the second `[42410, 42921)` ended at `0.56307317`, leaving the
target between their mapped ranges. The result was **0 passed, 1 failed,
0 ignored** with exit 101 in 64.39 seconds. Each guard checked 7,116 inputs with
zero drift, and the test process and observed helpers were gone at closure.
App acceptance and highlighting validation remain open. [Corpus result](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-repair-v1-corpus/source-frontier-corpus-1/result.json) ·
[Corpus trace](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-repair-v1-corpus/source-frontier-corpus-1/test.log)

Source coverage v1 is source-ready and held at review. The open issues are
exact fallback when the map is absent or at the file end, queued wheel deltas
that can move the frame used to acknowledge a request, and a failure path that
can lose newer queued input. The retained recovery path also needs an exact
test. Compilation and runtime validation remain pending. [Source-ready receipt](../_build/verification/viewer-refresh-20261117/viewer/source-coverage-recovery-v1/source-ready.json) ·
[Review hold](../_build/verification/viewer-refresh-20261117/coordination.md)

**Viewer helper v2:** passed **22 exact tests / 0 failed /
0 ignored** on source epoch `e0df…`: the original palette case, two teardown
regressions and 19 existing helper cases. Each ran separately under the unchanged
300-second bound. The compatible seven-target compile exited 0 with no warnings or
errors; six executables were Fresh and only Viewer UI was rebuilt (`934254c5…`).
Root independently verified 66 raw original/copy pairs, 44 invocations, 2,288 clean
source guards and seven executable identities. Parent closure additionally checked
77 pairs and all 45 owned PIDs absent. This accepts the affected fixture boundary.
The earlier v1 timeout and original closed panic remain preserved; their original
stop cause is not explained. See the [independent helper review](../_build/verification/viewer-refresh-20261117/root-palette-fixture-validity-v2-runtime22-review.json).

**Native maximum-state boundary passed:** the genuine frozen-C payload ran the
exact MAX case **1 pass / 0 failed / 0 ignored** in 7.95s. The first attempt failed
during setup because the wrapper had not created `native-work` before fixture
canonicalization. A new immutable adapter adds only the MAX work-directory mkdir;
application sources and the C bundle are unchanged, and the failed attempt is
preserved. Root verified 24 raw copy pairs, all 2,288 current source hashes and
all 2,487 frozen-C files, including identities, sizes, mtimes and exact membership.
The actual case proved ZIP availability, four Viewer and two Compare frames,
settings/navigation progress in 7ms, joined pools and reader/clipboard cleanup.
This does not prove all 20 configured workers simultaneously busy or the spare
image decoder Ready, and does not establish an old-source genuine pressure RED.
See the [native MAX review](../_build/verification/viewer-refresh-20261117/root-native-maximum-corrected-runtime-review-v1.json).

**Native Source remains RED:** the actual C++ native case failed **0/1/0,
exit 101**, in libtest-reported 61.56s at `source_native_acceptance_tests.rs:742`:
the required Source frame did not arrive before the shared absolute 45s deadline.
The outer 300s watchdog did not fire. Initial and closing 4,807-entry guards have
zero drift; owned cleanup passed with no processes remaining and private C++
metadata unchanged. The metadata records the expected automatic Source mode 2
route, without a first-destination report; the missing phase is unknown. This
does not qualify Source painting. The one-file, failure-only DIAG 1 metadata change
is now canonical (`6c8e…` to `410e…`, source epoch `e39e…`), preserving all 34
original assertions, the 45s waits and production behavior; pinned format exited 0.
The matching seven-target diagnostic compile now passed exit 0 with zero
diagnostics. Root approved epoch reuse (`501a7…`): the new harness consumes the
test-fixture change, while normal/C inputs exclude the two approved test-only
changes. Producer epoch `e39e…` and frozen-C epoch `9297…` remain distinct. The later
native DIAG 1 run failed **0 passed / 1 failed / 0 ignored, exit 101**, in 61.68s,
without firing the outer watchdog. At 45,000ms it was waiting for control ACK
step 4 in C++ trial 0, target 0.82, wanted sequence 4: expected Positions context
generation 1/sequence[3,0], last frame generation 2/sequence[0,0], with four frames
observed. These are **Positions context generations**, not Worker-open generations
or proof of a file reopen. The specific production cause remains unknown; Viewer
is tracing position refinement and viewport completion. Root verified 41 raw copy
pairs, 4,835 adapter pre/post guards plus 2,288 source and 2,498 extra guards, all
clean; owned cleanup passed and private C++ metadata was unchanged. No private C++
content was copied or hashed by that review. The original failures remain
preserved; this is diagnostic evidence, without native acceptance or a blind
retry. See the [DIAG 1 failure review](../_build/verification/viewer-refresh-20261117/root-source-native-diag1-failure-evidence-review.json),
[reuse approval](../_build/verification/viewer-refresh-20261117/root-source-native-diag1-epoch-reuse-acceptance-v1.json)
and [DIAG 1 promotion](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/source-native-failure-observation-v1/canonical-source-ready.json).
See the [native result](../_build/verification/viewer-refresh-20261117/build/native-current-source-e0df-prep/source-cpp-native-1/result.json)
and [raw assertion](../_build/verification/viewer-refresh-20261117/build/native-current-source-e0df-prep/source-cpp-native-1/test.log).

The private Source mapped-prefix change now has actual **3 behavioral RED →
18 GREEN** evidence (six new plus 12 existing position cases). Tests `4edac…` are
identical between RED production `07f…` and GREEN `25af…`. Root independently
verified 119 original/copy pairs, 42 invocations, both preserved executables,
feature/profile variants and all 1,099 private source hashes; only production
`worker_position.rs` changed. The original automatic RED classifier's false flag
is preserved: the raw expected-Reload panic is a real invariant failure before
Completed, while GREEN reaches Completed and pixel assertions. These private
results retain their source epoch; the subsequent canonical promotion below
still needs its own integration and native validation. See the
[runtime review](../_build/verification/viewer-refresh-20261117/root-source-mapped-prefix-runtime-red3-green18-review-v1.json),
[RED admission](../_build/verification/viewer-refresh-20261117/root-source-mapped-prefix-behavioral-red-admission-v1.json)
and [GREEN source review](../_build/verification/viewer-refresh-20261117/root-source-mapped-prefix-green-source-review-v1.json).

Five existing retained controls already passed on preserved GREEN executable
`1e5…`; preserve their raw evidence and carry them and the 18 position cases only
with explicit unchanged-source/feature/profile/production closure proof. A new
single retained Source test (`d394…`, 33,421B) now passed exactly once on `7ca…`
in 0.09s, preserving the old 27,335-byte test prefix and production 25af. The five
controls and 18 position cases were carried with closure proof and not rerun.
Root independently reviewed 59 original/copy pairs, all 1,099 v2 sources and
feature/profile equivalence. The fixture proves corrected retained pixels and a
numbered gutter against the CPU reference; it does not prove a live Session,
actual provider retained-line allowance or installed rapid dragging. See the
[retained runtime review](../_build/verification/viewer-refresh-20261117/root-source-mapped-prefix-retained-runtime6-review-v1.json)
and [v2 source review](../_build/verification/viewer-refresh-20261117/root-source-mapped-prefix-retained-v2-source-review.json).

**Source production is now canonical:** after actual Build release, Root promoted
`worker_position.rs` to 25af and its tests to d394, with 2,288 clean guards and
format 0. Full source snapshot is `c500b7e0…`; the prior normal 673/Main artifacts
were unchanged at promotion, and C9297 is not rebound. Build is assigned the
matching new normal producer, complete new C installation, current-canonical
24-case targeted integration and original Source 34-oracle actual-corpus native
validation. Those results remain pending; the old normal/C product does not
qualify changed production. No native acceptance, publishing or full-goal closure
is claimed. See the [two-file canonical promotion](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/source-mapped-prefix-v2/canonical-source-ready.json).

**Native Office remains RED:** the HTML captured-root online roundtrip failed
with a native-stage timeout (22.58s, exit 101), while the independent offline XLSX
case failed after catalog navigation: real CEF `Find(Workbook sheets)` expected 1
and returned 0 (7.5s, exit 101). The third failure took 24.83s during later navigation:
initial Alpha, retained Alpha after refusal and new-revision Alpha are recorded,
but Beta and the exact later subphase are unproved. The last two cases now passed:
parent change after resolve/before data (5.88s), and pending-resource close/
cancellation cleanup (3.73s). **All five Office cases executed: 2 passed /
3 failed / 0 unrun**, retaining their distinct harness epochs against the same
frozen-C/Office 710 runtime. The last two have clean 4,805-entry adapter guards,
2,288/2,498 parent checks and owned closure. Earlier content/capture proofs retain
their scopes; a rendered HTML screenshot does not establish Find/new-paint proof.
The one-file Office phase observer is now canonical after the final Build release:
`office_preview_native_tests.rs` changed from `710bea…` to `50eb…` (96,406B), with
2,288 clean pre/post guards, format 0, normal 673/Main unchanged and no production
change. The current full source snapshot is `2ae892…`; C9297 is not rebound.
Five preserved readbacks retain the prior 0bb executable, depfile, producer and
Office 710 fixture evidence. The separate compatible seven-target diagnostic
producer now compiled exit 0/zero diagnostics, with new Engine harness `3666…`,
normal 673 and source 2ae. Both metadata observations actually failed at their
initial `wait-loaded` phase: HTML 20.29s and XLSX 20.38s, with zero events, requests
and frames. Their cause is unknown; UI is tracing startup and the actual qualified
worker. Both 4,809-entry guards plus 2,288/2,498 companion checks and owned cleanup
passed. These observations are distinct from the original three navigation
failures and do not alter the original five-case 2 PASS/3 FAIL ledger. See the
[separate observation handoff](../_build/verification/viewer-refresh-20261117/build/office-native-phase-observation-v1-native-prep/combined-observation-handoff.json).

The additional HTML diagnostic, attempt `h1`, used the same C9297 installation,
single test and deadlines with archived harness `3666f4e2`. Its profile measured
209 UTF-16 units. Sixteen process snapshots sampled two renderer identities.
Header `(1, 1, 5)`, Loading event 1 and Loaded event 5 arrived. The run recorded
zero captured resource requests, no completed or offset-zero source read and no
retained frame. It failed in the initial `wait-loaded` stage with 0 passed,
1 failed, 0 ignored and exit 101 in 20.83 seconds. Pre- and post-run guards each
checked 2,511 entries with zero drift, including 2,487 install files. No owned
processes or monitor errors remained. The earlier 231-unit attempt received no
header. The scratch path family and run conditions differed, so the cause remains
open. Content and paint acceptance remain open. [Run result](../_build/verification/viewer-refresh-20261117/build/office-native-phase-observation-v1-native-prep/h1/0/result.json) ·
[Profile comparison](../_build/verification/viewer-refresh-20261117/build/office-html-short-profile-v1/profile-comparison.json)

The explicit unconsumed canonical Office 50eb
test-source exception may be bridged while private 710 fixtures and raw guards
remain immutable. Preserve the original 20s stage/300s supervisor/oracles/policy;
do not replay unchanged Office units or the two passing native cases. No overall
Office green or cause fix is claimed. See the [canonical diagnostic promotion](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/office-native-phase-observation-v1/canonical-source-ready.json),
[final two passes](../_build/verification/viewer-refresh-20261117/root-office-final-two-native-pass-review-v1.json),
[third failure](../_build/verification/viewer-refresh-20261117/build/native-office-remaining3-e39-prep/observation-handoff.json),
[observer review](../_build/verification/viewer-refresh-20261117/root-office-native-phase-observation-v1-source-review.json),
[HTML closure](../_build/verification/viewer-refresh-20261117/build/native-current-e0df-execution/office-failure-parent-close.json)
and [XLSX closure](../_build/verification/viewer-refresh-20261117/build/native-current-e0df-execution/office-remaining4-failure-parent-close.json).

**Private ownership work:** PO v3 compiled exit 0 without compiler warnings, then
ran 118 Viewer cases: **116 passed / 2 failed / 0 ignored**. The 11 Host cases are
prior evidence, not new executions. Preserve the real-session timeout assertion
and one-shot mailbox fixture failure in the [v3 handoff](../_build/verification/viewer-refresh-20261117/build/host-viewer-coop-po-v3-qa/green/handoff-failed-v3.json).
Root accepted private PO v4 source: 28 postimages, 41 references and a three-file
delta, retaining 25 v3 rows and the original real-session test prefix. Its atomic
Generated shared-reference release consumes a shared wrapper reference; the
opaque final wrapper remains retained with an unconfirmed error. The mailbox
fixture correction preserves refusal and no-touch assertions. The private v4
runtime now passed **128 distinct Viewer cases / 0 failed / 0 ignored**: focused 12
plus 116 disjoint cases. Root independently checked actual raw names/counts, three
executable identities and 1,387 reconstructed guards. Compilation had no warnings;
this is not full strict Clippy. Host 11 was not replayed and remains prior evidence.
No canonical promotion, final Generated cleanup or full lifecycle qualification
is authorized. See the [v4 runtime review](../_build/verification/viewer-refresh-20261117/root-viewer-po-admission-v4-runtime128-review.json)
and [source review](../_build/verification/viewer-refresh-20261117/root-viewer-po-admission-v4-source-review.json).

**Private Backend 109 compile failed:** the actual baseline compile exited 101
after 614.234s with 14 errors and four warning messages; all 74 planned runtime
tests remain unrun. This is a compilation failure, without behavioral RED or a
normal strict-build result. Five additional private Backend consumer rows are
reserved for the actual missing-field/old-API fixture corrections, preserving the
failed baseline. Follow-on history/idle tests still need a new coherent artifact.
See the [archived baseline](../_build/verification/viewer-refresh-20261117/build/a1-b1-coherent-engine-v1-build-prep/baseline109-handoff.json)
and [five-row reservation](../_build/verification/viewer-refresh-20261117/root-a1-b1-actual-compile-five-consumer-reservation-v1.json).

Private UI collector v2 disarms the setup guard after actual ticket registration
and adds a real terminal-success/prune/slot-reuse regression, preserving all 10
older tests. The actual private Engine v4/collector v2 boundary now passed
**11 exact tests / 0 failed / 0 ignored**, each separately under 300s. Root checked
33 original/copy pairs, 22 invocations, the actual Cargo artifact (`a60aa5ae…`),
all 1,099 private sources and the reconstructed 3,894-row baseline plus six runtime
inputs. The single unconsumed `410e…` test-source exception is explicit and
separate; the original compile/raw drift is preserved. This accepts only the
private boundary, without canonical/full lifecycle acceptance or proof of the
original production failure cause. See the [collector runtime review](../_build/verification/viewer-refresh-20261117/root-viewer-ui-po-collector-v2-runtime11-review.json)
and [source review](../_build/verification/viewer-refresh-20261117/root-viewer-ui-po-collector-v2-source-review.json).
Neither private packet is canonical or full lifecycle acceptance. Perf's history
branch-allocation P2 is assigned to the Backend owner after the coherent B1
checkpoint; no measured improvement is claimed. The cross-graph registered-idle
bridge is accepted for the existing three Backend-owned private files over v3.
The private history/idle follow-on is now source-reviewed (`d0ff…`), with 13 new
tests unrun. Preserve the 109/74 baseline checkpoint; the additive 109+5 composition
needs matching compilation, then 13 new plus 59 disjoint existing direct cases
(72 unique total). These are expectations, not results or a measured Perf gain.
See the [private follow-on review](../_build/verification/viewer-refresh-20261117/root-a1-b1-history-and-idle-bridge-source-review-v1.json).
The future Generated closed checked-spool amendment remains source-only and
unreserved; the final cleanup contract is not implemented.
The C validation bundle keeps its frozen `9297…` epoch; it is not rebound to
`e0df…`. All 54 requirements remain active; no Git integration or whole-product
acceptance is claimed. These assignments are recorded in [Root coordination](../_build/verification/viewer-refresh-20261117/coordination.md).

**Earlier fixture v1 runtime (superseded by v2 above):** on promoted `1e73…`, the original palette case passed,
the new normal-teardown regression exceeded 300s, and 20 cases were unrun. Owned
cleanup is clean; the packet is not accepted. Root's source review proves a missing
precondition: pinned Slint 1.18.1 `show()` retains a component reference while
visible, until `hide()`; dropping the Rust UI handles alone does not establish
UI/callback/service termination. The new test never hides the shown window.
This source flaw is proven, but it is not yet proven to be the sole wait cause.
UI is authorized a private same-file hide/weak-disappearance correction, preserving
the actual Session live before runtime drop. The old packet/run stay immutable;
original stop cause and full acceptance remain open. Engine PO and UI collector
work continue independently; the full goal is not marked blocked.

**Earlier producer and lint checkpoints:** the newer normal validation-input 2 producer passed exit 0,
with 2,288 clean source guards and a matching 359-package notice graph. Current normal
`me.exe` is 110,218,752B, SHA `673084bb…bbc95`; preliminary `c487…` and installed
`f311…` retain their historical epochs. Elevation `b761e65f…` and Main binding
hashes are unchanged; normal Main bindings were Fresh. Complete C validation-input
assembly/registration is now recorded below, without native/runtime acceptance.
After that producer boundary, Root
promoted a four-file Main/ViewerUI test-lint batch; strict retry 5 then exited 101
on one Office HTTP fixture `collapsible_if`. Root promoted the exact equivalent
one-file correction (`679bcf78…`, HTTP fixture `545ff640…`);
Direct checks and the final normal invocation remain pending. Earlier full no-run/UI 23,
headless 99 and failed lint retries retain their source; full 54 remains open.
After Root promoted the final UI parity 1/WorkBudget 2 test-only lint corrections,
the complete workspace/all-target **strict Clippy retry 7 passed exit 0**: 1,001 JSON
events, build-finished=true, zero warnings/errors, 2,288 clean source guards and
normal app `c487…` unchanged. This is a compile/lint pass, not runtime coverage.
Private checked-spool Windows v2 now passed
48 exact cases plus 3 doctests; Linux v1 remains a separate 41+3 checkpoint. Private
B0 v1 passed 34 cases with one warning retained. Later Host/Core/recipe deltas and
the composed 27-source Viewer v2 packet are private and runtime UNRUN; they do not
close production generic-resource cancellation, B1/native or full-goal acceptance.
The later seven-target linked test producer 2 passed exit 0. Actual runtime 1 is
**RED after 10 passes**: case 11's ViewerUI appearance/palette test exceeded the
existing 300s timeout and PID 37548 was killed; counts/assertion outcome are absent.
Cleanup and source/app guards are clean. The subsequent six-case continuation
passed, giving the exact current result **15 direct PASS/1 Viewer TIMEOUT plus a
separate Office 710 unit PASS**; overallAccepted=false. This is not 16 direct passes.
The separate exact-artifact/selector `--nocapture`60s diagnostic ended RED and
exposed the primary panic at `appearance.rs:508`: `palette.restore_palette.unwrap`
received `Err("Viewer is closed.")`. Original 300s RED remains unchanged; the
diagnostic is not acceptance or a reduction of the validation timeout. Backend
stop cause is not observed; closed/stopped/catalog startup remains under diagnosis.
The newer normal validation-input producer 2 passed with a new normal D app
`673084bb…bbc95` (same 110,218,752B). Its 359-package graph matches notice identities;
this remains producer/graph qualification, with no runtime acceptance. The later
same-original-seven-target-graph mailbox diagnostic v3 compiled exit 0/zero warnings
and its one selector passed in libtest-reported 0.21s with no mailbox error output. That isolated
diagnostic pass does not explain, fix or accept the original closed panic/timeout.
UI is implementing an approved private ACK/close fixture-validity change, not a
proven root-cause fix. Full UI, new C, native/B1/DR/PO and full 54 remain open.
Root's additional v3 review independently confirms three raw log pairs, all 2,288
sources and Cargo feature/profile-variant equivalence, with one exact diagnostic
PASS in 0.21s (`2b311c88…18334`); originalFailureExplained=false/acceptanceGreen=false.
Private Viewer PO v2 now has a confirmed source P1: the previously agreed typed
pre-ownership admission is missing. It remains immutable and non-integratable;
Viewer subsequently prepared the private v3 source epoch reviewed below. The three separate Host cases
now have actual behavioral RED evidence: compile exit 0/one historical warning,
three exact assertion failures after controlled rescue and both joins. The later
scoped cooperative GREEN run now passed **14/0/0**: 11 Host plus the three previously
RED Viewer cases. Root's independent review is complete, including four raw logs,
two executables and 1,294 source/proof/absence guards. This private boundary result
is not full Clippy, the 104-Viewer suite, typed PO, LIFE-01 or native/app acceptance.

Common 4 + Engine 33 are now canonical (receipt `da870d71…`). The first Engine
library/configuration compile exited 101 with eight E0308 watch-tree tuple mismatches;
**no runtime tests ran**. Backend applied the exact one-file owner-binding correction
`51609e3a…` without changing helpers or assertions. The corrected Engine libtest and engine_configuration compilation now passed
with two actual artifacts, zero compiler warnings/errors. The staged Engine gate then passed **51 disjoint tests /0 failed /0 ignored**:
initial 7 (raw 0.41s), plus 39 watch/tree library cases and 5 configuration cases.
Postguard drift[] and the application were unchanged; compilation had zero errors
and warnings. These 51 passes retain the staged Engine source. The coherent graph's subsequent compilation is recorded separately below;
UI/runtime and native qualification remain open. Prior private common 29/0 and older Engine
passes keep their own source identities; these counts are not additive.

The original coherent source integration is **canonical**: Root applied 112 exact writes and
retained two already-applied guards, then verified all 114 postimages and seven
protected hashes with zero mismatches (receipt `07417133…`). This includes UI 65,
Backend 19 and the rebased Viewer 28 writes. Cargo.lock was unchanged by this source
integration and remains Build's coherent generation responsibility. D executable
`f311…`, current Office V3 worker/tests and native oracle `710bea…` were preserved.
The original coherent all-target no-run attempt ended **exit 101**. Compiler
records prove fresh normal-library compilation for Main UI bindings, Viewer Engine,
Viewer UI, Compare Engine and Compare UI; this advances the earlier generation-only
milestone. Overall qualification failed on exactly one **E0639** at
`crates/midnight-viewer-ui/tests/ui_polish.rs:895`, a non-exhaustive `KeyEvent`
struct expression. That failed attempt also recorded one dead-code warning for `present_outputs` in
`crates/midnight-viewer-ui/src/lib.rs:904`. Postguards are drift[] and
appUnchanged=true; the executable at that historical attempt remained `f311…`.

The exact two-file correction is now canonical, verified by overlay receipt
`e0739b3c…`, composing with the original `07417133…` source receipt. Viewer UI's
`lib.rs` gates its test adapter with `cfg(test)`; `tests/ui_polish.rs` creates a
`KeyEvent::default()` then assigns its public text field. Assertions, Slint files,
dependency versions and lock `02329f51…` are unchanged. The subsequent compilation attempts are recorded below; runtime/native
acceptance remains pending. The original failed attempt and five successful normal-library
records retain their pre-overlay source; no full graph/runtime/native pass follows.

The Office post-drain finished-oracle fixture `710bea84…` is canonical (freeze
`6610deda…`). The finished-after-drain unit now passed within the 99 headless gate; the five
actual native cases remain pending.
The prior 71 Office plus one exact F5 passes retain their earlier source boundary;
they do not qualify the new helper or native cases. Historical Office parent 0/1
remains recorded; no new Office success is claimed.

The selected maximum-state archive-discovery route may place the genuine Cargo-built
libtest binary at a complete NEW C product's root as a separately recorded validation
payload **before** qualification/freeze. Root authorized the normal deployment tool's
default-off payload route. The real filesystem packages must be adjacent and the
fresh child must prove its resolver path and typed ZIP availability before continuing.
This is accepted execution setup, not an assembled/qualified product or native result.
That execution-setup decision establishes no executable, deployment, Git integration
or full 54-requirement closure; the later preliminary normal producer is separate.

Receipts: [Common/Engine 37 canonical](../_build/verification/viewer-refresh-20261117/backend/resident-workers/common-engine-canonical-stage/canonical-source-ready.json),
[Engine compile failure](../_build/verification/viewer-refresh-20261117/build/engine-resident-stage-gate/failure-handoff.json),
[exact correction](../_build/verification/viewer-refresh-20261117/backend/resident-workers/engine/watch-tree-fixture-canonical-application/canonical-source-ready.json),
[UI 65 source](../_build/verification/viewer-refresh-20261117/ui/appearance-bridge-implementation/review-checkpoint-v3/source-manifest.json),
[Office oracle](../_build/verification/viewer-refresh-20261117/viewer/office-provider-denial/finished-oracle-v1-canonical-freeze.json), and
[maximum-state execution decision](../_build/verification/viewer-refresh-20261117/architect/maximum-state-archive-discovery-decision.md).

The process-sampler prototype passed 12 functional cases and its seven processes
all exited naturally. This validates the collector's functional behavior and cleanup;
it establishes neither application old/new performance nor collector overhead.
See the [sampler review](../_build/verification/viewer-refresh-20261117/root-process-sampler-review.json),
[seven-test result](../_build/verification/viewer-refresh-20261117/build/engine-resident-stage-fixture-fix-gate/initial-seven-result.json), and
[rebased Viewer review](../_build/verification/viewer-refresh-20261117/root-coherent-integration-rebased-viewer-review.json).

Applied-source and staged-test receipts: [canonical 112 writes+2 guards](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/canonical-source-ready.json),
[additional 44 passes](../_build/verification/viewer-refresh-20261117/build/engine-resident-stage-fixture-fix-gate/watch-boundaries-result.json), and
[clean postguards](../_build/verification/viewer-refresh-20261117/build/engine-resident-stage-fixture-fix-gate/guard-after-watch-boundaries.json).

The failed overall attempt and successful normal-artifact records are retained in
[compile summary](../_build/verification/viewer-refresh-20261117/build/coherent-final-gate/compile-summary.json),
[terminal result](../_build/verification/viewer-refresh-20261117/build/coherent-final-gate/compile-result.json), and
[clean postguards](../_build/verification/viewer-refresh-20261117/build/coherent-final-gate/guard-after-compile.json).

Perf completed its scoped source review with **no actionable source finding**.
All 114 canonical inputs matched its reviewed base `07417133…` checkpoint; the
subsequent two-file compiler overlay is recorded separately above. The review
retains bounded-page palette reshaping and preexisting Compare UI pixel copying
as measurement limits. Authentic retained generated Rust totals **69,289,347 bytes**;
there is no qualified old baseline or measured delta. These source-byte counts
and review findings establish no additional application responsiveness/performance
result. See the [overlay receipt](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/ui-compile-fix-v1/canonical-source-ready.json) and
[Perf review receipt](../_build/verification/viewer-refresh-20261117/perf/coherent-render-review-v1/review-receipt.json).

Root applied the next seven consumer fixes on 18 November, receipt
`91150f0e…`. They compose with the original 114-path receipt and earlier two-file
override into **121 distinct canonical source paths**. The seven paths cover four
Viewer Engine constructor examples, local `examples/support/resident.rs`, and
Engine's two direct-source checksum harnesses. Workloads, timing and assertions
were preserved; changes add real resident ownership/cleanup and module wiring.
Slint, dependencies, lock `02329f51…` and executable `f311…` are unchanged.

The intervening library/integration compile failed at exit 101 on missing
`copy_nested_retirement` wiring (E0432), with an unused `CheckedWorkerOwned` import
warning. That failed attempt is retained. In the preserved retry 2 snapshot, all
four repaired examples and both checksum harnesses have fresh compiler-artifact
records, with no observed errors/warnings. There is no build-finished record or
terminal result in that snapshot: the overall compile was still running, with
no full-graph or runtime/native acceptance.

Next-container design remains **proposed only**. Root returned the fixed two
lifetime worker slots and 1 GiB live staging limits for explicit reconciliation with
deeper nesting and the accepted no-artificial-size-ceiling requirement. No A1
implementation approval or feature completion follows from those design documents.
See [seven-path application](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/consumer-fix-v2/canonical-source-ready.json),
[interim compile failure](../_build/verification/viewer-refresh-20261117/build/coherent-interim-libs-tests/compile-result.json), and
[preserved retry 2 observation](../_build/verification/viewer-refresh-20261117/docs/coherent-retry2-partial-observation.json).

The separate 18 November cleanup is complete. C removed four unused runtime
backup directories plus obsolete uv index caches: **677,447,900 logical bytes**,
with an observed **390,033,408-byte** free-space increase. D removed **1,390 obsolete
`.rlib`/`.rmeta` files** from seven old standalone targets: **2,500,434,725 logical
bytes**, with an observed **2,427,236,352-byte** free-space increase. Logical deletion
sizes and observed volume deltas remain separate measurements.

The initial D preflight was interrupted before deletion; all 1,565 original
candidates were confirmed present before excluding HTML. The HTML provider target
was entirely preserved, along with the default target, qa, verification, EXE/DLL
and native SDK files, screenshots and current C runtime. Three protected app/Source
hashes were unchanged at that cleanup checkpoint. No cleanup holds remained. The cleanup made no product-source or ACL changes. This cleanup adds no implementation, compilation, runtime or native
qualification. [Measured cleanup receipt](../_build/verification/disk-cleanup-20261118/final-result.json).

A separate renewed cleanup is now verified: C373 files removed **3,208,506,127
logical bytes**, with observed free space **+2,828,046,336 bytes**. D's exact six
obsolete FQA roots contained 1,795 files/**3,558,657,330 logical bytes**. Those D
fixtures were already compressed (about 483MB prior allocation), and concurrent
builds consumed space; the logical total is not a physical-reclaim measurement.
Current C/D application and generated Main hashes are unchanged. This later
cleanup is separate from the earlier totals and adds no feature/native result.
[Renewed cleanup receipt](../_build/verification/disk-cleanup-20261118-current/final-result.json).

A further separately sealed cleanup removed C412 files/**3,255,585,645 logical
bytes** and D642 files/**7,250,716,439 logical bytes**, with no skipped files.
Protected C/D executables are unchanged across this cleanup; 164 current default
symbols and the recorded QA retention remain intact. These are logical sizes,
not measured net physical reclaim; compression, hardlinks and concurrent activity
affect free space. No process was terminated or ACL changed by cleanup.
[Additional cleanup receipt](../_build/verification/disk-cleanup-20261118-additional/final-result.json).

A separate C-then-D follow-up removed C132,982 old staging/cache/log files
(3,263,580,433 logical bytes), skipping 90: 88 in use, one changed and one already
gone. D removed 318 explicitly owner-released intermediate/PDB files
(660,271,043 logical bytes), with no skips; 155 retained RED entries were rechecked
for identity/size/mtime. Installed rustc and three normal EXEs retain their hashes.
Root reports 18 audit files (~166MB) copied/hash-verified to D before removing only
this task's temporary C audit. Earlier 642 deletions are not counted again. These
are logical cleanup totals, not physical reclaim or feature/test acceptance;
no ACL/process/system-memory changes or source/user-doc/settings/chat/active-build
deletion occurred. [C-then-D follow-up receipt](../_build/verification/disk-cleanup-20261118-next/final-result.json).

The newer verified checkpoint supersedes retry 2's earlier partial observation:
retry 2 ended **exit 101** on 18 November, from the outdated
`AcceptedPreferences` caller in `midnight-ui/examples/native_keyboard_search.rs`.
An analogous Linux caller was identified. Bindings remained fresh/unchanged;
source postguards were drift[] with unchanged application. Full graph qualification
remains open.

Root reviewed **27 direct UI case receipts: 22 passed /5 failed**, excluding copied
`review-evidence` receipts. Every before/after guard, cleanupErrors and remainingOwned
check was clean. These are scoped runtime results, not a globally green UI/native
qualification. The five failures remain recorded rather than removed from totals.

The Unicode failure was traced to per-event NFC in the pinned Slint fixture path.
The fixture-only `1783…` → `7713ab2a…` change is canonical: per-scalar key events
preserve the exact decomposed query/clipboard assertions. Production text handling,
Slint and dependencies are unchanged. Replay is pending.

Root inspected the Options focus BMP and converted touch-footer PPMs. The affected
capture helper allocates a new buffer but requests reused-buffer dirty-region
rendering, producing partial screenshots. The UI 8 full-frame test-helper source correction is now canonical;
existing geometry/input results retain their scope, while affected captures are
**not full-frame visual proof**. No passing capture-helper replay is claimed.

The Source native harness cleanup is now applied in the Source 2 packet below;
compiler/native replay remains pending, with no production-leak finding. A fresh Office native driver must select the actual five native cases plus
the separate unignored finished-after-drain unit, and bind the new complete C
product/actual harness identities. The old three-case driver is insufficient.
No complete new C product or native qualification is claimed.

Root accepted the exact three deployment tools with **22 frozen payload tests**;
the **13 prior unchanged direct-boundary passes** retain their own scope. The older
combined 34 overlaps this evidence and must not be added again. Tool acceptance is
not deployment or native MAX success.

Architect R2's source checkpoint reports 164 hashes with zero drift. Its later
StageProgress addendum records only a narrow private filesystem-host accounting/
direct-staging checkpoint. Full A1/logical/history and canonical/Core/Engine/API
integration remain pending explicit freeze; no A1 feature or runtime acceptance
is implied. The original 54-requirement goal remains open.

Current receipts: [UI 27 review](../_build/verification/viewer-refresh-20261117/root-ui-runtime-overlap-review.json),
[literal fixture promotion](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/ui-literal-fixture-v1/canonical-source-ready.json),
[retry 2 terminal result](../_build/verification/viewer-refresh-20261117/build/coherent-final-retry-2/compile-result.json),
[tool review](../_build/verification/viewer-refresh-20261117/root-validation-payload-tool-review.json), and
[R2 proposal](../_build/verification/viewer-refresh-20261117/architect/tree-container-next/handoff.json), and
[required native selectors](../_build/verification/viewer-refresh-20261117/docs/office-driver-required-selection.json).

Root's completed headless retry 2 review verified **99 distinct passes /0 failures /
0 ignored** within that gate: Compare 17, Viewer 26, Host 47 and checksum 4+5. The
finished-after-drain unit actually passed. All 1,400 source guards and owned cleanup
were clean. These are complete artifacts from a partially failed compile, not a
synthetic overall compile-success result. Host 47 overlaps older 71/epoch evidence;
the totals must not be added across checkpoints.

The two Source native-fixture paths are now canonical. Original 34 assertions and
workload helpers are unchanged; real Runtime/Owner 5 registration, checked shutdown
and unwind observation are fenced in the fixture. Compilation/native replay remain
pending; this is not a production-leak finding or native acceptance.

The eight UI correction paths are now canonical. Original deadlines/exact assertions
remain, with six assertions strengthened. One production Main Slint change corrects
Options popup-focus ordering; the other changes cover test/probe inputs, stronger
diagnostics and full-frame capture helpers. The exact decomposed Unicode invariant
from 7713 remains in successor 19214. Runtime confirmation is pending, and old partial
captures do not become full-frame proof. Main bindings legitimately require
regeneration; Viewer/Compare Slint sources are unchanged.

Build's retry 4 full workspace/all-target no-run compilation completed successfully;
strict Clippy subsequently failed as recorded below. Root's actual source-size guard passed after the
promotions: **2,542 source files at most 100,000 bytes**. That policy check is separate
from compiler/runtime/native qualification.

Backend's private scope now also includes the isolated checked-spool/native-root
implementation authorized after Root accepted the R3.2 Metadata 1 contract at
proposal level. Full canonical A1/Core/history integration remains unapproved.
That private authorization establishes no complete native product or full 54 closure;
the later preliminary normal producer has its own evidence below.
Receipts: [headless 99](../_build/verification/viewer-refresh-20261117/root-headless-retry2-review.json),
[Source 2 canonical](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/source-native-retirement-v1/canonical-source-ready.json), and
[UI 8 canonical](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/ui-scoped-corrections-v1/canonical-source-ready.json).

Root first verified **18 retry 4 UI passes /0 failed**, including the four formerly
failing Viewer/Compare cases. A newer completion receipt now proves the exact
immutable 20 set passed **20/0/0**, using the new matching Main comparator artifact
for its two Options focus/layout cases; the prior 18 were not rerun or added again.
The separately requested `resizing_preserves_the_existing_external_field_and_its_caret`
also passed once and remains outside immutable 20.

The two separate Options transaction cases subsequently passed once each:
`cancel_escape_and_x_restore_preview_without_writing_settings` and
`ok_saves_once_and_closes_only_after_backend_success`. Root's final addendum
verifies exactly 23 unique UI passes /0 failed /0 ignored with clean guards and
owned cleanup: original 20 plus caret 1 plus transactions 2. Earlier 18/20 receipts
are overlapping checkpoints and are not added again. The actual compile JSONL
ends `build-finished success=true`, compile result is exit 0, and closing guards
are drift[]/appUnchanged=true. This qualifies the no-run compilation and selected
UI cases; normal application/native acceptance remains open.

Strict Clippy subsequently ended exit 101 with one `clippy::duplicate_mod` error:
Explorer's test fixture helper `resident_test_runtime.rs` is loaded three times.
The full no-run compile and UI 23 results remain valid for their source/scope.
Root subsequently promoted the exact four-file `cfg(test)` single-definition/import
correction (`fec5c57f…d303cc`). Root's inverse byte comparison proves the original
test bodies and helper/runtime behavior unchanged. Its complete 2,288-source guard
found no drift outside the intended four files; the helper and normal app remain
unchanged. The immutable Viewer freeze `dd204ddc…d1a` matches that canonical receipt.
The same complete strict Clippy command advanced past the Explorer issue, then
retry 1 ended **exit 101** on two `clippy::duplicate_mod` diagnostics in CompareUI's
libtest: raw-platform-path helpers and `tests/support/mod.rs` loaded repeatedly.
Closing 2,288-source guard is drift[]/appUnchanged=true. This establishes progress
past the Explorer lint issue, not a strict-lint or corrected Explorer harness pass.
Root subsequently promoted the reviewed four-file CompareUI shared `cfg(test)`
definition/import correction (`6bccd5af…d1d7fc`). Exact inverse/body guards preserve
all 14 test bodies, helper bytes, raw Unicode fixtures and runtime limits. Root's
complete 2,288-source guard is otherwise clean and the four-file rustfmt check
passed exit 0. New strict Clippy, affected harness compilation and seven direct
checks remain pending with Build. These source/format checks supply no new lint,
runtime, native or normal executable qualification; compile/UI 23 remain dated.
The subsequent strict Clippy retry 2 now ended **exit 101** on three
`clippy::chunks_exact_to_as_chunks` uses in Compare Engine test pixel handling.
The earlier helper-module lint issues are fixed. Root subsequently promoted the
two-file mechanical pixel iteration correction (`386f10ae…1e951`): only three
expressions change to `as_chunks`, with the same RGBA comparisons and ignored
remainder. Assertions, runtime, production behavior and dependencies are unchanged;
the complete source guard is otherwise clean and app `f311…` is unchanged.
The v1 iterator shape was Root's request; strict retry 3 subsequently exited 101
on `manual_contains`. That failed retry remains recorded. Root then promoted
pixel v2 (`c92cae2a…ac50f3a`), replacing the same three membership expressions
with `contains`, with unchanged RGBA/remainder semantics, assertion values,
runtime, production behavior and dependencies. Build's pinned isolated expression
probe passed after the change (before exit 1/after exit 0); it is not a full-crate
or workspace qualification. Whole-workspace strict replay and direct runtime
remain pending, with no strict Clippy PASS. Relevant future A1 private adapters
must preserve this accepted baseline.

At the confirmed terminal boundary Root separately promoted the approved one-line
HTML manifest spelling correction (`ad55e66a…322d06`): `=156.4.0+156.0.26` becomes
`=156.4.0`. Both locked `cef`/`cef-dll-sys` versions remain `156.4.0+156.0.26`
with identical checksums, selected MIT licenses, native SDK and sandbox features.
Only the requirement metadata suffix changed; dependency resolution, inventory
and notices are unchanged and need no update for this spelling correction.
The warning-free standalone producer replay is still pending with Build; historical
Cargo warnings remain evidence and their removal is not yet claimed. Strict lint, corrected harness,
native acceptance and the new normal executable remain open. Earlier lint failures
and compile/UI 23 passes retain their dated sources; no new PASS is claimed.

Root also viewed the new initial-focus BMP: the focused error field is visible
inside the scrolling body and the complete Options/Comparators layout is readable.
This is attributed visual evidence for that capture, not all layouts/platforms.

Root visually verified the new complete paged/continuous 320×200 footer captures
after lossless conversion. The reused-buffer black-delta capture flaw is corrected
for these new images; old partial captures remain preserved with their limitations.
This confirms the selected footer evidence, not every layout/platform or performance.

Private H/R source review found two Windows-invalid ParentDir fixture roots. Only
the exact two expressions changed in immutable `fixture-path-v2` (`3d6a34ee…`).
Root/Perf source reviews are clear; eight postimages/baselines and 30 v1 artifacts
plus their manifest are preserved/checked. Build may qualify this private checkpoint,
and its earlier handoff correctly records 15 tests UNRUN at that time. A subsequent
isolated qualification compiled once in 39.157s and passed **50 unique exact cases
/0 failed /0 ignored**: 15 new accounting/staging/watchdog cases plus 35 existing
direct boundaries. Root checked all 50 raw stdout/stderr identities/outcomes, all
eight actual compiled postimages against v2, unchanged artifact `9fe3a727…`, and
clean cleanup/source/Root guards. This is **private only**, with no canonical A1,
full-feature, native or application acceptance.

All 42 required registry name/version/source/checksum pins match; 42 metadata and 39
compiled feature records show no expansion beyond the frozen Root reference.
Original raw 11 whole-lock-row mismatch flags remain preserved: only pruned
dependency lists differed, with no repins. The successful pin-field projection
explains those flags; it does not erase the original report or imply a whole-app
dependency/security scan.

Root accepted the separate seven-file checked-spool/native-root candidate for
**private qualification only** after source/native ownership review and exact
pre/post/legacy diff guards (`5bb6d4b3…`). Its 31 authored units are partitioned
common 10/Windows 13/Linux 8, plus three compile-fail doctests; all were UNRUN at the
source-review handoff. Subsequent private runtime receipts now record **Linux 41
PASS/0 failed plus 3 negative-API doctests**, and **Windows 47 PASS/1 setup RED plus 3
doctests**. Root independently verified raw logs and clean source/pin/feature guards.
The Windows failed construction case remains unqualified; Backend is diagnosing
raw Win32 path/setup behavior. Per-platform cases overlap and must not be summed
as a unique total or added to H/R50. This is partial private qualification, with
no canonical code, M7 staging/graph/actor integration or full native-close acceptance.
The current application producer retains priority.
Root approved private Windows fixture v2 replay (`908d8f8b…`): exactly one setup
expression changed; all production bytes, the other six postimages and 29 v1
artifacts remain unchanged. The later v2 result now passed **48 exact cases plus 3
doctests**, including 23 new Windows-applicable cases. Root checked 100 raw-log
hashes, seven executables and seven composed postimages (`f274e348…e0fa8e`). Perf
disposition is pending; no canonical promotion, M7 or LIFE-01 closure follows.
The old Windows RED remains recorded. Raw OS 123 setup failure is proven;
the mixed-separator/verbatim explanation remains an inference pending the exact
Win32 probe. Existing Linux 41+3 evidence retains its unchanged consumed-source epoch.

R3.2 now explicitly specifies one application-wide Archive Metadata 1 resident
domain, created at Explorer bootstrap and owned by the outer application guard.
Engine 13/Viewer 5/Compare 2 and the original transient 2 async/16 blocking stay fixed;
the proposed resident total is 21. Captures receive clients only. The metadata
actor remains alive through native retirement, then drains before its owner joins.
Root accepted this contract at proposal level and authorized the isolated private
checked-spool/native-root implementation. This is explicit topology, not evidence
of production implementation or execution; M11 acceptance remains UNRUN and full
canonical A1/Core/history integration remains unapproved. The earlier R3 handoff's
flags retain their historical scope; the later shared-contract freeze below now
establishes accepted design without claiming implementation completion.

Root accepted the completed shared A1 contract and froze `A1-shared-api-v1`
(`f319802d…ec06e`, shared API `64bfa555…e2764d`). The freeze records 153 unique
owner paths, current baselines, decision documents and nine read-only guards.
The Engine interface must obtain the existing vacant `ServiceSlot` before
ownership transfer: duplicate configuration cannot replace the service. Duplicate
and unused-slot tests remain required. This incorporates Root's correction to
the proposed replacement-style setter; no new dependency is introduced.

Backend B0 is privately dispatched; Backend owns the five app-integration rows
after B0. Viewer V and UI U prepare private adapters/fixtures in parallel and must
attach immutable B0 before compilation. No canonical A1 writes are authorized.
Contract acceptance does not complete implementation: R2-1..11, M1..M11, duplicate/
unused-slot acceptance and later B/C work remain required. Canonical build, the
new complete C product and native qualification retain priority; full 54 stays open.
Viewer identified one concrete `ArchiveViewer`/`source_identity` declaration
omission in the frozen contract. B0's subsequent immutable additive clarification
now supplies the declaration/constructor and typed fixture seam; original frozen
artifacts remain unchanged. Source identity is backend-issued and copied with the
complete original revision; no local IDs are invented. This private source result
does not claim B0/Viewer adapter compilation or behavioral qualification.
The actual immutable B0 packet now has 16 rows plus its typed fixture recipe,
reviewed by Root (`09c428a3…ad58c`) and accepted for private Core/plugin compile/tests
and real typed consumer fixtures. This source review is not an executed test or
canonical actor/feature result. Root found cooperative host read/preparation
pre/post `source.validate()` lacks the local cancellation predicate; the real
Viewer Asset validation can wait on a noncancelable mutex loop. Architect's
minimal additive hook decision is pending. The 14 inline tests do not establish
end-to-end cancellation, including actual-host contention/EOF boundaries.
Subsequent immutable B0 v1 private execution now passed **34 cases: 14 new plus 20
direct**, independently reviewed at `d32d1ff2…`. Its one deprecated `fetch_update`
constructor warning remains explicit; this is not a strict-qualified private gate.
The separate actual-host delta `9ddee287…` and Core/typed-recipe correction
`29a13391…` are later private source epochs, not a rebind of those 34 passes.

Root accepted the composed private Viewer/Core/Host v2 source packet:
freeze `51c32688…`, inventory `58ff82bd…`, 27 verified source rows (`7e910068…`).
At that source-review checkpoint the selected 104 Viewer/11 Host cases and three
separate original-B0-v1 RED declarations were all UNRUN. The three RED cases have
since actually failed on the expected cancellation assertions, then passed in the
scoped 14-case cooperative GREEN run with 11 Host cases. The broader source packet's
“GREEN” label still establishes no full-suite or PO result. This 14-case scope is
not an extra unique total to add to the source selections or the prior three REDs.
Root also authorized five extra UI production-consumer reservations beyond the
frozen UI 30 (`b72a7ef2…`), still private, with original frozen A1 artifacts unchanged.
Production generic-resource cancellation/cleanup, real B1/native and installed
acceptance remain open; no canonical lifecycle implementation is claimed.
Root subsequently confirmed the typed pre-ownership P1 in private PO v2
(`77c65185…54e3a`). Admission must precede source handoff and serialize with seal;
late refusal cannot mutate completed counts/results. Already-owned refused sources
remain in the caller's checked disposition boundary, outside the completed PO
receipt, preserving real errors and uncertain owners. The prior v2 source review
is retained but integrationAccepted=false. Viewer prepares a narrow additive
correction and Architect confirms the caller boundary; old v2 artifacts are not
rewritten or rebound. Host RED compilation is live at Root's checkpoint, with no
runtime result at that earlier checkpoint; compilation failure or zero selected
cases remains insufficient. The later actual 3-RED receipt below supersedes only
that execution status, not the PO admission gap.

Root independently verified three exact behavioral REDs (`63867970…c01b`): Prepare,
Read and positive-EOF cancellation waited for an unrelated held read. Nine raw
log/list/summary hashes and actual executable `42422763…` match. The controlled
rescue and both joins complete before the final assertions, so these are behavioral
failures rather than a compile error or stranded timeout. Compilation passed with
one historical `fetch_update` warning; this is not strict-qualified. Existing
At that checkpoint GREEN v2 was authorized for 14 scoped cooperative cases, unrun.
That authorization was followed by actual **14 PASS/0 failed/0 ignored**. Root's
completed review (`d5d14402…93c5c`) verifies exact 11 Host+3 Viewer selectors, four raw
logs, two actual executables and all 1,294 source/proof/absence guards across three
schemas. This proves the private cooperative-cancellation boundary only. Ordinary
`cargo test --no-run` compilation passed with zero warnings; it is not a full
Clippy `-D warnings` qualification, regardless of the compile parser's field name.
Canonical promotion, typed PO admission, full 104 Viewer, whole LIFE-01 and native/
application acceptance remain incomplete. The original 3-RED evidence is retained.

Root also source-reviewed the sealed one-file palette fixture-validity candidate
(`085a…`, `e3bc41ea…` → `1e73da24…`): 25 referenced records checked, 19 old tests
preserved, two new regressions UNRUN and rustfmt exit 0. It waits for the actual C
assembly source hold to release before canonical promotion and targeted runtime.
Only helper-created tracked sessions and entry ACK are covered; full presentation,
bitmap/platform/appearance bodies and production/catalog/runtime inputs remain
unchanged. This source acceptance neither explains nor fixes the original stop
cause and provides no whole LIFE-01/native acceptance.
Root also observed actual C assembly PID 54224 with complete normal 673 inputs and
the real `cf411…` test payload/source epoch `9297…`. The destination is being
populated; it was not yet frozen or accepted at that live checkpoint. That hold
then prevented canonical promotion of the private palette helper candidate.
That assembly subsequently completed exit 0 and normal-only registration succeeded.
Root independently rehashed the exact **2,487-file/752,608,762B** new C validation
bundle (`c14c67c8…30d4`): six Viewers, three filesystems, two thumbnailers, 662 notices,
normal `673084…` (`test=false`) and genuine `cf411…` payload (`test=true`). Inventory
and post-registration hashes match. NativeRoot is copied verbatim from actual
native-identities/completion:
`<user-profile>\MidnightExplorer-validation-20261118-07672814`.
This is a complete frozen validation input with normal-only registration, not
normal/native acceptance: no application or test has launched from it.

After Build released the assembly source hold, Root promoted the one-file helper
`e3bc…` → `1e73…` (`fdf06c25…df9af`), with 2,288 clean guards, rustfmt 0 and unchanged
normal 673/Main artifacts. Its new source snapshot `9c25…` is distinct from the
assembled C bundle's immutable `9297…` epoch. Same-original-seven-target compilation
and the original palette case plus two new lifecycle selectors remain pending at
the 300s validation deadline. The old C bundle and failed epochs are not rebound;
original stop cause is still unexplained and no full 54 acceptance is claimed.
Root then accepted the additive private PO v3 source packet for matching
validation (`2db73d40…29563`): 33 sealed references and all 27 source rows verified,
five Engine changes with 22 rows unchanged and exact canonical baselines. The
token-before-capture/caller/neutral-Finished paths were source-reviewed. This
authorizes **three compatible admission RED and 129 GREEN expectations only**;
no tests are executed or canonical integration authorized by that review.
The old v2 remains immutable/non-integratable. Actual UI collector, App/DR/native
LIFE integration and original palette acceptance remain pending; source review is
not a behavioral fix or full-goal completion.
Viewer also reported explicit erased-resource and abandoned-layout cleanup receipt
gaps. Architect owns the additive seam decisions; these are open findings, not
implemented cleanup guarantees. Full LIFE-01 remains open, including checked
ownership/release acceptance beyond the private checked-spool source review.
Private DR Core/native and PO extra Viewer reservations are dispatched against
their exact contracts. These assignments establish no canonical lifecycle
implementation or full-goal completion.

After the preliminary normal producer released its boundary, Root promoted the
final Main/ViewerUI four-file test-lint batch (`5c166af1…`): Main's shared raw-path
helpers use one `cfg(test)` definition/import wiring, and one ViewerUI test pixel
allocation is equivalent. Root inverse/helper guards and pinned rustfmt passed;
the 2,288-source guard is otherwise clean and the new `c487eff2…` app is unchanged
during promotion. Current-source strict Clippy, direct tests and final normal
invocation are pending with Build. The normal `test=false` producer milestone and
this later source promotion do not qualify complete C deployment/native workflows.
The normal module-producer handoff reports all 9 Viewer/filesystem modules plus 2
thumbnailers compiled/staged. A newer Root review completed 1,821 staged file-hash
and 1,945 producer-input checks with no errors. This verifies that staging scope;
normal host/root notices, complete C assembly and native acceptance remain separate.
Strict retry 5 subsequently ended 101 on one Office HTTP fixture `collapsible_if`;
Root then promoted the exact short-circuit-equivalent correction (`679bcf78…`).
HTTP fixture `online_http.rs` is now `545ff640…` (previously `e053fc1c…`), with
duplicate-Host error, trimming and case comparison preserved; source guards are
otherwise clean and normal app `c487…` unchanged. This creates a new test-only
input epoch: actual native Office replay requires refreshed qualification and old
receipts are not rebound. Full Clippy and the updated native libtest remain pending.
Pinned `cargo check --help` advertises `--keep-going` at line 43. Root authorized
that CHECK option through Clippy's next replay to collect independent failures;
this authorization does not assume support in `cargo test` or establish a result.
The supported keep-going retry 6 actually collected three test lints across two
files: Main UI `filename_colors.rs` (`manual_is_multiple_of`) and work-budget
resident tests (`redundant_async_block`, `manual_noop_waker`). It ended 101;
owners are fixing these findings and no whole-workspace strict PASS is claimed.
Those three findings are now corrected by the canonical final test-only batch
`99eb030d…`; production semantics and test selectors remain unchanged, with no
lint suppression. The actual full workspace/all-targets retry 7 subsequently
passed **exit 0/zero warnings-errors**, 1,001 JSON events ending build-finished=true,
2,288 source guards/drift[] and unchanged `c487…` application. Root independently
reviewed the raw result (`368be4ef…38dd`). Earlier failed retries remain dated
evidence; this compile/lint gate supplies no broad runtime pass.

The initial six-target producer was followed by the actual seven-target linked
producer 2, including the required MainUI wrapper libtest, which passed exit 0. Only the
giant bindings-crate libtest is excluded from this targeted producer. The corrected
scope retains **all 16 direct cases plus the separate Office unit**. The MainUI
requirements include native picker raw-path identity/adoption and stale reply
guards; no test is dropped by the earlier overly broad exclusion. Actual runtime
acceptance remains RED: original runtime 1 stopped after 10 passes at case 11, then
the six independent cases passed separately. Root's final raw review verifies all
17 selector outcomes: 15 direct PASS/1 TIMEOUT plus separate Office unit PASS,
51 log/summary readbacks, five capture hashes (not visual review), and seven actual
executables; overallAccepted=false. The separate 60s diagnostic exposed
`restore_palette.unwrap` receiving `Viewer is closed` at line 508, while the 300s
failure stays recorded. UI owns private failure-only mailbox logging and fixture/
lifecycle source review; Architect reviews existing PreparedCatalog seams. No
backend stop cause, production guard weakening, fake runtime or canonical fix is
claimed. Final
normal 359-graph comparison, new C freeze and native Source/Office/
MAX acceptance remain pending. Authored/private archive actor work is not implemented
or qualified by the passing Clippy gate.
The later normal validation-input 2 producer now completes the 359-identity graph
comparison against notices, with `test=false` app `673084bb…bbc95`; actual new C
assembly/freeze and runtime/native acceptance remain pending. Mailbox-original-
graph-v3 preserves the original seven-target graph and compiles cleanly. Its one
diagnostic selector PASS/no mailbox error is an observation in a later instrumented
test epoch, not a rebind of the 17 original outcomes or proof of a fixed stop cause.

Host notice completion-v3 resolves the 11 omitted license-source packages:
662 staged files, 61 override files and zero missing inputs. Root verified exact
crate checksums/VCS commits and 27 saved official manifest/license responses and
copy hashes (`75f68311…ece974`). Dependencies, project license and license choices
are unchanged. This notice-source staging is ready for the final normal 359-graph
comparison and explicit copy before new C freeze; it is not final distribution
qualification. The 9-module/2-thumbnailer staging review remains clean at 1,821
file and 1,945 input hashes (`10be6a5a…`); it supplies no native acceptance.
Receipts: [retry 4 eighteen](../_build/verification/viewer-refresh-20261117/root-ui-retry4-runtime-review.json),
[immutable 20 plus separate caret](../_build/verification/viewer-refresh-20261117/build/coherent-final-retry-4/test-prep/ui-root20-completion.json),
[final compile/UI 23 addendum](../_build/verification/viewer-refresh-20261117/root-ui-retry4-runtime-final-addendum.json),
[compile result](../_build/verification/viewer-refresh-20261117/build/coherent-final-retry-4/compile-result.json),
[strict Clippy failure](../_build/verification/viewer-refresh-20261117/build/coherent-final-lint/compile-summary.json),
[canonical test-module correction](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/explorer-test-module-lint-v1/canonical-source-ready.json),
[Clippy retry 1 failure](../_build/verification/viewer-refresh-20261117/build/coherent-final-lint-retry-1/compile-summary.json),
[canonical CEF spelling correction](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/cef-constraint-spelling-v1/canonical-source-ready.json),
[canonical CompareUI helper correction](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/compare-libtest-shared-helpers-v1/canonical-source-ready.json),
[private H/R50 runtime review](../_build/verification/viewer-refresh-20261117/root-decoded-work-private-v2-runtime-review.json),
[private checked-spool source review](../_build/verification/viewer-refresh-20261117/root-checked-spool-r3-private-review.json),
[strict Clippy retry 2 failure](../_build/verification/viewer-refresh-20261117/build/coherent-final-lint-retry-2/compile-summary.json),
[canonical pixel iteration correction](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/compare-pixel-iteration-lint-v1/canonical-source-ready.json),
[canonical pixel v2 correction](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/compare-pixel-iteration-lint-v2/canonical-source-ready.json),
[strict retry 3 failure](../_build/verification/viewer-refresh-20261117/build/coherent-final-lint-retry-3/compile-summary.json),
[isolated expression probe](../_build/verification/viewer-refresh-20261117/build/manual-contains-probe/result.json),
[preliminary normal producer](../_build/verification/viewer-refresh-20261117/build/normal-host-preliminary-1/qualification.json),
[canonical Main/ViewerUI test-lint batch](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/main-viewer-lint-batch-v1/canonical-source-ready.json),
[checked-spool partial platform runtime review](../_build/verification/viewer-refresh-20261117/root-checked-spool-r3-runtime-partial-review.json),
[strict retry 5 failure](../_build/verification/viewer-refresh-20261117/build/coherent-final-lint-retry-5/compile-summary.json),
[pinned CHECK help](../_build/verification/viewer-refresh-20261117/build/coherent-final-lint/pinned-check-help.txt),
[canonical Office HTTP fixture correction](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/online-http-host-lint-v1/canonical-source-ready.json),
[private Windows fixture v2 review](../_build/verification/viewer-refresh-20261117/root-checked-spool-r3-fixture-v2-review.json),
[Windows v2 actual 48+3 review](../_build/verification/viewer-refresh-20261117/root-checked-spool-r3-windows-v2-runtime-review.json),
[B0 immutable source review](../_build/verification/viewer-refresh-20261117/root-a1-b0-interface-source-review-v1.json),
[B0 ArchiveViewer additive clarification](../_build/verification/viewer-refresh-20261117/backend/a1-b0-api-v1/core-api-v1/reviewed-inputs/archive-viewer-api-clarification-v1.md),
[normal module staging review](../_build/verification/viewer-refresh-20261117/root-normal-module-staging-review-v1.json),
[strict retry 6 findings](../_build/verification/viewer-refresh-20261117/build/coherent-final-lint-retry-6/compile-summary.json),
[full strict retry 7 pass](../_build/verification/viewer-refresh-20261117/root-full-strict-clippy-retry7-review.json),
[final canonical test-lint batch](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/final-three-lints-v1/canonical-source-ready.json),
[host notice source review](../_build/verification/viewer-refresh-20261117/root-host-notices-staging-review-v1.json),
[notice completion-v3 qualification](../_build/verification/viewer-refresh-20261117/build/host-notices-stage/completion-v3/stage-qualification.json),
[runtime 1 sealed result](../_build/verification/viewer-refresh-20261117/build/current-boundary-runtime-v1/runtime-1/parent-boundary-review.json),
[seven-target linked producer 2](../_build/verification/viewer-refresh-20261117/build/post-lint-targeted-producer-2/compile-result.json),
[private B0 v1 actual 34 review](../_build/verification/viewer-refresh-20261117/root-a1-b0-v1-runtime-review.json),
[composed Viewer/Core/Host v2 source review](../_build/verification/viewer-refresh-20261117/root-viewer-po-composed-v2-source-review.json),
[five extra private UI consumers](../_build/verification/viewer-refresh-20261117/root-a1-ui-five-consumer-reservation-v1.json),
[current exact 17-outcome partial review](../_build/verification/viewer-refresh-20261117/root-current-boundary-runtime-partial-review.json),
[separate nocapture diagnostic review](../_build/verification/viewer-refresh-20261117/build/viewer-ui-nocapture-diagnostic-v1/parent-review.json),
[normal validation-input 2 qualification](../_build/verification/viewer-refresh-20261117/build/normal-host-validation-input-2/qualification.json),
[same-seven-graph mailbox v3 diagnostic review](../_build/verification/viewer-refresh-20261117/build/viewer-ui-mailbox-original-graph-v3/parent-close-review.json),
[independent original-graph v3 review](../_build/verification/viewer-refresh-20261117/root-original-graph-viewer-diagnostic-v3-review.json),
[PO v2 typed admission P1](../_build/verification/viewer-refresh-20261117/root-viewer-po-v2-admission-gap-review.json),
[three actual Host/Viewer behavioral REDs](../_build/verification/viewer-refresh-20261117/root-host-viewer-cancel-three-red-review.json),
[sealed palette fixture-validity source review](../_build/verification/viewer-refresh-20261117/root-palette-fixture-validity-v1-source-review.json),
[narrow cooperative GREEN 14 independent review](../_build/verification/viewer-refresh-20261117/root-host-viewer-cooperative-green14-review.json),
[completed C9297 validation bundle review](../_build/verification/viewer-refresh-20261117/root-new-c-validation-bundle-9297-review.json),
[actual C NativeRoot identities](../_build/verification/viewer-refresh-20261117/build/new-c-product-assembly-v1/attempt-current9297-1/native-identities.json),
[actual C assembly completion](../_build/verification/viewer-refresh-20261117/build/new-c-product-assembly-v1/attempt-current9297-1/completion.json),
[canonical palette fixture promotion](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/palette-fixture-validity-v1/canonical-source-ready.json),
[private PO v3 admission source review](../_build/verification/viewer-refresh-20261117/root-viewer-po-admission-v3-source-review.json),
[fixture 1e73 partial runtime and shown-owner precondition review](../_build/verification/viewer-refresh-20261117/root-fixture-validity-v1-partial-runtime-and-visible-owner-review.json),
[fixture 1e73 sealed runtime boundary](../_build/verification/viewer-refresh-20261117/build/viewer-palette-fixture-validity-v1/parent-boundary-close-review.json),
[Options transactions 2](../_build/verification/viewer-refresh-20261117/build/coherent-final-retry-4/test-prep/ui-options-runtime-1/result.json),
[R3.2 execution contract](../_build/verification/viewer-refresh-20261117/architect/tree-container-next/handoff.json),
[accepted A1 shared-contract freeze](../_build/verification/viewer-refresh-20261117/architect/tree-container-next/a1-freeze-v1.json),
[Root A1 contract review](../_build/verification/viewer-refresh-20261117/root-a1-shared-contract-v1-review.json),
[footer conversion](../_build/verification/viewer-refresh-20261117/root-render-review/retry4/conversion.json), and
[private fixture-path-v2](../_build/verification/viewer-refresh-20261117/backend/decoded-work-r2-20261118/fixture-path-v2/source-ready-v2.json).

## Previous scoped checkpoints


The 16 November independent Windows plugin qualification passed Source 49
and Markdown 76 with 1 ignored, covering all 13 normal artifacts each. Physical
desktop and full-app qualification remain separate; earlier 41/42 are prior subsets.
The specific Copy authority fix passed five required runtime cases, while AB56
remained 78/1/0 with four critical selectors unrun in that phase. The initial
13-component rebuild passed with recorded warnings. Corrected full Engine lib
passed 1375/0/8, filesystem-host lib 46/0/1 and RealOps 14/0/1
(passed/failed/ignored). All 6 API, 11 production plugins, 2 support boundaries
and root doctests passed. Corrected UI continuation qualified 460 unique cases
across 82 harnesses. Scoped quality and warm host/inventory/validator passed;
the corrected first-use image regression and all 25 selected native cases are
qualified across the retained and final scoped runs. Viewer libraries passed
33/0/4 and 256/0/4; the thumbnail observer fixture correction passed four native
cases without changing production timeouts. Actual normal-profile deployment and
launcher registration were verified. Original RED evidence is preserved.
Desktop10/manual/UNC/Linux gaps remain.
[Active work](ACTIVE_WORK.md) retains exact scopes.

Compatible plugin packages can be replaced with the app closed, then loaded
after restart. There is no per-plugin pinned SHA. Protected path metadata
selects the registered runtime by default; opt-in verification uses EXE hashes.
Loaders still check ABI, platform, manifest and permissions.
See [runtime discovery](developer/RUNTIME_DISCOVERY.md).

Current branch refresh (17 November): Source implements Lua/YAML highlighting,
C++ raw-string state and style-preserving dense-page cropping. Latest Source
post-review fixes passed 64 unique checks across seven groups, strict Clippy/
format 0, including both P2 RED→GREEN cases. Prior 99 remains pre-fix evidence,
not rerun or additive. These are local Source scopes, not final app/native UI
qualification. Image comparison now has Side by side / Swipe /
Onion skin buttons, and all 26 historical theme selection fills are restored;
five targeted headless UI checks plus visual/scoped quality checks passed. These
changes are not yet in a newly qualified app. Archive-reader **50/0/0** is
fixture-scoped. A separate exact-ZIP direct-plugin OLD/NEW pair reduced counted
returned bytes by 163.41× (index 761.5608/92.0232 ms), with stable ordered root
metadata. It was unoptimized, OLD-first, cache unknown/warm and contended; this
is neither physical I/O nor isolated/native-capture/GUI opening qualification.

The subsequent five-library gate passed **275 unique headless tests / 0 failures /
0 ignored**; 325 executions include 50 repeats. This covers host capture, plugin
and web-host boundaries, typed comparison identity and the preceding Viewer
adapter/document slice. Scoped strict Clippy/format passed after owner repairs.
Final native PDF **2/0** used Viewer harness `f00316db…` with the real C PDF
worker and left no child workers; the app was unchanged. The earlier `b43d0280…`
pre-lint native run is separate evidence. These functional captured-file fixtures
do not establish production native-filesystem capture, app input or performance.
Viewer/comparison source closures were then released. A later configured-route
gate passed **77 / 0 / 2** (passed/failed/ignored), strict Clippy/format 0, with no
source drift and unchanged app. Architect/Perf review found no further actionable
defect. This overlaps prior comparison checks; do not add the totals.

<a id="p3-ui-packet-results"></a>

The canonical P3 UI packet is promoted: **104 source files /159 source-derived
selections**, not 159 executed passes. The continuation passed **31/0** (MainUI 28,
Viewer footer 1 and keyboard 2); Root independently rehashed/read all seven exported
raw logs. With the initial 7/1, the selected 39 is **38 passed /1 Options-init-focus
failure**. The former footer failure and both keyboard tests pass on this snapshot.
The older MainUI/CompareUI 55/0 retains its own source and overlaps these scopes.
P3 MainUI units initially recorded 20/1; after the fixture precondition correction,
the exact draft selector passed once. The other 20 were not rerun, so there is no
fresh 21-pass claim. Options focus still fails: the trace/screenshot show OK focused
while the expected pattern field is visible. The diagnostic compile qualified with
zero warnings/errors, 1890 source guards and 323 feature sets; app/bindings unchanged.
The earlier Viewer 46 timeout and subsequent 45/1 retain their dated source.
On the canonical `fc4fcdf7…` shared-helper checkpoint, the exact former failure
passed first and the other 45 then passed: **46 actual passes /0 failures**.
Both raw logs retain their separate 1+45 counts; all nine document cases and
19 image-presentation cases passed, including the genuine immediate-open errors.
These overlap earlier scopes and are not additional unique passes or all 159.
Options focus, native/full-app and production migration acceptance remain open.

<a id="faithful-native-comparator-fixture"></a>

The corrected faithful native comparator fixture ran **4 actual Windows cases /
0 failures** (`comparator-d6ce28b8eb`): raw parent/argv/cwd, NotStarted cleanup,
preworker cancellation/two session credits/no child, and non-Unicode Automatic
fallback. Components were unchanged and every case left remainingOwned[]. This
closes the old fixture settings-timeout for those cases; historical failures remain
recorded. Fixture 16 blocking/6 resident/10 transient-holder qualification does not
implement the production-pool repair or supply native ReadAt counts.

Built-in Office contents preview remains in development:
DOCX/XLSX/PPTX/ODT/ODS/ODP are the six-format goal; XLSX was the first slice.
The coherent six-format service 27-file and Viewer first-content/native fixture
six-file freeze compiled cleanly and passed **113/0/0**: service lib 68, XLSX
integration 18 and Viewer adapter/frame-guard 27. The later coherent CSS/reader/
diagnostic checkpoint compiled cleanly and passed **86/0/0**: 41 HTML/format +18
XLSX +27 Viewer Office/frame-guard checks. These overlap 113 and are not additive.
The subsequent `favicon-lint-1` checkpoint compiled cleanly, passed strict Clippy 0
and formatting 33 files 0, and passed **60 scoped checks: 18 XLSX +9 HTML +33 Viewer**.
These overlap earlier 86/113. The native Source acceptance harness is freshly
compiled after the mechanical lint repairs but **has not run**.
These overlap earlier scopes and do not establish complete native/app support.
Initial readable contents on open is the approved contract;
an index-only opening is not the accepted final UX. Preview means readable
contents, not faithful Office layout, using bounded inert HTML without external
Office, macros or formula execution. Native Office correlation/cancellation/path
policy acceptance remains open. The latest native navigation case **passed** all 11
content proofs, with nine expected generated requests/grants and empty denied/
unexpected sets. The unchanged allowlist passed after the approved generated
data-icon head prevented favicon requests. Parent-change then **failed** with
`native browser exited before requested evidence`; close/cancel is unrun. This is
**1 native pass /1 failure /1 unrun** in that immutable receipt. The separately
authorized third close/cancel case later passed once on the same old frozen
harness, giving that pre-promotion source two actual passes and one parent-change
failure across separate runs. No earlier case was rerun to obtain that result.
Host 5 changed files and coherent Viewer 3 are now canonical after Architect/Root
review (freezes `d9d1dedb…` / `29e79f51…`). Their initial 16-new-Host/18-target-Viewer
plan was subsequently qualified within **47 Host +44 Viewer =91/0/0** on that
preserved source.

<a id="office-parent-diagnostic-9fe36f20a6"></a>

The latest parent diagnostic `office-9fe36f20a6` remains **0 passed /1 failed**:
completed real Find returned 0, Copy failed and Session stayed live. Its blank 902×602
diagnostic frame is not validated fresh content/resize proof (resize_ok=false,
fresh_contrasted_frame=false). Source refusal remains required; no provider/Find/
loading root cause is established. Host 47 +Viewer 44 =91/0 retain their frozen source;
the later four assertion-style lint fixes passed strict Clippy 0/format 13 files 0 and
two overlapping resource tests, not 93 unique tests or a full 91 rerun.

<a id="small-body-navigation-and-office-v3"></a>

Root accepted the generic provider small-body navigation contract for private
implementation: captured main-frame GET/no-post targets of 0..1MiB are completely
admitted through genuine Resolve/ResourceRead/Data before replacement navigation;
larger targets retain streaming through the same resolved capability. Small-body
failure cannot fall back to streaming/old bytes. Preserve one body credit through
actual response release, exact one-shot request binding, a 30-second admission
deadline and current authority checks. This changes no public API/SDK or dependency
pins. The strong retained-document boundary applies to eligible small targets;
large streaming retains its late-Read-denial limitation. Office V3's exact 16 sources are now canonical (freeze `10d0a5e6…`), with
132 nonowned guards unchanged and public API/SDK, Cargo, executable and runtime
preserved. The unchanged pure navigation core `c6596c4c…` passed **34/0**, compile 0;
the existing CEF version-metadata manifest warning remains, so this is not a
zero-warning gate. These tests exclude CEF/native execution and do not qualify
new V3 route/adapter integration. Five native cases await a legitimate complete
new C product; no new executable or deployment is claimed.

Following the earlier three passes
and one compression-level fixture setup failure, the amended **four canonical +
two private cases passed / 0 failed**. Seven result records include the private
compiler. Actual method 8 budget rejection and same-session second-sheet recovery
passed. For a 10,000-entry fixture with 1,048,512 name bytes, observed requested
Rust heap increments were 11,322,758 B open peak, 6,013,594 B retained, 192,183 B
first-render peak and 0 after session drop. These require allocation quiescence;
they are not RSS, exact concurrent peaks, universal 32MiB bounds or leak proof.
Timings are instrumented/contended, not a broad speed result. The production
service freeze is released. Initial-target/adapter headless qualification is
included above; native qualification remains partial/open. The earlier
performance-test Clippy 101 belongs to its own target/snapshot; the fresh 60 scoped
Clippy 0 is not an all-targets quality claim.
Immutable local API/SDK 0.2 and the host/engine preparation gate are verified
(98 selected passes); Source has the scoped post-review results above, and
canonical packaging now passed 9/0/0 with scoped Clippy/format 0, exact old 0.1/new 0.2
archives and fresh-cache sparse-consumer resolution. This goes beyond the private
registry proof, but is not plugin execution or a fresh remote-CI/app result. PDF
rule-local red migration passed its fresh full 20-test suite. The new targeted
Engine bridge gate passed **141 unique tests / 0 failed / 0 ignored**: five
configuration integration cases and 136 library cases. Compilation has zero
warnings, post-test source drift is empty and the app is unchanged; the earlier
conversion compile error is corrected. Its five initial private-preferences
`result_large_err` findings are now closed by a separate repair gate: **5
configuration +22 preferences passes / 0 failures**, tests/strict Clippy/format 0,
zero compile warnings and clean source/app guards. The 141 passes retain their
earlier source identity; these overlapping 27 are not 168 distinct Engine tests.
The pre-merge viewer work branch recorded **330 payload slots / 8 waiter tokens**, with
all cited Architect source identities independently matched. Older 324/7 figures
remain dated history. Comparison credits 2, desktop capture slots 6 and process
trackers 8 shared/16 openers are separate limits within their stated ownership
contracts; they do not create another retirement domain. See
[Engine lifetime](developer/ENGINE_LIFETIME.md#capacity-and-ownership).
Native process, GUI and Explorer qualification remain open; the
exact three-hunk Explorer composition is applied/reviewed source. Descendant-style transport and new
UI controls/config still need integrated qualification. User help for the new
workflows is in [comparison](developer/FILE_COMPARISON.md),
[viewer preferences](developer/VIEWER_PREFERENCES.md) and
[Find](developer/VIEWER_PRESENTATION.md#find-in-development). The configured
comparison gate above consumes the 27-file freeze with 30 newly authored tests;
native/UI acceptance remains separate.
[Active work](ACTIVE_WORK.md) keeps the qualified scopes and remaining work.

The normal Source producer qualified cached reuse of 16 fresh Cargo artifacts and
the unchanged `9915dfda…` DLL, with 46 source identities and SDK/API 0.2/cache/profile
guards checked. This was not code recompilation, DLL loading/deployment or native
Source acceptance. The four actual comparator passes above retain their faithful-fixture scope.


<a id="resident-worker-design-and-production-pool"></a>

The accepted private resident-worker design is **13 Engine /5 Viewer /2 Compare**
resident threads, keeping 2 async/16 transient blocking. These are planned domains,
not implemented threads, process measurements or new payload/waiter capacities.
Preserve worker affinity, distinct external ownership, checked retirement and current
window/session credits. The later old-scheduler run `production-pool-7e458befac` is genuine production RED.
With the original 2 async/16 blocking threads, 13-resident readiness and four windows,
the prepared-save callback had not entered and the second Compare was incomplete
at the original 20,000ms pressure boundary. Closing two Viewers recovered the same
prepared save (exactly one callback, coherent raw publication) and the same Compare.
Strict cleanup and exclusive capture-reader reopen checks completed before the
intended progress-failure assertion; no owned processes/monitor errors remained.
All 3,941 guarded identities were unchanged; the unconsumed 4917→F5 test-fixture
exception is separately recorded. Raw test time was 23.02s. This establishes the
old-source failure and same-work recovery, not migration GREEN, an unrecoverable
deadlock or a performance result. The earlier missing-plugin setup failure remains
historical and distinct. The old-source production-pool attempt `production-pool-27587e6553` reached real
watch/report and 13-nonwindow readiness, then failed Viewer setup because no compatible
Basic Viewer plugin was installed beside the test executable. The raw test ended in
0.23s before four-window pressure, the 20-second pressure deadline, pre-rescue counters
or recovery. Cleanup/exclusive-open checks preceded the failed outcome assertion;
remainingOwned[] and source/harness identities were unchanged. This is a setup
failure, not a production-starvation reproduction or a progress pass. Production
implementation promotion stays held.
The catalog-only host constructor is canonical; the old scheduler and dependency
pins are unchanged. Its six selected catalog/startup checks actually recorded
**5 passes /1 fixture-precondition failure**: constructing a dot alias had already
normalized the path, so the test failed before the product validator. The exact
raw-OsString test-only repair is canonical `f5d09b3b…`; replay is pending. Root's
44,433-byte `eb391200…` consumer remains canonical with unchanged registration
parent `e3a869…`. The later genuine pressure RED exercised qualified catalogs through this constructor;
the exact F5 alias test replay and migrated-consumer acceptance remain open. The
prior missing-plugin setup failure is a separate historical attempt.

<a id="round-two-common-worker-qa"></a>

Archived private common-worker round-two QA passed **13 default-feature-off tests**
and **24 resident-feature-on tests**. The latter follows exactly three unit-channel
type annotations (18 test-only bytes); production `resident.rs` remains
`60908d11119f…`. Retain the original feature-on test-build E0282 failure separately.
These are isolated immutable round-two snapshots, not final round-four, migrated
consumer, native, performance or production qualification. In particular, the
known repeated-panic-retention finding is not closed by these passes.
The faithful native fixture 4/0 is separate and cannot qualify this production repair.

<a id="viewer-compare-live-appearance-contract"></a>

Root accepted the Viewer/Compare live-appearance contract for private
implementation; it is not implemented or passing. Application UI font is separate
from document font. Frontend apply_appearance consumes actual applied/preview Theme,
density and UI font scalars for existing/new windows; backend restore_palette accepts
foreground/background/selection/accent only. Accepted revision is not painted/native
acknowledgement. Bounded latest requests, A→B→A, rollback and frame-generation fences
must preserve source/layout/position without recapture, relaunch or source I/O.
Source/Markdown semantic roles must recolor active/cached content, gutters and raster;
Office/author HTML CSS and original image/PDF pixels remain content-owned. Reuse
supported UI controls and preserve input/accessibility; no SDK expansion is selected.

The accepted opening-palette amendment adds host `Session::palette_revision()`
and `WindowSession::palette_revision()` getters of the current accepted revision.
They remain readable after close/disconnect, perform no admission or mutation and
are not paint acknowledgements. Frontends merge the four actual app colors before
open, then seed opening fences from the real getter before publishing/consuming
the session; `restore_palette` is for later changes. Preserve document fonts and
typed terminal ExternalStarted/Unknown outputs. Do not hardcode a universal initial
revision: current Viewer and Compare conventions differ. This is accepted private
implementation in progress, without Core DTO/public plugin SDK changes or passing
integration evidence.

The newer immutable resident common round-four checkpoint `6c04f591…` passed
**29 tests /0 failures**, with 13 default tests filtered out rather than rerun.
Production source is `31178fbd…`, tests `fb306359…`; raw runtime was 0.20s and
compilation completed in 4.05s with no warnings. This is separate from archived
round-two 13/24 evidence. It does not qualify Engine/Viewer/Compare integration,
native cleanup, performance or the full resident migration.

Private Slint V2 generation passed all three build-script steps with zero
errors/warnings and 194 input guards unchanged. Main's ordinary and tooltip passes
finished in 57.438s and generated 63,596,709 bytes of Rust; Viewer/Compare each took
about 1.3–1.4s. These are code-generation observations, not a performance benchmark.
Generated Rust is uncompiled; there is no input, paint, palette-acknowledgement or
full application result.

The Architect accepted the Engine's additional 200ms pool-exit observation only
after existing stop/abort controls and actual resident-ticket drain, followed by
tree-root shutdown. It stays inside Explorer's original shared absolute 2,500ms
deadline and cannot reset/extend it or shorten actor observation. Expiry means
unconfirmed retirement and detaches the checked waiter; it grants no cleanup or
resource credit. The original supplied transient runtime stays time-enabled and
driven. This is an accepted private contract, not integration/runtime evidence.

The full batch and retained 54-requirement goal remain open.

Cleanup is complete. C removed 10 obsolete QA installations: **1,826,040,781
logical bytes**, with an observed free-space increase of **1,168,015,360 bytes**.
D removed obsolete caches totaling **659,051,203 logical bytes**; observed free-space
increase was **577,359,872 bytes**. NTFS compression of historical captures reduced
their allocation by **3,078,541,312 bytes**, retaining paths, contents and timestamps.
Concurrent writes mean whole-volume deltas are not exclusive cleanup attribution.
Root's later observation was approximately 21.3 GB free on C and 27.9 GB on D; the
C pagefile itself was 23,426,166,784 bytes and was not touched. These are time-scoped
observations, not additional cleanup savings. The runtime and D executable were
preserved. Cleanup adds no EXE qualification, Git integration, Office success or
completion of the active 54-requirement goal.

The integrated 12 November panel batch adds bounded tree publication/command
admission, captured native actions and root-level native directory comparison.
Ctrl+D clears every mark, including folders and expanded descendants; Ctrl+Shift+D
has no default assignment. The filename palette has twelve categories, including
a separate coral PDF rule. The [panel qualification](verification/2026-11-12-panel-interaction-polish.md)
records the final debug executable, targeted Windows/Linux checks and startup
smokes; it does not establish full-workspace/remote CI, accepted elevation prompts
or completion of the broader feature goal.

The [pixel-scrolling contract](developer/SCROLLING.md) distinguishes fractional
movement, native input routing and actual frame cadence. Native observations and
known performance limits are recorded in the [scrolling investigation](NATIVE_SCROLL_VERIFICATION.md);
intermediate offset tests alone do not establish 60 Hz scrolling.

The 11 November scrolling update applies fractional wheel coasting to panel and
standard-list scrollbar paths, shares input handling across tool windows, and
reuses visible-row and GPU rendering resources. The Windows System32 panel fixture
has p50/p95/max redraw gaps of 16.531/20.024/21.947 ms. Text, source, Markdown and
hex viewers now move bounded retained pixels between backend page replies, with
an explicit camera handoff when a partial long line requires viewport rendering.
Cached movement improves, but high-DPI refill and idle restart still have measured
delays; see the
[viewer qualification and remaining work](verification/2026-11-11-viewer-pixel-scroll.md).
Exact JSON/XML wheel and drag regressions now pass on GPU and CPU; GPU startup
failures also recover once into software before the first window reveal. The fresh
debug application passed backend, software-UI and native-UI startup
checks with its protected plugin runtime. Linux compilation passed; Linux native
frame pacing has not been qualified in this update.

The 19 November kinetic-scrolling update adds touchpad momentum on Linux (X11 and
Wayland), where the system reports none after the lift (Slint's own Flickable
fling on a Wayland lift is replaced, so there is one model everywhere). One shared
scroller (`KineticScroll`, owned per window by `midnight_ui_input::NativeScroll`)
estimates the finger's final speed, continues a flick with exponential friction,
stops at once on a click, key, touch, wheel notch, modifier change or pointer move,
and stops at content ends without overshoot. It drives the file panels, every
viewer mode (text, source, Markdown, image, PDF, and the HTML surface through its
native wheel path) and the comparator, only over content that scrolls along the
flick; Windows and macOS keep their own system momentum. On X11 only a clear,
fast flick coasts (there is no lift event), and high-resolution mouse wheels stay
notches. X11 touchpad line streams reach panels and viewers without wheel easing,
and comparator wheel notches glide (not with Ctrl, Alt or Super held).
Software-window tests cover the physics and each surface family; see
[kinetic scrolling](developer/SCROLLING.md#touchpad-momentum-kinetic-scrolling).
After hands-on feedback on 20 November the glide is longer everywhere: the decay
time constant rose from 0.35 to 0.5 s and then to 0.8 s (more than twice the
original travel), and frames slower than 16 ms no longer slow it down (each
advances its real time, up to 50 ms).
Not verified: the feel on a physical touchpad, the X11 heuristics against real
touchpad and TrackPoint timing, and HTML momentum inside a live browser engine.

The 12 November follow-up adds XML-family syntax highlighting, previous-file
navigation (Backspace/Alt+Left), selected-item panel navigation (Alt+Up/Down),
and Salamander view shortcuts (Alt+2/3/5). Saved bindings remain authoritative.
Custom Commands now have compact ordered forms, separators, explicit terminal
execution and managed 128-pixel icon copies. Find actions and native tree metadata
use captured identities. [Current debug evidence](verification/2026-11-12-panel-controls-and-notices.md)
records startup and targeted integration scope. Fresh foreground viewer tests pass
the original guards, but measured refill/drag gaps remain; see the
[foreground qualification](verification/2026-11-12-foreground-scroll-resume.md).
The subsequent retained-seek change reuses complete-page surfaces during JSON/XML
scrollbar dragging. Its matched native probes reduce maximum content gaps to
24.624/41.918 ms respectively; XML p95 slightly worsens and partial Markdown still
has roughly 33 ms content intervals. See [exact scope and evidence](verification/2026-11-12-retained-complete-page-seek.md).
Native Custom Commands from expanded trees now use captured provider identities
and independently retained focus. The Windows gate passes 63 tests; Linux targeted
gates also pass, including actual native process launches and post-pin identity
checks. [Tree command qualification](verification/2026-11-12-tree-custom-commands.md)
records exact scopes and fixture corrections. A final early-rejection cleanup
follow-up passes 19 cases on each OS, and four actual-window/native-process
integration cases pass against that final implementation. The rebuilt Windows
debug application and corrected SVG/AVIF image plugins have a protected runtime;
[debug integration evidence](verification/2026-11-12-tree-images-debug.md) records
its hash, startup checks and one initial transient integrity refusal.
HEIC/HEIF remains unsupported.

The settings follow-up adds ordered wildcard/regex editor associations, a recent
panel-filter history, measured metadata column widths and Ctrl+Shift+P for the
focused native full path. Options has a pinned plugin rescan action, alphabetic
themes and a 96 px thumbnail default; existing explicit preferences are preserved.
Forms share their themed title/icon/divider, including compact Rename. See the
[settings qualification](verification/2026-11-11-settings-ui-follow-up.md).

The Markdown follow-up fixes large-table scroll anchors and adds BAT/CMD syntax
highlighting. Rich tables that exceed the bounded document transport use wrapped
plain presentation instead of partial table geometry. Catalog discovery now runs
in parallel and starts before native viewer construction. This improves measured
loading paths without removing plugin trust checks. Current timings and remaining
high-DPI/GPU costs are recorded in the [scrolling](verification/2026-11-11-viewer-scrolling.md)
and [image loading](verification/2026-11-11-image-loading.md) reports.

The accepted 12 November Markdown allocation slice (`cc4857e`) borrows retained
semantic context and clones only selected complete layout groups. Its independent
plugin handoff records 62 passed tests, one ignored manual profile and identical
saved response streams. The lower bounded allocation does not establish an
end-to-end latency or native frame-pacing improvement; [Active work](ACTIVE_WORK.md)
records the measured scope and remaining host/native qualification.

The current transfer update is available in the qualified Windows debug build:
whole-operation speed charts, native same-volume Move without
content copying, and Options → General full SHA-256 verification (default on).
Its optional quick mode checks size and first/middle/last 64 KiB, or all bytes up
to 192 KiB. The immutable mode also covers clipboard/drop/tree/archive requests.
See the [transfer contract](superpowers/specs/2026-11-10-transfer-verification-and-native-move.md)
and [speed-chart evidence](verification/2026-11-10-operation-speed-charts.md).
Compact Windows staging names and cancellation cleanup also have native Windows/
Linux, queue and UI coverage; see the [final debug qualification](verification/2026-11-10-staging-cleanup.md).
The [folder-drop update](superpowers/specs/2026-11-10-folder-drag-and-native-handoff.md)
adds exact native folder targets, tree captures and volume-based Copy/Move defaults.
Windows native handoff now passes a real owned-window success/cancel/success test.
Actual Shell targets also pass same-volume Move/Ctrl-Copy and cross-volume
Copy/Shift-Move for files and nested directories; see the [dated qualification](verification/file-drag-drop.md#2026-11-10-folder-targets-and-windows-shell-move).
Linux outgoing transport remains Copy-only on X11/XWayland; native Wayland
external dragging is unavailable. An incoming drop that showed the Copy cursor and
is then refused (an unreadable or over-limit file list, failed admission, a changed
destination or a file list the source does not deliver) is reported once on the
status line, on Windows and on Linux X11 alike and in fixed texts; a Move-only offer
or a drop that never showed Copy is refused silently
([native boundary](../SECURITY.md#file-drag-and-drop)). Integrated component checks and debug deployment
are complete; an end-to-end drag from the final Slint app into a running Explorer
window still needs separate native qualification.

The 9 November follow-up adds [Make File List](MAKE_FILE_LIST.md) on Ctrl+M,
using captured native/archive/inline-tree metadata without member extraction, and
[user viewer associations](VIEWER_ASSOCIATIONS.md) in Options > Plugins. Markdown
scrolling now reuses semantic pages, fixes incorrect line/block limits and backward
table overlap, and has a proper GitHub block layout. See the dated
[file-list](verification/2026-11-09-make-file-list.md),
[associations](verification/2026-11-09-viewer-associations.md) and
[Markdown native verification](verification/2026-11-09-markdown-scroll-and-style.md).

A further Markdown correction preserves complete bounded paragraph/table context
across overlapping scroll requests and reuses parsed context for nearby positions.
GitHub selection resets its body typography to platform system-ui at 16 px;
tables use content-aware columns, intact numeric tokens, correct body striping
and stable physical-pixel borders. See the [continuity evidence](verification/2026-11-09-markdown-continuity.md)
and [font/table checks](verification/2026-11-09-markdown-font-tables.md), including
the actual report-file regressions missed by the earlier exact-anchor test.

Inline tree background checks now batch without false queue-exhaustion errors;
Network follows the drives before local special folders on both navigation surfaces.
Ctrl+A selects only top-level files; Ctrl+Shift+A includes top-level folders.
Both replace selection and clear nested marks. See
[tree scheduling](verification/2026-11-09-tree-budget.md) and
[tree selection](verification/2026-11-09-tree-select-all.md).

Adopted inline trees now retain literal prefix search and optional quick-search
filtering over their loaded rows, composed with persistent filters and sibling
sorting. Archive extension sorting uses original UTF-8 member names and locale
keys. See the [current scope and qualification](verification/2026-11-10-tree-prefix-search.md);
captured native operation consumers are summarized below. Native manual refresh and
batched watch notifications now reconcile the adopted tree in-session; see the
[watch contract](developer/TREE_PANEL.md#native-watches-and-reconciliation) for
scope and bounds. Archive watches and broader tree workflows remain separate.

Native clipboard routing from adopted trees is implemented in source: Ctrl+C/Ctrl+X
capture exact selected or focused native occurrences, and Ctrl+V retains the panel
root destination and ordinary verified Copy/Move behavior. Windows presenter and
real-Engine UI bridge tests pass, as do Windows/Linux backend libraries. The UI
bridges substitute the clipboard boundary; Linux UI and native desktop clipboard
interoperability remain unqualified. Archive/mixed clipboard export stays unsupported.
See the [clipboard contract](developer/TREE_PANEL.md#native-clipboard-from-expanded-panels)
and [measured scope](verification/2026-11-10-tree-clipboard.md).

The settings polish merges Panel/Command into General, reuses ordered argument
editors and compact priority actions, and separates viewer/filesystem catalogs.
Panel metadata now aligns date and time independently; Windows Hidden/System
attributes dim file and folder artwork. Ctrl+A selects top-level files only;
Ctrl+Shift+A includes folders. Ctrl+D clears all selection, including folder
and expanded-descendant marks; Ctrl+Shift+D has no default assignment. Ctrl+Shift+I inverts top-level
files and folders. F10 reads effective shortcuts from current settings, including
remaps, removed aliases and restored defaults while the menu is open. See the
[current deselection/menu qualification](verification/2026-11-12-panel-interaction-polish.md),
[earlier live-menu verification](verification/2026-11-11-selection-and-live-menu.md)
and [settings qualification](verification/2026-11-10-settings-polish.md).

Native, archive and expanded-tree size scans now retain clickable yellow warning
reports. Red listing errors expose full details separately from Retry/Collapse.
Tooltips show the actual reason; clicks and Copy preserve the captured occurrence
and complete diagnostic. See the [contract](developer/PANEL_DIAGNOSTICS.md) and
[qualification](verification/2026-11-10-panel-warning-details.md).

Default filename coloring now covers twelve categories with contrast adjustment
for every theme and selection/focus state. PDF has its own coral base color;
DOCX and Markdown retain the brighter neutral document base. Exact historical
defaults migrate; custom rules remain unchanged. See the [current palette contract](developer/FILENAME_COLOR_OPTIONS.md)
and [PDF/palette qualification](verification/2026-11-12-panel-interaction-polish.md).
Readable sizes use KiB/MiB/GiB/TiB for the existing 1,024-based conversion, and
Reverse order is separated in the sort menu; the [older palette/units report](verification/2026-11-10-filename-palette-and-units.md)
records that earlier batch's scope.

Copy and Move now deselect successfully completed roots progressively, including
verified archive extraction. Skipped, failed and unfinished roots stay selected;
folders clear only when their complete subtree succeeds. See
[transfer-selection verification](verification/2026-11-08-transfer-selection.md).

Copy/Move also has versioned Retry/Skip/Cancel I/O error decisions with safe
retained-object checkpoints. Automatic progress closes on success; manually
opened Jobs and session history remain. The 11 November follow-up separates the
Jobs ledger from per-operation Copy/Move/Delete progress, shares the progress/speed
chart UI, and routes X/Escape to the exact operation. The 12 November follow-up
adds confirmation before cancellation, with Continue as the default; completed
progress needs no confirmation. Pause/discovery and native
window regressions passed integrated Windows/Linux affected suites and Windows
native/software startup checks; see the
[qualified build](verification/2026-11-11-settings-ui-follow-up.md) and
[progress contract](developer/OPERATION_PROGRESS.md). Unsupported checkpoints cannot be
retried optimistically. See [error recovery verification](verification/2026-11-08-transfer-errors.md).
Since 20 November Delete (Trash and permanent, panel and expanded tree) pauses on
each item it cannot remove with the same prompt: Retry, Skip (the item and the
folders that contain it stay, reported once per target), Skip all errors and Cancel
job, and goes on with every remaining item and target; its errors are counted per
failure. On Linux a write-protected folder that the user owns inside the confirmed
tree gets owner write and search permission before it is emptied (original mode back
if it stays), so it no longer stops the delete; the containing folder and other
users' items are never changed ([security](../SECURITY.md)). Covered by engine,
Linux platform and presenter tests only, without installed qualification. Windows
offers decisions for whole items only and does not clear read-only attributes yet;
Move's cross-volume source removal does not yet make protected folders writable;
Jobs rows need an attention mark and Resolve for waiting Delete decisions (job
window follow-up).
Also since 20 November, on Linux a file or folder inside a copied folder that
cannot be opened pauses on its own (stage Reading the source): Retry opens it again
by name, Skip leaves out only that child and the folder's enumeration goes on, so
an unreadable subfolder no longer drops the rest of its root; a folder whose names
can be read but not opened still fails once as a whole. A same-volume Move of a
folder the user owns that lacks owner write gets owner write through its retained
descriptor only for its renames and its own mode back afterwards, and a prepared
Move source is rechecked by identity, size and modified time, so permission changes
(for example the fix before Retry) no longer invalidate it. Covered by engine and
Linux platform tests only. Windows and archive sources still report a child that
cannot be opened as a failure of its folder. One of several selected items that
cannot be prepared (for example a folder nobody may read) offers Skip and Skip all
besides Retry, and the other items are copied; captured tree and archive sources
and a single item keep Retry and Cancel only. Ordinary cross-volume Move children
that cannot be prepared and halts after an uncertain commit or cleanup remain as
recorded in [the Copy/Move audit](developer/OPERATION_PROGRESS.md).
The 11 November retry correction retains one owned pre-copy stage across Move
retries, keeps failed-cleanup paths in bounded history, and refuses a fresh
metadata preparation attempt after cleanup failure. Ordinary recoverable errors
still offer Retry; see the [regressions and limits](verification/2026-11-11-transfer-stage-retry.md).

Native/archive inline trees also consume typed targets for directory sizes:
selected folders or all displayed folders (including expanded descendants) without
selection, exact-folder Space,
coalesced progress, known selected totals and explicit size sorting. See the [size verification record](verification/2026-11-10-tree-folder-sizes.md)
and [expanded-folder correction](verification/2026-11-10-expanded-folder-sizes.md).
F3/Alt+F3 also retain root-file viewing while branches are expanded. Viewer capture
uses the exact root aliases and visible order, preserving native/archive source
authority; inserted branches contain folders, not descendant-file rows. See the
[viewer integration contract](developer/TREE_PANEL.md#viewers-from-expanded-panels)
and [Windows/Linux qualification](verification/2026-11-10-tree-viewers.md).
F4 now also captures the focused native regular file after tree adoption, using
acknowledged occurrence identity and backend-owned lifetime/cancellation. It does
not enable directory/archive editing or remove guards from other tree actions;
see the [external-editor evidence](verification/2026-11-11-tree-external-editor.md).
Native tree Delete/Rename/metadata, checksums, Custom Commands, Properties/context
menus, clipboard/drag and native comparisons also use captured targets; Make File
List exports captured native/archive/tree metadata. Find retains the native pane
root and its separate captured result actions. These do not enable archive writes,
virtual clipboard/drag export, archive Find/watches or container attachment/nesting.
Literal focused-name insertion into the command editor remains guarded after tree
adoption. Complete installed/native qualification remains separate; see the
[per-action contracts](developer/TREE_PANEL.md) and [active requirements/evidence backlog](ACTIVE_WORK.md#requirements-and-evidence-backlog).
Network Neighborhood and viewer requests retain their separately recorded scope. The
[current follow-up contract](superpowers/specs/2026-11-07-panel-viewer-followups.md)
and current ledger track work in
progress separately from released behavior, including archive Hot Paths qualification.

Markdown now renders pipe tables with aligned, wrapping cells and borders,
strikethrough, and read-only task markers. Its font chooser lists installed
families and retains the Markdown family/size independently of text/source
preferences. See [verification and limits](verification/2026-11-08-markdown-tables-and-fonts.md).

Fixed UI timers no longer run while the main window is idle, except the drag
bridge's 250 ms safety net: the bridge polls every 16 ms only while drag work is
pending, window geometry is sampled only after native geometry events, and the
folder-size, thumbnail and retry pulses run only while they have work (the thumbnail
service wakes its pulse for frames, its cache location and notices). Indicative
Windows measurements (debug build, software renderer, mostly a locked session) fell
from about 80 to about 10 idle wakeups per second with the window active; the rest
is the caret blink of the focused filename search and that safety net
([idle wakeup record](verification/2026-11-18-ui-idle-wakeups.md), UI-PERF-01 in the
[backlog](IMPLEMENTATION_BACKLOG.md)). The file comparator presents output on worker
wakes instead of a 16 ms window timer, and idle clipboard sessions and open
drag/drop sessions wait for operating-system events instead of pumping every 10 ms
([engine lifetime](developer/ENGINE_LIFETIME.md)).

| Feature | Current implementation | Remaining work / design |
| --- | --- | --- |
| HTML viewer | Required independent Chromium/CEF process plugin, captured native/archive assets, in-window rendering, find/zoom and explicit offline/online controls; default HTML/HTM/XHTML associations are configurable | Complete matching CEF package required. [Windows installed verification](verification/2026-11-10-html-viewer.md) and [Ubuntu 24.04 WSLg XWayland host/Slint qualification](verification/2026-11-10-html-linux.md) pass. Linux requires Chromium sandbox plus existing delegated aggregate resource limits; where AppArmor restricts unprivileged user namespaces (Ubuntu 23.10 and later), the viewer refuses to start Chromium without its sandbox and says why in the page's place. Where `/usr/bin/pkexec` and its folders belong to root it offers **Install…** (keyboard reachable), which installs the profile through polkit with one constant root command that runs no file of the installation, then opens the document again; nothing is checked or installed before such a refusal. Otherwise it explains how to install its profile with `install-html-viewer-apparmor.sh`, naming the exact command only for a root-owned installation ([system requirements](SYSTEM_REQUIREMENTS.md), [root command](../SECURITY.md)). Install… is covered by unit tests only: profile text against the script's `--print`, a fake pkexec runner, and its root command run unprivileged in a temporary folder; a real polkit prompt and kernel load were not exercised. The WSLg qualification had no such restriction, and the script's root install was not run in its [record](verification/2026-11-20-viewer-speed.md#html-viewer-under-apparmors-user-namespace-restriction); native Chromium Wayland and physical IME input remain unqualified (IME composition/commit now pass CEF's required invalid replacement range; a synthetic IME commit is covered by the opt-in native fixture). The host session blocks until there is work instead of polling: with real Chromium on Windows, an idle open document caused about 174 host context switches per second before this change and under 1 after. The provider itself still waits at most 2 ms between Chromium message-loop pumps while a document is open, about 64 pumps per second on an idle Windows document; Linux was not measured ([idle-wakeup record](verification/2026-11-18-html-idle-wakeups.md), VIEW-04 in the [backlog](IMPLEMENTATION_BACKLOG.md)). [API and security contract](developer/WEB_VIEWER_API.md) |
| Native panels | Windows and Linux browsing, tabs/bookmarks, multiselection, locale sorting, watches, selection-aware directory sizes, three view modes, themes and configurable shortcuts. The rows still shown stay watched while a refresh or another native folder listing is pending and after it fails. An automatic refresh after a change keeps the rows on screen without the loading indicator, and when only a few rows changed it updates just those rows; a folder that keeps changing is re-read at most about once a second ([busy folder record](verification/2026-11-19-busy-folder-refresh.md)). When native change notifications cannot be registered for a folder, it is checked about every second while the registration is retried in the background, which switches to notifications silently once it succeeds; only when registrations keep failing for a minute (a tree: at once) does a line above the details bar say that live updates are unavailable and the folder is checked regularly (its tooltip gives the reason). On Linux all watches share one inotify instance; a watcher that fails restarts at once, then at most once a minute, and the line says automatic refresh is paused meanwhile. Neither is a panel error. Names and paths shown in rows, tabs, the path bar, dialogs, errors and status text, and in viewer and comparator captions and status lines, display the explicit bidirectional controls (U+202A–U+202E, U+2066–U+2069) as `\u202E`-style escapes, so a name cannot reorder its shown extension; operations, search, comparisons, the copy actions for paths, details and errors, and name and destination fields keep the raw name, while text selected in a shown field keeps its escapes ([raw identities](../SECURITY.md#current-browsing-and-explicit-operation-milestone)). Names drawn in rows, path components, tabs, the details bar and the single-name line of the Copy/Move and Attributes dialogs keep a left-to-right base direction (display-only isolation marks), so `עברית.txt` does not show as `txt.עברית` | macOS application integration/packaging remains pending; a separate read-only Darwin filesystem provider already exists. Archive and Network Neighborhood navigations still release the shown rows' watch when they start (WATCH-01 in the [backlog](IMPLEMENTATION_BACKLOG.md)). [Watch status](developer/PANEL_DIAGNOSTICS.md#watch-status) |
| Shared toolbar | One action row above both panels follows the active panel; includes navigation/history, Checksums, Folder Sizes, Properties, file/directory comparison, filtering and Hot Paths; View and Sort belong to each panel drive bar | Scrollable overflow preserves the 320×200 minimum; context changes and canceled input cannot retarget actions. Drives, paths and tabs remain per-panel. [Verification](verification/2026-11-08-shared-toolbar.md) |
| Inline folder branches | Production native/archive panels lazily enable occurrence-based branches, selection, disclosure and typed navigation; prefix/quick search, persistent filters and sibling sorting share acknowledged backend reprojection. Captured native actions include transfers, clipboard/drag, F4, mutations/metadata, checksums, Custom Commands, Properties/context menus and comparisons; file lists also accept archive/tree metadata | Search examines loaded visible rows without enumeration; filtering retains unselectable ancestor context and rebases selection atomically. Native refresh retains the tree. Viewers/file comparison use root files; directory comparison is native/root-only. Virtual exports/writes, Find rooted in an archive (Search inside archives covers archives under native roots), archive watches, container attachment, archives inside archives in expanded branches (flat panels open them) and focused-name command insertion remain separate gaps. [Panel contract](developer/TREE_PANEL.md), [latest qualification](verification/2026-11-12-panel-interaction-polish.md). These integrations do not establish full TREE-01 completion or native workflow parity. |
| Panel status | Filename or selection summary takes space before secondary metadata; Windows attributes use compact characters such as A/C/R with a tooltip legend; clicks copy full captured values | Linux details include symbolic and octal permissions, including special mode bits. Narrow panels elide only after assigning the available width to the name. [Verification](verification/2026-11-07-status-and-viewer-readability.md) |
| Properties | Alt+Enter or the toolbar: Windows opens its Properties dialog. Linux, which has none, shows a read-only summary: for one item its type, size, owner and group (names, or numeric ids), permissions and created, modified and accessed times, then for a folder its files, folders and total size counted on a backend worker; for a selection the counts and the total size of its files | The folder count follows no links, enters no other filesystem and stops when the summary closes ("Calculating…" until it arrives). Engine tests (`desktop_properties_tests.rs`) and the `files_navigation` acceptance scenario cover the summary; no dated record lists their results yet ([active work](ACTIVE_WORK.md#current-checkpoint)). |
| Compact main window | Exact 320×200 logical minimum at every density; active pane and other-pane rail, command editor and horizontally scrollable function keys; Options/Jobs stay visible | Regular two-pane controls return when space permits. See [small-window verification](verification/small-window.md) |
| F10 main menu | Configured actions grouped by Files, Selection, Panels/navigation, Tools and Application; category picker, icons and configured key hints | Reuses existing action/confirmation paths and availability rules; [verification](verification/2026-11-08-main-menu.md) |
| Window placement | Saved physical normal bounds, maximized/fullscreen state and monitor; unavailable monitors fall back to the primary work area; viewer windows inherit parent bounds and always accept Escape | Wayland owns absolute placement. See [window geometry](verification/window-geometry.md) |
| Dialogs | Help, Find/Checksums, Jobs, job progress and operation decisions use native modeless hosts; no native host is owner-modal. About, Options, notices, Make File List, Rename/Create/Delete, Copy/Move drafts, Panel Filter, Hot Paths and Attributes / Times / Case are modal popups embedded in the main window. Exact request/session guards retain drafts and reject stale callbacks; failed creation reports a visible owned notice. Escape, the window's close button (X) and the header close of a Copy/Move, Rename or metadata decision window close only that window, like Decide later: the decision stays pending, Jobs → Resolve reopens it, and only Cancel job cancels the job | Only modeless working tools require independent movement/taskbar access; ordinary dialogs have no separate taskbar item. Further embedded-modal migration is not required. The earlier native-host batch recorded 88 targeted Windows and 63 Linux checks, native Windows ownership/foreground checks and startup qualification; those are dated evidence, not final qualification of this follow-up. Linux compositor/cross-monitor checks remain separate. [Current policy](superpowers/plans/2026-11-11-dialog-polish-followup.md), [earlier host evidence](verification/2026-11-11-native-tool-windows.md), [ergonomics contract](developer/DIALOG_ERGONOMICS.md) |
| Shared Windows build startup | Matching protected deployment can supply viewers, thumbnails and archive providers to the original build EXE; normal Windows build helper deploys/registers it | Raw Cargo builds need explicit registration; restart after deployment. [Runtime discovery](developer/RUNTIME_DISCOVERY.md) preserves package/ancestor trust checks |
| Keyboard settings | Search, capture, conflicts and reassignment; fixed first-character crash caused by retiring hovered tooltip rows during Slint popup handling | Causal native release reproduction and corrected four-profile matrix passed 657 assertions. [Verification](verification/2026-11-07-keyboard-search-crash.md) |
| Drag selection | Rectangle selection from empty panel space in Details/Brief/Thumbnails; Ctrl/Shift, archive identities, edge scrolling and revision-checked backend ranges | Windows software input regressions pass; final Linux/native release qualification remains. Dragging actual rows retains native file drag/drop. [Selection contract](developer/SELECTION.md) |
| Plugin catalog | Options → Plugins lists Viewers, Filesystems and Thumbnailers; asynchronous Refresh includes installed image/video helper metadata and discovery errors | Thumbnailer overview is read-only; global controls stay on Previews. [Catalog contract](developer/VIEWER_PREFERENCES.md#plugin-catalogs-in-options) |
| Media thumbnails | Required in standard builds and release packages; independent image/video workers, private persistent cache, bounded decoding and selected-video animation; typed archive member captures share the same services. Protocol-3 cooperative staging cancellation is verified with installed ZIP/solid-7z workers, same-mount retry and preservation of cached members; all 11 archive format controls pass | Trusted helpers and stable archive member IDs are required. Images: PNG/JPEG/GIF/WebP/BMP/ICO/TIFF plus bounded static SVG and AVIF from the shared decoder; HEIC/HEIF is unavailable. PDFs: the image thumbnailer draws page 1 with the PDF viewer's Hayro 0.9.0 renderer (content-sniffed, at most 64 MiB read once); password-protected, damaged or unsupported pages keep the placeholder. The thumbnailer version changed, so cached image thumbnails are regenerated once ([PDF thumbnail record](verification/2026-11-20-viewer-feature-gaps.md); Windows and large real documents not run). [Image-format qualification](verification/2026-11-12-image-host-formats.md) records actual packaged-worker checks. Initial video codecs remain limited, with no MP4/H.264 promise. Actual container storage controls network opt-in. Native panel frame pacing, Linux desktop/archive-panel behavior, all-format persistent reopening and generic remote leases remain unqualified. Legacy protocol 1/2 drains admitted staging; protocol 3 waits for a bounded checkpoint/active native call, retaining real capacity until cleanup. [Verification](verification/2026-11-10-archive-thumbnails.md), [worker SDK](developer/WORKER_SDK.md) |
| Panel search/filter | Unicode filename prefix search and optional quick-search filtering in flat and adopted tree panels; per-panel Masks/Regex/typed metadata Expression filters on Ctrl+F12 compose with the prefix and sibling sorting. Backend validation preserves the last valid view. The embedded filter form retains its opening pane/navigation and draft across resize, UI refresh and same-folder directory refresh (watch, Ctrl+R, finished operations); navigating elsewhere closes it. Cancel and Show all keep their captured-context semantics | [Native Filter/Hot Paths checks](verification/2026-11-11-native-tool-windows.md) cover stale callbacks, draft preservation, compact first-open Escape and injected host-creation failure recovery through a visible owned Notice. Trees search loaded raw names; opaque archive captions cannot invent a match. One latest desired query and one in-flight options request preserve typing under queue pressure. Insert waits for cleared search/settled selection; hidden/context rows and stale carets cannot authorize keyboard actions. Options commit only on matching publications; Cancel discards and clear retains the persistent expression. [Syntax](developer/METADATA_FILTERS.md), [prefix scope and qualification](verification/2026-11-10-tree-prefix-search.md), [earlier filter evidence](developer/TREE_PANEL.md#filter-and-sort-evidence). Full installed tree workflows remain separate. |
| Show hidden files | Ctrl+H (`panel.hidden-files`), F10, each panel's View menu and Options → General hide or show dot-names (Linux/macOS) or hidden-attribute entries (Windows) in native listings, archive listings and inline trees; saved in settings schema 35, shown by default. A change re-reads both panels in place: the cursor moves to the nearest shown row, marks on hidden entries are dropped and the status line counts the hidden entries. Ctrl+A, mask selection, folder sizes without a selection and Compare directories use the shown entries; Find searches hidden entries too | Presenter, engine (listing, archive, tree, re-sort), settings-migration and window tests; the `files_filter_search` acceptance scenario covers the keys, menus, Options with a restart, cursor, marks, Ctrl+A, the status line and an inline tree ([feature evidence](verification/2026-11-20-hidden-files-and-mask-selection.md)). The Windows attribute policy is mirrored but was not compiled here. Network locations never hide entries. |
| Selection by mask | Ctrl+= / Ctrl+− (no keypad needed) and the numeric keypad's + / − open Select / Deselect files with the last mask (default `*`) and Include folders; masks use the panel filter's Masks syntax, ignore case and change only shown rows (expanded tree descendants included). Keypad * inverts files, or files and folders when Include folders is chosen. Also in F10 → Selection; all three are configurable; the number row's + − * stay filename-search text | Native flat views, archive listings and inline trees resolve masks on their selection workers. Engine, presenter, window and keypad-adapter tests and the `files_selection` acceptance scenario passed ([feature evidence](verification/2026-11-20-hidden-files-and-mask-selection.md)); the scenario sends the keypad keys through the native key hook's dispatch, which synthetic Slint events cannot reach. That record predates the Ctrl+= and Ctrl+− keys: their tests (also on Czech and Nordic layouts) have no recorded results yet. |
| Filename colors | Ordered masks/regex rules, twelve semantic category presets including a separate coral PDF rule, one base foreground and optional background; automatically derived focus/selection colors across all themes and a four-state preview | Matching runs on backend snapshots; invalid drafts do not replace active rules; explicitly removing all presets survives restart |
| Internal viewers and F4 | Independent basic text/binary, source highlighting, Markdown, image and PDF packages; text wrap/search/copy, default-on source line numbers and logical position, font controls and Markdown GitHub/Compact/Comfortable presets; image/PDF Fit/1:1/zoom/pan, captured-order Space/Backspace navigation; configurable external editor | Requires installed, enabled viewer packages in a trusted installation. The navigation order retains at most 8 MiB of path descriptors; in larger folders (roughly 25,000–65,000 files, depending on path length) Space/Backspace cycle within a contiguous window around the opened file, centered unless it reaches the start or end of the folder. A file's identity is captured when navigation opens it, and its immediate neighbors are prefetched and revalidated, so files changed after the viewer opened show their current content; a change while a file is displayed is reported. Single-file views, including Find results, reopen their original identity. Text selection/copy is bounded to the loaded page and Ctrl+A selects the visible rows; distant source positions index asynchronously. File content cannot reorder displayed lines: the basic viewer and the text comparator show the explicit bidirectional embedding, override and isolate controls (U+202A–U+202E, U+2066–U+2069) as `\u202E`-style escapes, and the source and Markdown viewers as U+FFFD; search and copy use the displayed text. Images decode at most 128 MiB of pixels and present at most 4 megapixels; animated raster formats show the first frame. A refused image or PDF names the decoder's own short reason after the host's sentence (image viewer API 0.3.0 failure reason; workers before SDK 0.4.0 keep the plain sentence). Source highlighting covers 25 lexical profiles, including CSS, Makefiles (`Makefile`/`GNUmakefile` by whole name, `.mk`, `.mak`), and CSV (`.csv`, RFC 4180 quoting) and TSV (`.tsv`, quotes are text), whose fields take the colour of their column; the CSS, Makefile and failure-reason checks are in the [viewer gap record](verification/2026-11-20-viewer-feature-gaps.md). SVG uses an inert bounded static subset with bundled fonts; AVIF supports bounded still-image decode. [Codec scope](developer/IMAGE_VIEWER_API.md) documents format-specific limits and exclusions. See [presentation and native controls](verification/2026-11-08-viewer-controls.md), [viewer limits](verification/2026-11-07-viewer-controls.md) and [image API](developer/IMAGE_VIEWER_API.md) |
| Copy | F5, clipboard and drag/drop share a bounded queue, full SHA-256 readback by default, explicit size plus three 64 KiB samples as a weaker option, folder merge and recoverable replacement; opt-in exact modified time for up to 64 native regular files | Provider capabilities govern replacement; strong identity-conditional overwrite is unavailable. On Linux links are copied as links (exact target text) or, with Follow links, followed where they can be ([links](developer/TRANSFERS.md#links-in-copy-and-move)); Windows keeps its directory-link options. Complete metadata preservation remains pending. Linux Copy destinations need `O_TMPFILE` (ext4/XFS/Btrfs/tmpfs); FAT/exFAT/NTFS media and NFS/SMB/sshfs/GVfs shares are refused. [Transfer contract](developer/TRANSFERS.md) |
| Move | Qualified same-volume native rename moves files/whole folders without content reads or hashes. Cross-volume Move copies and verifies each file before source removal; explicit directory merge processes children individually | Both OS backend gates pass; final integrated release qualification is pending. Linux identity/removal is live-validated, not atomic compare-and-unlink. On Linux a link moves as a link (renamed, or recreated and then removed across volumes). Virtual providers and full metadata remain unsupported. [Current contract](superpowers/specs/2026-11-10-transfer-verification-and-native-move.md), [evidence](verification/2026-11-10-transfer-verification-and-native-move.md) |
| Find files | Alt+F7 or the magnifier opens a modeless window with multiple roots (a dropdown with a checkbox per folder; Add types a path or browses with the native folder chooser), Masks/Regex/Expression with syntax help and clickable examples (Help or F1), clickable error/status details, resizable result columns, recursion, literal or per-line regular-expression UTF-8/BOM-marked UTF-16 content, whole words, modified-within minutes/hours/days, exact-content duplicate groups, five sortable headers, phased progress, Stop, validated Go to file and explicit guarded View/Basic View/Edit/Open | Native Windows/Linux folders with explicit link/mount/encoding/budget exclusions. New relative-age/duplicate/content behavior has 10 November backend regression coverage; duplicate groups stay adjacent and publish atomically. Guarded native result actions retain exact session/generation/identity, revalidate on backend execution and use the installed native provider for viewers. Search inside archives matches archive members through the installed archive plugins, with no Find limit on the number or size of archives, their members or the time spent in them: it searches until it ends or Stop, which also ends an archive being opened or a member being staged, while the search's general budgets and the plugin's own limits still apply (Go to file opens the archive folder at the member; members are never native targets); Feed to panel shows the native results as a list over the panel, where View, Edit, Copy, Move, Delete, clipboard and marks act on the real files and create/rename/paste are refused. [12 November action evidence](verification/2026-11-12-find-actions.md) records focused qualification. Installed native-desktop qualification remains separate; archive search has fake-provider coverage only. [Find contract](developer/FIND.md), [earlier mode/sort evidence](verification/2026-11-08-find-refactor.md) |
| Native mutations | Rename, empty-file creation, recursive F7, confirmed Trash/permanent deletion, attributes/timestamps/case changes. Native inline-tree Delete and focused F2 Rename use typed captures and the existing confirmation/overwrite/progress flow; native Properties and context menus retain exact branch occurrences. Recursive attribute, time and case edits capture each folder's names before changing its entries and pin each child only while processing it, so other pending metadata jobs no longer limit folder size; on one path from the selection, the captured names not yet processed may total at most 100,000 names and 4 MiB, and a folder that does not fit is reported and left unchanged with its contents ([evidence](verification/2026-11-18-recursive-metadata-traversal.md)). On Windows, a folder Rename or case change that is refused because a file below the folder is open (for example by an antivirus scan of new files) is waited out for a bounded time; same-volume Move stops with Retry/Skip/Cancel instead ([record](verification/2026-11-18-windows-directory-rename-refusals.md)). The Attributes / Times / Case editor names a single item (or counts items) in plain words, edits each timestamp in separate typed Date (year-month-day or the system's numeric date order), Time (24-hour or the system's clock, including AM/PM) and UTC offset fields with Up/Down part stepping and calendar/time pickers, starting from the item's current native-local time, and closes without a result notice when Apply completes cleanly ([ergonomics](developer/DIALOG_ERGONOMICS.md)) | Native inline-tree metadata uses captured targets; virtual-provider writes remain guarded. Recycling a Windows folder while an entry below it is open still fails at once, without a wait or a Retry decision; the error says that the item or something inside it is open in a program, which can be this application (a panel, an expanded folder or a viewer), and keeps the Shell code (TRASH-01 in the [backlog](IMPLEMENTATION_BACKLOG.md)). On Linux, a Rename, Trash move or permanent Delete that completed but could not remove its private staging folder stays completed: the job ends Completed with errors with one issue naming that folder. A Properties rename in that case still applies the item's other changes but counts the item as failed ([progress contract](developer/OPERATION_PROGRESS.md#lifetime-and-controls)). [Delete contract](developer/TREE_PANEL.md#delete-from-expanded-native-panels), [Rename contract](developer/TREE_PANEL.md#rename-from-expanded-native-panels); capability limits and per-item failures remain explicit |
| Directory comparison | Ctrl+F10 and the shared top toolbar compare displayed names, sizes and modification times; optional unmatched folders, case matching and time tolerance; optional byte-by-byte contents (size and time pre-check, progress, cancel) and recursive subfolders (folders with differences selected on the differing side, displayed subfolders of an expanded tree too) | Both panels must be native; adopted trees compare selectable root-level rows and retain exact occurrence-based selection. Virtual descendants are refused. Contents and subfolders use the native search service on a backend worker with node, depth, memory and candidate limits; links are matched by name and never followed. No attribute comparison. [Compare directories](developer/COMPARE_DIRECTORIES.md), [Tree contract](developer/TREE_PANEL.md#directory-comparison-with-expanded-branches) |
| Checksums | Ctrl+F8 opens a modeless window with captured-selection calculation, including selected native files/folders in adopted trees, and explicit checksum-file verification; SHA-256 default and optional SHA-512/MD5/SHA-1/CRC32, streamed progress, Stop, per-hash/full-row copy and verified export to the five corresponding formats, including SFV | Tree fallback requires a native regular file; archive/link targets are refused. Partial export requires explicit consent; output choices match the captured run. Manifest paths are bounded and validated without following links; matching checksums do not authenticate sources. Windows/Linux focused checks are recorded; packaged release qualification remains separate. [Checksum contract](developer/CHECKSUMS.md), [tree capture](developer/TREE_PANEL.md#checksums-from-expanded-panels), [file verification evidence](verification/2026-11-08-checksum-files.md) |
| Archive providers | Independent workers browse ZIP/7z/GZIP/TAR and TAR.GZ/TAR.XZ/TAR.BZ2/TAR.ZST plus raw compressed streams; F5 uses verified copying. Protocol 2 streams captured source ranges and verified members through private disk storage without artificial source/member/decoded-disk/ratio byte ceilings | Protocol 1 retains legacy limits. Protocol 2 retains two mounts, 1 GiB worker memory, 20-second progress watchdog, 256 KiB I/O and decoder/header bounds; available space on the private spool volume limits content, and staging stops before that volume falls below a max(1 GiB, 2% of capacity) free-space reserve. Nothing limits how many members an archive has or a folder lists, or the bytes of their names, in the worker, the plugin host or the engine: panels, F5 copy-out, folder sizes and Find take archive folders of any size, and F5 copies a selection of any size. The worker's memory bounds the index, about nine million TAR, two million 7z or one and a half million ZIP members (measured index memory and a sandboxed-worker check of 3,000,000 TAR and TAR.GZ, 2,000,000 7z and 1,200,000 ZIP members: [archive worker](../plugins/midnight-fs-archive/README.md)); a larger archive is reported as needing more memory than the worker has. A panel showing an archive folder holds about 650 bytes per member in the application (measured with a million members, [details](developer/FILESYSTEM_API.md)). Remaining limit: the worker SDK grants a worker at most 1 GiB, so a ZIP archive opens with up to about 1.5 million members. A source that cannot be read is reported as such, never as an invalid archive. Installed Windows/Linux worker checks passed 9 November; complete application/UI qualification remains separate. Archives inside archives open from a staged member for the session, up to 4 levels and 3 nested mounts ([details](ARCHIVE_FORMATS.md#archives-inside-archives)). No writing, passwords, RAR or external member launch; unavailable saved tabs remain explicit. [API and limits](developer/FILESYSTEM_API.md), [streaming verification](verification/2026-11-09-archive-streaming.md) |
| PDF | Independent isolated PDF plugin with page controls, zoom and pan; new PDFs open Fit window, including enlargement of small pages; Ctrl+PageUp/Down preserves manual zoom, while Fit adapts to each page size | Hayro rendering; unsupported/encrypted documents report an error. No scripts/network/link activation; bounded source/pages/rendering. See [opening/zoom verification](verification/2026-11-08-viewer-opening-toolbar.md) |
| File comparison | Ctrl+Shift+C or the shared top toolbar opens text differences, image comparison with Side-by-Side, Swipe and Onion Skin, or a side-by-side byte comparison (hex view with a summary and difference navigation) for binary files, text over the text limits and other pairs; the Hex button shows the bytes of a text pair; two active-panel selections take priority, otherwise an opposite selection or exact same-name counterpart is used. An unresolved pair opens File A / File B with native Browse and editable paths. | Native regular files only, including visible root files in an expanded native panel; automatic matching uses visible native names with exact case, never an unrelated opposite cursor; text up to 32 MiB and 1,000,000 lines of at most 16 KiB per file, bounded raster sizes, and a streamed byte comparison with bounded memory ([byte comparison](developer/FILE_COMPARISON.md#byte-comparison-hex-view)). [Automatic pairing](verification/2026-11-08-comparison-same-name.md), [manual choice contract](developer/FILE_COMPARISON.md) |
| Comparator settings / associations — planned | Ordered BOTH-basename overrides for Automatic, built-in text/images or a trusted external native program; [design](superpowers/specs/2026-11-13-comparator-associations-design.md), [UI wireframes](superpowers/specs/2026-11-13-comparator-associations-ui.md) and [plan](superpowers/plans/2026-11-13-comparator-associations.md). | Current-batch Options/config source is written but unqualified; backend/routing and end-to-end feature work remains in progress. The earlier private Core 43 result and UI setup failure retain their historical scopes, not feature availability. Existing file comparison remains available. |
| Custom Commands | Windows/Linux backend supports captured native arguments/cwd, per-item or grouped execution, previews and bounded process ownership; settings includes drag ordering, a conditional toolbar and configurable shortcuts; the F9 User menu lists and runs them, or points to their Options page when there are none | Windows Program/PowerShell/Cmd and Linux Program/POSIX profiles. Cmd supports single-line literal scripts with qualified direct native argument companions; builtins and reparsing have separate semantics. Native macOS support remains unavailable. [Cmd qualification](verification/2026-11-12-cmd-custom-commands.md). [Custom Commands design](superpowers/specs/2026-11-07-user-commands-design.md), [settings verification](verification/2026-11-07-custom-command-settings.md) |
| Native providers | Independent Windows/Linux native libraries provide panel Browse/Stat/Storage and native F3 content; required packages are included by build/release helpers, with no hidden fallback | Remaining services still use explicit OS adapters. Generic provider action migration, writes/watches and nested-container attachment remain pending; inline directory browsing is integrated. [Native API](developer/NATIVE_FILESYSTEM_API.md) |
| Network Neighborhood | Independent Windows WNet/MPR and Linux GIO/GVfs plugin; Network entry after drives in the dropdown and each panel drive bar, incremental discovery, cache, manual targets, explicit native-share handoff, parent/back context, persisted typed tabs and discovery Hot Paths with configurable numbered shortcuts | Requires OS discovery/runtime services. Discovery bookmarks retain opaque provider identity even when unavailable; [schema/shortcut evidence](verification/2026-11-11-network-hot-paths.md). Controlled live SMB/FUSE interoperability and interactive Linux credentials remain separate qualification/implementation work; native share Hot Paths also work. macOS is a later adapter. [Contract](developer/NETWORK_FILESYSTEM.md), [qualification](verification/2026-11-09-network-neighborhood.md) |
| Options | OK validates all pages, saves asynchronously and closes on success; Cancel/Escape/X discard the session and restore appearance previews; scrollable color-picker popup with fixed actions, native file choosers and editor argument rows. Appearance → Text rendering chooses Sharp (fitted), Balanced (snapped) or Smooth (subpixel) glyph fitting; it is saved and applies from the next start, and `MIDNIGHT_GLYPH_FITTING` (`fitted`, `snapped` or `subpixel`) overrides it for one run | Automatic session saves exclude unaccepted previews; [transaction design](superpowers/specs/2026-11-08-options-transaction.md). Release qualification is separate. |
| Archive Hot Paths | Typed provider/container/member/decoder addresses persist losslessly, display `archive.zip@member` and deduplicate; re-adding refreshes the source revision while preserving the shortcut slot. Source/settings/presenter checks passed 8 November; the embedded Hot Paths manager now captures its typed Add location at opening and preserves existing numbered-slot uniqueness | The 11 November [native manager checks](verification/2026-11-11-native-tool-windows.md) cover archive capture after navigation, deduplication, slot swaps, stale callbacks and visible creation-failure recovery without bookmark changes. Qualify the complete real-plugin menu/shortcut, restart/restore and unavailable-provider workflow before deployment. Display text, including literal `@`, never becomes a parsed locator. [Dated evidence and qualification gap](verification/2026-11-08-panel-viewer-followups.md) |
| Latest panel/viewer follow-ups | Larger same-height icons, middle path elision and longer pixel wheel coast have focused checks; Markdown/source presentation controls have actual Windows input checks. Filter icon/applied state, useful native drive information and omission of unmeasured folder totals exist in source | Current filter usability, thumbnail retention, remaining viewer latency and broader native/release qualification are tracked in [Active work](ACTIVE_WORK.md); existing controls are not wholly missing features |

The older [follow-up stages](superpowers/plans/2026-11-05-follow-up-stages.md)
retain historical requests; later accepted per-file Move, verification and dialog
refinements supersede conflicting stage text. Current assignments and remaining
acceptance are in [Active work](ACTIVE_WORK.md). Bounded admission and native
Copy scheduling are implemented in the current source; the specific authority
fix has targeted passes while broader concurrency/retirement acceptance remains
open. Copy/Move jobs are admitted at once, also beside a busy job, and run
concurrently on every storage class; the configured disk-I/O limit interleaves
their steps. Each job copies one file at a time and holds its disk slot only while
it executes native steps (not while paused or waiting for a decision). Where two
jobs use the same files or folders, only that item of the later job waits, with
its reason shown; reading what another job writes waits for that job. A namespace
change that a running peer refuses before any mutation is repeated automatically,
without a prompt, once that peer changes. Endpoint qualification and device budgets for streams within one job
remain required. Details are in the repository's security and architecture
documents.

The [10 November same-SSD benchmark](verification/2026-11-10-ssd-copy-probe.md)
measured substantial gains in the native primitive for large/mixed files, but
repeated abnormal audit delays prevented full qualification. Its decision is
no-go for implementation from this run; it does not mark parallel copying as
available or establish that parallel copying has no benefit.

The release archives include this status file; relative design links are intended
for the repository version at https://github.com/TomasJelinek/fm.

## Completion evidence index (54 retained requirements)

This read-only index preserves all 54 original IDs. **Implemented** means the
named source baseline exists; native/platform/release acceptance remains separate.
**Partial**, **missing**, **bounded** and **extension** distinguish implementation
boundaries. Complete per-ID findings, current source hashes, exact raw paths and
result limits are in `_build/verification/viewer-refresh-20261117/docs/evidence.json`
under `completion_index` and `completion_evidence_catalog`. A test's existence is
not a pass. Unretrieved historical raw evidence is an evidence-retention gap, not
proof of missing implementation.

| ID | Current implementation/source | Verified evidence boundary | Remaining work |
| --- | --- | --- | --- |
| ARCH-01 | Implemented; [session.rs](../crates/midnight-filesystem-host/src/session.rs) | [Archive 50/0/0 selected](../_build/verification/viewer-refresh-20261117/backend/qualification-packet.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| ARCH-02 | Partial; [nested.rs](../crates/midnight-filesystem-host/src/nested.rs) | Source/contract boundary; host, engine and `viewers_archives` coverage without a dated result record | Flat panels open archives inside archives for the session (4 levels, 3 nested mounts); installed-worker qualification, outer-source changes, solid readers, inline branches and persistence remain. |
| ARCH-03 | Extension; [lib.rs](../plugins/midnight-fs-archive/src/lib.rs) | Source/contract boundary; no complete capability gate claimed | Later write/auth/export contracts. |
| ARCH-04 | Implemented; [archive_extensions.rs](../crates/midnight-core/src/archive_extensions.rs) | [Historical report](verification/2026-11-08-archive-custom-extensions.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| CMD-01 | Implemented; [user_commands.rs](../crates/midnight-core/src/user_commands.rs) | [Historical report](verification/2026-11-12-command-executable-icons.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| CMD-02 | Implemented; [command_line.rs](../crates/midnight-ui/src/command_line.rs) | [Nov 16 secondary reuse accounting](../_build/verification/final-review-tests-20261116/root-runtime-remainder-v3/runtime-stage-results.json); secondary accounting; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| CMD-03 | Implemented; [user_commands.rs](../crates/midnight-ui/src/presenter/user_commands.rs) | [Historical report](verification/2026-11-09-custom-command-preview.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| COPY-01 | Implemented; [transfer.rs](../crates/midnight-platform-common/src/transfer.rs) | [Historical report](verification/2026-11-07-recoverable-copy.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| COPY-02 | Implemented; [copy_worker.rs](../crates/midnight-engine/src/copy_worker.rs) | [Historical report](verification/2026-11-07-recoverable-copy.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| COPY-03 | Partial; [transfer_metadata.rs](../crates/midnight-core/src/transfer_metadata.rs) | [Historical report](ACTIVE_WORK.md); raw not retrieved | Windows link Preserve/Move (Linux implemented, [links](developer/TRANSFERS.md#links-in-copy-and-move)) and broader metadata. |
| DIALOG-02 | Implemented; [dialog-frame.slint](../crates/midnight-ui/ui/dialog-frame.slint) | [Historical report](verification/2026-11-11-native-tool-windows.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| DRIVE-02 | Implemented; [volume_information.rs](../crates/midnight-core/src/volume_information.rs) | [Historical report](verification/2026-11-08-drive-information.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| FILTER-01 | Implemented; [parser.rs](../crates/midnight-platform-common/src/filter_expression/parser.rs) | [Historical report](verification/2026-11-07-metadata-expression-filters.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| FILTER-02 | Implemented; [panel-view-controls.slint](../crates/midnight-ui/ui/panel-view-controls.slint) | [Historical report](verification/2026-11-11-native-tool-windows.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| FIND-01 | Implemented; [find.rs](../crates/midnight-engine/src/find.rs) | [Historical report](verification/2026-11-12-find-actions.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| FIND-02 | Implemented; [find_predicate.rs](../crates/midnight-engine/src/find_predicate.rs) | [Historical report](verification/2026-11-08-find-refactor.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| HASH-01 | Implemented; [checksum.rs](../crates/midnight-core/src/checksum.rs) | [Historical report](verification/2026-11-08-checksum-files.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| HASH-02 | Extension; [checksum.rs](../crates/midnight-core/src/checksum.rs) | Source/contract boundary; no complete capability gate claimed | Later algorithm/plugin/history contracts. |
| HOTPATH-02 | Implemented; [bookmarks.rs](../crates/midnight-ui/src/presenter/bookmarks.rs) | [Historical report](verification/2026-11-08-panel-viewer-followups.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| ICON-02 | Implemented; [archive-badge.slint](../crates/midnight-ui/ui/archive-badge.slint) | [Nov 16 secondary reuse accounting](../_build/verification/final-review-tests-20261116/root-runtime-remainder-v3/runtime-stage-results.json); secondary accounting; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| IO-01 | Partial; [copy_worker.rs](../crates/midnight-engine/src/copy_worker.rs) | [Historical report](verification/2026-11-10-ssd-copy-probe.md); raw not retrieved | SSD/shared-device scheduler; benchmark NO-GO. |
| LIFE-01 | Partial; [engine_lifetime.rs](../crates/midnight-engine/src/engine_lifetime.rs) | [Engine 5+22 repair; tests/lint/format 0](../_build/verification/viewer-refresh-20261117/build/engine-repair-gate/completion.json)  [Retirement capacity by source checkpoint](developer/ENGINE_LIFETIME.md#capacity-and-ownership); [Resident private contract](../_build/verification/viewer-refresh-20261117/architect/production-design-and-appearance-handoff.json); [Private round 2 off 13/on 24](../_build/verification/viewer-refresh-20261117/build/resident-common-private-gate/completion.json); [Genuine old-source RED](../_build/verification/viewer-refresh-20261117/root-production-pool-red-review.json);  [Private PO v4 128/0/0](../_build/verification/viewer-refresh-20261117/root-viewer-po-admission-v4-runtime128-review.json); [Genuine native MAX 1/0/0](../_build/verification/viewer-refresh-20261117/root-native-maximum-corrected-runtime-review-v1.json);  [Private collector 11/0/0](../_build/verification/viewer-refresh-20261117/root-viewer-ui-po-collector-v2-runtime11-review.json); | Private 128 Viewer/11 collector passes and genuine native MAX pass qualify their named boundaries only. Host 11 not replayed; final Generated cleanup, canonical actor/UI/App integration and full lifecycle/ownership/Rename acceptance remain open. |
| MEDIA-01 | Implemented; [archive_source.rs](../crates/midnight-thumbnail-host/src/archive_source.rs) | [Historical report](verification/2026-11-10-archive-thumbnails.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| MEDIA-02 | Bounded; [native.rs](../plugins/midnight-thumbnail-video/src/native.rs) | Source/contract boundary; no complete capability gate claimed | Intentional codec limits; review accepted extensions. |
| MEDIA-03 | Implemented; [cache.rs](../crates/midnight-thumbnail-host/src/cache.rs) | [Historical report](verification/2026-11-10-archive-thumbnails.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| META-02 | Implemented; [metadata-controls.slint](../crates/midnight-ui/ui/metadata-controls.slint) | [Historical report](verification/2026-11-11-dialog-fonts-metadata-and-drag-feedback.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| MOVE-01 | Partial; [move_native.rs](../crates/midnight-engine/src/move_native.rs) | [Historical report](verification/2026-11-10-transfer-verification-and-native-move.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| NAV-01 | Implemented; [clipboard_text.rs](../crates/midnight-engine/src/clipboard_text.rs) | [Historical report](developer/PATH_PASTE.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| NAV-02 | Implemented; [presenter.rs](../crates/midnight-ui/src/presenter.rs) | [Historical report](verification/2026-11-08-panel-viewer-followups.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| NET-01 | Partial; [catalog.rs](../plugins/midnight-filesystem-network/src/catalog.rs) | [Historical report](verification/2026-11-13-linux-network-discovery.md); raw not retrieved | Linux interactive auth and live share acceptance. |
| NOTICE-02 | Implemented; [error-notice.slint](../crates/midnight-ui/ui/error-notice.slint) | [Nov 16 secondary notice accounting](../_build/verification/final-review-tests-20261116/root-ui-remainder-f450/runtime-stage-results.json); secondary accounting; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| OPTIONS-02 | Implemented; [options.rs](../crates/midnight-ui/src/presenter/preferences/options.rs) | [Engine 5+22 repair; tests/lint/format 0](../_build/verification/viewer-refresh-20261117/build/engine-repair-gate/completion.json); [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json)  [UI 104/159 source-derived](../_build/verification/viewer-refresh-20261117/ui/current-phase.json); [MainUI/Compare 55](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/main-compare-units-prep/review-evidence/execution-receipt.json); [Draft 1 pass/focus 1 fail](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/failure-diagnostic-1/execution-receipt.json); [Opening palette contract/private](../_build/verification/viewer-refresh-20261117/architect/opening-palette-revision-contract-decision.md); | Retry 4 immutable 20 and separate named caret 1 pass, including two Main focus/layout cases. Separate Options Transaction 2 unrun; full graph/native/platform acceptance open. |
| PANEL-02 | Implemented; [theme.slint](../crates/midnight-ui/ui/theme.slint) | [Historical report](verification/2026-11-07-options-ergonomics.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| PERF-01 | Extension; [transfer.rs](../crates/midnight-platform-common/src/transfer.rs) | Source/contract boundary; no complete capability gate claimed | Evaluation contracts/measured benefit first. |
| PFS-01 | Partial; [operations.rs](../crates/midnight-engine/src/operations.rs) | [Historical report](superpowers/plans/2026-11-09-provider-performance-usability.md); raw not retrieved | Provider action audit/mutation migration. |
| PFS-02 | Implemented; [archive_sizes.rs](../crates/midnight-engine/src/archive_sizes.rs) | [Archive 50/0/0 selected](../_build/verification/viewer-refresh-20261117/backend/qualification-packet.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| PLAT-01 | Missing; [lib.rs](../crates/midnight-platform/src/lib.rs) | Source/contract boundary; no complete capability gate claimed | macOS application services/packages/CI. |
| PLAT-02 | Partial; [lib.rs](../crates/midnight-platform/src/lib.rs) | Source/contract boundary; no complete capability gate claimed | Distribution packages and physical/native matrix. |
| PLAT-03 | Missing; [file_drag_drop.rs](../crates/midnight-platform-linux/src/file_drag_drop.rs) | Source/contract boundary; no complete capability gate claimed | Native Wayland external transport. |
| REG-01 | Extension; [mod.rs](../crates/midnight-engine/src/providers/mod.rs) | Source/contract boundary; no complete capability gate claimed | Later typed Registry provider. |
| REMOTE-01 | Extension; [mod.rs](../crates/midnight-engine/src/providers/mod.rs) | Source/contract boundary; no complete capability gate claimed | Later remote transport/auth provider contracts. |
| SCROLL-02 | Partial; [worker.rs](../crates/midnight-viewer-engine/src/worker.rs) | [Source 64/0; lint/format 0](../_build/verification/viewer-refresh-20261117/viewer/recovery/review-fixes-v1/receipt.json)  [Earlier Source native compiled/unrun](../_build/verification/viewer-refresh-20261117/build/office-repair-gate/headless-completion.json); [DLL cache reuse only](../_build/verification/viewer-refresh-20261117/build/native-worker/source-candidate-preflight/normal-producer-1/qualification.json); [Current native Source 0/1/0 exit 101](../_build/verification/viewer-refresh-20261117/build/native-current-source-e0df-prep/source-cpp-native-1/result.json); | Current native frame-deadline failure unresolved; Causal native Source drag/cold-target/pacing evidence. |
| SELECT-01 | Implemented; [selection_rectangle.rs](../crates/midnight-ui/src/selection_rectangle.rs) | [Nov 16 secondary reuse accounting](../_build/verification/final-review-tests-20261116/root-runtime-remainder-v3/runtime-stage-results.json); secondary accounting; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| STATUS-02 | Implemented; [selection_sizes.rs](../crates/midnight-ui/src/presenter/selection_sizes.rs) | [Historical report](verification/2026-11-08-panel-viewer-followups.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| TERM-01 | Missing; [desktop.rs](../crates/midnight-core/src/desktop.rs) | Source/contract boundary; no complete capability gate claimed | Reusable terminal/sudo broker/session API. |
| THEME-02 | Implemented; [theme-palettes.slint](../crates/midnight-ui/ui/theme-palettes.slint) | [Five checks: 2 theme+3 comparison](../_build/verification/viewer-refresh-20261117/ui/green-results.json); [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json)  [MainUI/Compare 55](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/main-compare-units-prep/review-evidence/execution-receipt.json); [Appearance private contract](../_build/verification/viewer-refresh-20261117/architect/viewer-compare-appearance-contract-decision.md); | Retry 4 selected UI 20 pass; new full 320x200 footer captures Root-verified. Old partial images retain limits; all-layout/platform/performance qualification open. |
| TREE-01 | Partial; [tree.rs](../crates/midnight-engine/src/providers/tree.rs) | [Historical report](verification/2026-11-12-recursive-tree.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Container discovery/archive watches/leases/nesting in inline branches; parity. |
| UI-01 | Implemented; [panel-view-controls.slint](../crates/midnight-ui/ui/panel-view-controls.slint) | [Historical report](verification/2026-11-09-panel-controls-and-paths.md); raw not retrieved; [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json) | Current installed/native input, platform and release acceptance; no absent baseline established. |
| VFS-01 | Partial; [abi.rs](../filesystem-api/midnight-filesystem-plugin-api/src/abi.rs) | [Historical report](verification/2026-11-13-checkpoint-evidence-history.md); raw not retrieved | Current installed/native input, platform and release acceptance; no absent baseline established. |
| VIEW-01 | Partial; [routing.rs](../crates/midnight-viewer-engine/src/routing.rs) | [Office 68+18+27=113/0/0](../_build/verification/viewer-refresh-20261117/build/office-six-format-gate/headless-counts.json); [native 1 pass/1 fail/1 unrun](../_build/verification/viewer-refresh-20261117/build/native-worker/office-00d13f4a3c/results.json); [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json)  [UI 12/1/2 unrun](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/independent-integration-1/result.json); [Repair 86/0](../_build/verification/viewer-refresh-20261117/build/office-repair-gate/headless-completion.json); [Fresh 60/lint 0](../_build/verification/viewer-refresh-20261117/build/office-repair-gate/favicon-lint-1/quality.json); [UI 104/159 source-derived](../_build/verification/viewer-refresh-20261117/ui/current-phase.json); [MainUI/Compare 55](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/main-compare-units-prep/review-evidence/execution-receipt.json); [Host repair/source binding](../_build/verification/viewer-refresh-20261117/backend/office-web-host/current-source-freeze.json); [91 frozen passes](../_build/verification/viewer-refresh-20261117/build/office-host-revocation-gate/quality.json); [Office parent 0/1](../_build/verification/viewer-refresh-20261117/build/native-worker/office-9fe36f20a6/handoff.json); [Historical Viewer 45/1](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/viewer-fixture-repair-1/execution-receipt.json); [Affected Viewer 46/0](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/catalog-helper-viewer-1/execution-receipt.json);  [Current native HTML failure](../_build/verification/viewer-refresh-20261117/build/native-current-e0df-execution/office-failure-parent-close.json); [Current XLSX failure](../_build/verification/viewer-refresh-20261117/build/native-current-e0df-execution/office-remaining4-failure-parent-close.json);  [All 5 Office executed: 2 PASS/3 FAIL](../_build/verification/viewer-refresh-20261117/root-office-final-two-native-pass-review-v1.json);  [OfficeDIAG test-source promotion](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/office-native-phase-observation-v1/canonical-source-ready.json); | Native 710bea oracle retained. All 5 cases actually executed across explicit harness epochs: 2 PASS/3 FAIL/0 UNRUN. HTML timeout, XLSX Find and later-navigation failures remain; refusal-before-data and close/cancel cases passed. OfficeDIAG fixture 50eb canonical/full source 2ae892, separate diagnostic producer compile 0/new 3666; two metadata observations failed initial wait-loaded/zero events 20.29s/20.38s, distinct from original three navigation failures/cause unknown; old 710/0bb preserved; no overall native Office green. |
| VIEW-02 | Implemented; [syntax_lua.rs](../plugins/midnight-viewer-source/src/syntax_lua.rs) | [Source 64/0; lint/format 0](../_build/verification/viewer-refresh-20261117/viewer/recovery/review-fixes-v1/receipt.json); [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json)  [UI 12/1/2 unrun](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/independent-integration-1/result.json); [Earlier Source native compiled/unrun](../_build/verification/viewer-refresh-20261117/build/office-repair-gate/headless-completion.json); [UI 104/159 source-derived](../_build/verification/viewer-refresh-20261117/ui/current-phase.json); [DLL cache reuse only](../_build/verification/viewer-refresh-20261117/build/native-worker/source-candidate-preflight/normal-producer-1/qualification.json); [Current native Source 0/1/0 exit 101](../_build/verification/viewer-refresh-20261117/build/native-current-source-e0df-prep/source-cpp-native-1/result.json);  [DIAG 1 native failure 0/1/0](../_build/verification/viewer-refresh-20261117/root-source-native-diag1-failure-evidence-review.json);  [Private Source 3 RED->18 GREEN](../_build/verification/viewer-refresh-20261117/root-source-mapped-prefix-runtime-red3-green18-review-v1.json);  [Source production promotion](../_build/verification/viewer-refresh-20261117/root-coherent-source-integration/source-mapped-prefix-v2/canonical-source-ready.json); [Retained private 6 proof](../_build/verification/viewer-refresh-20261117/root-source-mapped-prefix-retained-runtime6-review-v1.json); | Mapped-prefix production 25af/tests d394 now canonical/source c500. Private 3 RED->18 GREEN and five prior controls plus one new retained exact PASS retain their epochs; no repeated 23. Work-branch canonical 24 and the d872/C40dd validation inputs were verified on c500. The private installed-provider fix passed its selected case. The wider corpus failed at C++ ACK 22. Merged integration and native Source acceptance remain open. Original failure context metadata does not prove Worker-open generation/reopen/specific cause. |
| VIEW-03 | Implemented; [viewer_preferences.rs](../crates/midnight-core/src/viewer_preferences.rs) | [Engine 5+22 repair; tests/lint/format 0](../_build/verification/viewer-refresh-20261117/build/engine-repair-gate/completion.json); [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json)  [UI 12/1/2 unrun](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/independent-integration-1/result.json); [UI 104/159 source-derived](../_build/verification/viewer-refresh-20261117/ui/current-phase.json); [Historical Viewer timeout](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/failure-diagnostic-1/execution-receipt.json); [Shared helper source](../_build/verification/viewer-refresh-20261117/ui/viewer-shared-fixture-fix/canonical-promotion.json); [Affected Viewer 46/0](../_build/verification/viewer-refresh-20261117/build/ui-runtime-worker/catalog-helper-viewer-1/execution-receipt.json); | Current preference baseline; appearance bridge, Preview/Reset/Cancel/window propagation remain unimplemented/unqualified. |
| VIEW-PERF-01 | Partial; [glyph_cache.rs](../crates/midnight-viewer-render/src/glyph_cache.rs) | [Source 64/0; lint/format 0](../_build/verification/viewer-refresh-20261117/viewer/recovery/review-fixes-v1/receipt.json); [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json)  [Earlier Source native compiled/unrun](../_build/verification/viewer-refresh-20261117/build/office-repair-gate/headless-completion.json); [DLL cache reuse only](../_build/verification/viewer-refresh-20261117/build/native-worker/source-candidate-preflight/normal-producer-1/qualification.json); [Private common round 2 only](../_build/verification/viewer-refresh-20261117/build/resident-common-private-gate/completion.json); [Current native Source 0/1/0 exit 101](../_build/verification/viewer-refresh-20261117/build/native-current-source-e0df-prep/source-cpp-native-1/result.json); | Current native frame-deadline failure unresolved; Native long-line/page/DPI/Linux latency/RSS. |
| VIEW-PERF-02 | Partial; [glyph_cache.rs](../crates/midnight-viewer-render/src/glyph_cache.rs) | [Source 64/0; lint/format 0](../_build/verification/viewer-refresh-20261117/viewer/recovery/review-fixes-v1/receipt.json); [Earlier UI compile 101](../_build/verification/viewer-refresh-20261117/build/ui-explorer-batch/integration/owner-fix-required.json)  [Earlier Source native compiled/unrun](../_build/verification/viewer-refresh-20261117/build/office-repair-gate/headless-completion.json); [DLL cache reuse only](../_build/verification/viewer-refresh-20261117/build/native-worker/source-candidate-preflight/normal-producer-1/qualification.json); [Current native Source 0/1/0 exit 101](../_build/verification/viewer-refresh-20261117/build/native-current-source-e0df-prep/source-cpp-native-1/result.json); | Current native frame-deadline failure unresolved; Matching packaged/native drag/paint acceptance. |

### Implementation still missing after this batch

The resident-worker design, its production-pool runs and the catalog-only host
constructor are recorded under [Previous scoped checkpoints](#resident-worker-design-and-production-pool).

The archived round-two common-worker QA is recorded under
[Previous scoped checkpoints](#round-two-common-worker-qa).

The Viewer/Compare live-appearance contract is recorded under
[Previous scoped checkpoints](#viewer-compare-live-appearance-contract).

The small-body navigation contract and the canonical Office V3 sources are recorded
under [Previous scoped checkpoints](#small-body-navigation-and-office-v3).

- Archives inside archives in inline branches, container discovery and archive
  watches remain under ARCH-02/TREE-01 (flat panels open archives inside archives
  since 20 November). Prepared-Rename engine/UI finishing and provider
  mutation/metadata/watch migration remain under VFS-01/PFS-01/LIFE-01.
- Windows no-follow link Preserve/Move and broader metadata remain under
  COPY-03/MOVE-01 (Linux transfers links as links). The IO-01 SSD/shared-device scheduler is absent; its benchmark
  prerequisite remains NO-GO. Linux network authentication (NET-01), reusable
  terminal/sudo sessions (TERM-01), macOS app services (PLAT-01), native Wayland
  external drag (PLAT-03) and remaining distribution packaging (PLAT-02) are
  implementation work, not merely unrun tests.
- ARCH-03, REMOTE-01, REG-01, HASH-02, MEDIA-02 and PERF-01 retain separate later
  extension/evaluation contracts. No automatic paid/proprietary codec, RAR, GPU
  or whole-file feature obligation is inferred. Current supported archive reads,
  five hashes, codecs and CPU SHA-256 remain valid baselines.

### Validation and diagnosis still open

The P3 UI packet and shared-helper Viewer results are recorded under
[Previous scoped checkpoints](#p3-ui-packet-results). The Options focus failure
recorded there was resolved later: the immutable20 set, including both Options
focus/layout cases, passed 20/0/0
([current checkpoint](#current-integration-checkpoint--18-november-2026)); its
native/full-app and production migration acceptance remain open.

The faithful native comparator fixture run is recorded under
[Previous scoped checkpoints](#faithful-native-comparator-fixture).

The Office parent diagnostic `office-9fe36f20a6` is recorded under
[Previous scoped checkpoints](#office-parent-diagnostic-9fe36f20a6).

The small-body navigation contract, its navigation-core checks and the five pending
native cases are recorded under [Previous scoped checkpoints](#small-body-navigation-and-office-v3).

Source DLL cached reuse remains qualified; native Source acceptance is unrun.
The full 54-item implementation/acceptance goal remains open.
