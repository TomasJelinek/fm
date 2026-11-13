# Recorded checkpoint evidence — 13 November 2026

This note retains exact older run identities, failures, overlapping counts and
limits moved from the [current work ledger](../ACTIVE_WORK.md#current-checkpoint).
It was extracted from ledger SHA256
`68124f3a1739cb271cd32af4fb812f67c1289ce10b30eb908989897814e9e280`. Statements below describe their named source/environment
snapshots; they do not qualify current `174c5fd`, a fresh debug build, deployment
or native performance. Current decisions and open acceptance remain in the ledger.

Existing tracked reports retain the original
[recursive/tree and thumbnail qualification](2026-11-12-recursive-tree.md),
[panel qualification](2026-11-12-panel-interaction-polish.md) and
[CSC warning-retention checks](2026-11-11-directory-authorization.md#12-november-csc-warning-retention-regression).
Their dated Linux-unexecuted, fixture/mock, ignored-test and platform limits stay
in force; later WSL receipts below have separate scopes.

## Earlier source integration

Native-source merge `5baee471` also included packaging-parser checkpoint
`8e7320dc`; these identities precede source checkpoint `174c5fd`.

Earlier `fe8a4177` includes the `338eb0a` presence/Find source/test/docs checkpoint,
CI engine-warning corrections and the Clippy-step reorder, retaining its own
recorded gates.

The user authorized selecting useful historical branch changes, discarding obsolete
changes and deleting branches. The Orchestrator accepted the archive/thumbnail
source-equivalence reports and deleted their exact local/remote tips and clean
worktrees.
The `origin/fix/cargo` tip `8e084b3339ea3bf6cbc72567d7691af1dfb4fa7d` is now
incorporated by upstream `a9f31d7` and completed local merge `28cc79a`, preserving
Docker fallback/native Rust bootstrap alongside the reviewed guards.
The reconciled contract keeps `FM_BUILD_BACKEND=auto|native|docker` and
`./build.sh [debug|release|clean] [--install-deps] [--dry-run]`; explicit package
installation selects native setup and conflicts with explicit Docker. Docker uses
the full app/elevation/filesystem/viewer chain and `.tools/docker/` caches.
The Orchestrator verified the exact Cargo tip is an ancestor of upstream and
deleted remote `fix/cargo` under that exact-tip lease. No local `fix/cargo` branch
existed; the reported remote branch list now has no unmerged branches relative
to `origin/main`. The already merged
`main-deep-audit` worktree is retained because the Orchestrator's host cwd uses it.
No role independently switches/merges/deletes branches. No new build or native
desktop input starts without the coordinated lease. Independent headless tests use
bounded parallelism; desktop/input and performance probes run serially.

Merge ownership was exclusive: Documentation resolved `README.md`; Build resolved
`build.sh`. The Orchestrator completed the merge. Other frozen documentation
changes are preserved in checkpoint `5ab0e86`; this ledger retains the cleanup
status.
Staging, commit and push remain Orchestrator-owned.


Earlier committed/pushed D `main` base: **`1546030`**, 12 files and verified clean by
the Orchestrator at that checkpoint. It integrates the qualified link-maintenance source,
clipboard helper correction and documentation.
It includes recursive expansion `ce9eff3`, thumbnail correction `8e205c0` and
portable panel scroll regression tests `3bee1dc`, alongside the earlier API/SDK, CMD-02 and viewer work.

The initial assignment names integrated `main` commit `369b161`. The Orchestrator
accepted the Markdown slice as `cc4857e` and checkpointed the build port/README as
`b5b3b7b`. Upstream `origin/main` then advanced to
`a9f31d7472bc6ad12cf9856fd513061fef940b50`, incorporating `8e084b3` Docker/native
Rust bootstrap work. Merge `28cc79add548eed0d6c9e465a6fe26a5d141f9b7` is complete
and includes that upstream head. It reconciles Docker/native bootstrap with
reviewed cleanup/dry-run/native-setup guards and the full Docker build chain,
whose caches belong under `.tools/docker/`. The merge, including README, is pushed.
The seven-document checkpoint is committed as
`5ab0e86f40c0164f4700da96e1ff09a3b5f65de5`.
The checkout is shared and
later role edits need their own source/build identity.
This documentation reconciliation inspects source and existing reports; it does
not execute application tests or qualify another role's changes.


## Retained private runtimes

### Previous Windows launch-qualified runtime `c6c371bc`

The fresh debug receipt (`_build/verification/image-prewarm-debug-20261113/debug-build-result.json`)
records exit 0, zero drift and **101,975,552 bytes** for `_build/debug/me.exe`,
SHA256 `c6c371bc313b15913ea2584bf5adc88a3ad165702d9bd64a5b95e40d012511ac`.
The registered private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261113-024953-c6c371bc/me.exe`.

The qualified startup receipt (`_build/verification/image-prewarm-debug-20261113/qualified-debug-result.json`)
records source `fd94d8568f83cf10fe61729bec4f66556aa5c353`, all 17 frozen sources
matching committed blobs, zero drift and both established smokes exit 0:
engine **17.445 s**, native UI **12.867 s**. The visual-review receipt records
normal fixture panels/toolbars/status/footer. All 1,382 runtime file hashes were
verified; fixture/preference/settings hash-and-mtime checks are unchanged. The
previous `26ff61f2` runtime and desktop shortcut are preserved. Native Rename candidates were not
deployed and no tree-performance candidate is included.

Owned job cleanup completed. The extra blanket pre-cleanup PID assertion failed
on PID 29940; its identity/path/parent lifetime remains unclassified. Natural
shutdown of all auxiliary processes remains unqualified. The later
[image-spare shutdown correction](../ACTIVE_WORK.md#image-spare-shutdown-gate) is committed and
passes its scoped gates, but is absent from this runtime. PID 29940 remains
unattributed. No smoke retry or new-runtime performance harness run occurred. Startup
does not qualify large-tree speed, complete viewer workflows or physical Linux UI.

### Previous Windows launch-qualified runtime `26ff61f2`

`_build/verification/backend-folder-presence-links-20261113/debug-build-result.json`
records exit 0, zero source drift and **101,794,816 bytes** for
`_build/debug/me.exe`, SHA-256
`26ff61f2de03e13b86e82e66bf3ddad1087fc21d47bdd445e9dedb4d10d9ad59`.
The registered private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261113-004830-26ff61f2/me.exe`.

The qualified startup receipt (`_build/verification/backend-folder-presence-links-20261113/qualified-debug-result.json`)
and `private-entrypoint-smoke-results.json` in that directory record engine
`--smoke-test` exit 0 (**17.779 s**) and native `--ui-smoke-test` exit 0
(**6.883 s**), qualified at **00:51:43 UTC on 13 November**. Source is baseline
`1546030` plus the frozen presence/Find files, with zero drift; production Find
is unchanged. All 1,382 runtime file hashes were verified, fixtures/preferences
are unchanged, the previous `269f3419` runtime is retained and the desktop
shortcut is unchanged. Root independently inspected the normal two-panel startup
snapshot. Native Rename candidates were not deployed. This qualifies Windows startup, not protected CSC/UAC access, full viewer workflows, Linux
desktop UI or native performance.

### Previous Windows launch-qualified runtime `269f3419`

`_build/verification/backend-tree-link-refresh-20261113/debug-build-result.json`
records exit 0, zero source drift and **101,794,816 bytes** for
`_build/debug/me.exe`, SHA-256
`269f34195139865e4fb68430fdc396eeb384469dc9cc28a7222612b6f89658eb`.
The registered private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261113-269f3419/me.exe`.

`private-entrypoint-smoke-results.json` and `private-qualified-windows-result.json`
in that directory record both engine `--smoke-test` and native `--ui-smoke-test`
exit 0, zero source drift, and baseline `1714d99` plus frozen source overlays.
The previous `d2643b49` runtime, preferences and desktop shortcut are preserved;
candidate native Rename plugins were not deployed. This qualifies Windows
startup, not full viewer workflows, manual CSC/UAC access or Linux desktop UI.


### Previous launch-qualified runtime `d2643b49`

The historical **101,768,704-byte** EXE is
`C:/Users/tjelinek/MidnightExplorer-20261112-224859-d2643b49/me.exe`, SHA256
`d2643b49917c29b8f5f3b0171dc075bb30058817de6f0b6b1212d8361c0f658d`.
Its original startup scope is retained in the
[recursive-tree report](2026-11-12-recursive-tree.md). Additional ledger provenance:
`_build/verification/build-20261112/final-debug-build-result.json`; qualified source
`3619070f6e54eb442ddfa13fcd7b17d9282225f4`, at **23:51:48 CET on 12 November**.
This is an older runtime,
not the current qualified `516fc158` or an application performance gate.

## Presence, Find and scaling

### Native link presence and Find gate

The native flat-panel admission correction accepts `DirectoryLink` only for an
exact captured native root; archive batches still reject links and files. One
native link previously discarded the entire viewport request. Empty or denied
links now retain individual hints alongside ordinary siblings, with no recursive
probe or provider/UI/API change. Frozen `folder_presence.rs` SHA256 is
`af9cdba4549caace4bfedf02eaaf34577fa273dee9a13320b52553bfe90b6c95`.
The actual RED receipt is
`_build/verification/backend-folder-presence-links-20261113/build-red-20261113T003717Z/result.json`:
compile 0, **6 passed / 2 failed / 0 ignored**, zero drift.

The final Build receipt (`_build/verification/backend-folder-presence-links-20261113/build-green-find-20261113T004248Z/result.json`)
records **18 selected / 18 passed / 0 failed / 0 ignored**, both compiles 0 and
zero source drift. Presence is **12/0/0**: admission 8 + native provider 4.
Durable Find regressions are **6/0/0**: Engine 5 + Windows native 1. All owned
processes exited and the source lease was released. The fresh `26ff61f2` debug
build and registered private engine/native UI startup pass with zero drift, as
recorded in the [26ff61f2 runtime receipts](#previous-windows-launch-qualified-runtime-26ff61f2); the previous
`269f3419` runtime retains its earlier source scope.

The earlier Find diagnostic receipt (`_build/verification/backend-find-result-revision-20261113/build-owned-20261113T002043Z/runtime-result.json`)
records **13/0/0**, including read-only probes of the exact reported
`D:/dev/ov-physics-trunk/ovclash/source/DynamicMatrixCacheLimit.h`. Its hash, size
and mtime were unchanged; raw path, kind, identity and all six revision words
matched, and repeated validation was accepted. The original Open report remains
unreproduced: its old build and captured metadata are unknown. No production Find
fix or proven false-stale cause follows. The final six durable regressions
overlap that diagnostic scope, with no user-fixture rerun in the final gate;
they are not six additional unique diagnostic cases.

<a id="tree-scaling-candidate-gate"></a>

### Tree scaling candidate gate — aggregate failed

The corrected candidate build (`_build/verification/tree-performance-20261113/candidate-build-result.json`)
exits 0 with `fresh=false` (Cargo rebuilt the artifact), zero drift and controller SHA256
`16b725e3e997da4459e6ee581761904ae2b9ee251178cb7c587b8035ba83cb1e`.
The functional receipt (`_build/verification/tree-performance-20261113/candidate-functional-result.json`)
passes **95 unique cases / 0 failed / 0 ignored**: library 91, including 10 new
invariants, plus controller 4. Candidate Clippy (`_build/verification/tree-performance-20261113/candidate-clippy-result.json`)
exits **0**, zero drift. These functional scopes remain valid separately from
the failed timing aggregate and are not additional passes for older gates.

The admitted candidate aggregate (`_build/verification/tree-performance-20261113/candidate-scaling/result.json`)
reports **FAILED**, exit **101**. Timings are from a debug **opt-level 0** backend
controller with a synthetic provider: no filesystem I/O or UI paint is measured.
`N` includes the invisible root; `K` is the number of settled cached directory
sizes. The existing baseline (`_build/verification/tree-performance-20261113/baseline-scaling/result.json`)
was retained without a repeat:

| Fixture | Existing baseline | Corrected candidate |
| --- | --- | --- |
| Wide `N=1,000`, depth 1, `K=0` | 720.696 ms; completed | 496.748 ms; completed |
| Wide `N=10,000`, depth 1, `K=0` | Censored at 60 s; 7,935 callbacks | 50.83 s; completed |
| Wide `N=10,000`, depth 1, `K=1` | Not measured | 50.01 s; completed; candidate-only |
| Broom `N=10,000`, depth 32, `K=0` | Not measured | Censored at 60 s; 9,936 callbacks / 9,920 published entries; incomplete; candidate-only |

The completed 1k comparison is one unrepeated pair, without a statistical claim.
The 10k baseline and candidate broom are censored observations, not completed
durations; the broom has `failureNone=false`. `K=1` and the broom have no baseline
comparison.
The initial compiler artifact was stale because copied source timestamps preceded
Cargo dep-info. It was rejected before timing and retained as
`stale-fingerprint-*` evidence. Only the freshly rebuilt corrected candidate was
admitted; the existing baseline was not rerun. This cap16 aggregate remains failed.
The later cap128 gate below has its own source identity and recorded scope; it
does not rewrite this result. The earlier `fd94d85`/`c6c371bc` application predates
both source increments.


The later cap128 measurements and their synthetic-only limits remain in the
[current cap128 section](../ACTIVE_WORK.md#recursive-tree-cap128-gate).

## Selected local gates and Linux/probe provenance

The `panel-scroll-costs-baseline.json` diagnostic measures bounded row/model
behavior and synthetic pressure, not native input-to-paint or universal retention.

The Root-approved alias-only correction for `tests/copy_dialog.rs:666`
`type_complexity` was applied as `82f0a5a3…edd26`, preserving bodies/assertions.
Root confirmed the original watch callback-lock P2 closed against applied
`watch/tree.rs` hash `a17e9279…b96d5`; generic provider watch contracts stay separate.

The worker `cfg(test)`-only correction's actual receipt is
`_build/verification/worker-platform-libtest-clippy-20261113/result.json`.
Strict Clippy `--lib --tests` passes default **18.3488 s** and
`startup-diagnostics` **1.6885 s**, both exit **0** with libtest artifacts;
**nine preserved inputs / zero drift**. Root reports rustfmt **0**. Runtime tests
were not executed and the app was not rebuilt; `516fc158` remains the qualified
runtime. These local checks do not establish a full remote CI pass.

The Linux receipt is
`_build/verification/build-20261113/linux-tree-native-1714d99/result.json`,
with its raw `test.log`: one real symlink test passed on WSL/tmpfs. The wrapper's
CRLF-only exit 2 was corrected without rerunning the test; Linux desktop UI remains
outside this qualification.

The **updated** Linux link-maintenance fixture also passes **1/0/0**, compile/test
exit 0 and `source_drift=[]`, with the frozen source overlays. Its receipt is
`_build/verification/backend-tree-link-refresh-20261113/linux-engine-only-20261112T235114Z/result.json`.
This extends WSL/tmpfs filesystem coverage, not Linux desktop UI qualification.

The Source revisit probe is complete and archived:
`_build/verification/viewer-20261112/source-revisit-probe/status.json` records
1,098 operations, 14 reference streams, 732 exact replay comparisons, no mismatch,
a 53,144-byte maximum request and zero reads for the logical 10 GB distant-Locate
control. It made no production change and does not explain the reported native
drag delay on the 134-line C++ file. Its machine-specific harness is no longer a
repository test target; native input-to-paint attribution remains open.

The original UI 69/Backend 158/1/refined 11/0, CSC overlap and thumbnail 12/2 → 14/0
histories remain in the tracked recursive/CSC reports linked above. CSC warning
retention originally passed UI `20f507c` **17/0** and Engine `3826298` **5/0**.
The integrated presenter **62 = tree44 + folder presence9 + elevation9**, plus
disclosure5 and tree panel2, gives **69**, not 69 plus the earlier CSC counts.
Mock helper denial/cancellation/transport and event retention do not prove
successful privileged CSC access. Recorded
interaction subsets were Windows copy dialog 8/0, Native Jobs 2/0,
native-operations UI 4/0 and viewer image/pixel preparation 26/0; they do not close
all native input/layout/viewer/platform acceptance. Portable panel commit
`3bee1dc` records 4/0 regression tests and no production behavior correction.
CMD-02 `21c9c7a` records 37/0/0 after historical 31/5 setup failures; remaining
Linux/raw-name/native workflows are separate.

<a id="qualified-ui-checkpoint-35a159c"></a>

### Earlier bounded evidence

The `35a159c` positive-input gate passed 20 distinct tests (native Filter 17,
Filter library 2, exact operation-lifetime 1); exact-first replay overlaps those
counts. Its `0bf32ff3` private runtime and shared-engine retry remain historical.
The initial shared executable-hashing failure in `app-smoke-results.json` remains
unattributed; later stable checks, diagnostics and new startup passes do not
identify its original changed field. Existing embedded-modal, selection and
font-cache behavior remains covered only by each recorded source/test scope.

The corrected Markdown gate is **72 passed / 0 failed across 12 targets**.
Its strict Clippy receipt,
`_build/verification/viewer-20261112/candidate-corrected-clippy-result.json`,
also records exit 0 at **19:41:22 CET**. The correctness summary is
`candidate-corrected-correctness/summary.json` in that viewer evidence directory.
These results qualify the corrected source manifest; matching packaged-worker
deployment/native scrolling qualification is not established by these receipts. Controlled debug request wall-time
and allocator measurements below supported Perf's source-integration acceptance;
Markdown is committed at `255fbdc`. Native scrolling/deployment and HTML viewer
qualification remain separate.
Earlier 69/2 artifacts retain only their pre-correction scope.

The controlled debug A/B summary is
`_build/verification/viewer-20261112/corrected-comparison-paired-20261112-1920/summary.json`.
All **32 profiling passes** completed with exact serialized-output equivalence.
Large-fixture backend request wall-time medians are about **23 ms → 11–15 ms**,
with **383,320 / 461,739 bytes** more per-request incremental allocator peak
(not total process memory or RSS). Small-fixture medians are approximately flat;
p95 rises **0.038–0.298 ms**, with mixed maxima. Timings use `Instant`, not process
CPU time. `_build/verification/perf-20261112/markdown-ab-acceptance-review.md`
accepts source integration with those costs. Native scrolling, packaged-worker
deployment and release/OS-cold qualification remain open.

HTML diagnostic-only instrumentation is committed at `e6e086c`.
`_build/verification/build-20261112/html-validation-result.json` records **20
tests passed**, strict Clippy/default build exit 0 and unchanged frozen source
hashes. `nativeCapturesRun=false`; no HTML performance improvement or native
qualification is claimed.

The later two-attempt Source/PNG bundle,
`_build/verification/viewer-20261113/native-baseline/build-owned-two-case-20261113T003328Z-e4cf9604/result.json`,
records failed foreground admission for both attempts and no qualified native
performance baseline. Owned jobs stopped and source/runtime/preferences/fixtures
were preserved. The partial PNG review (`_build/verification/perf-20261113/png-partial/REVIEW.md`)
places the dominant observed gap before the first counted image bytes; it does
not identify decoder cost or establish F3, completed native redraw or observer
overhead. The subsequent headless host control (`_build/verification/viewer-20261113/image-startup/headless-host-startup-20261113T005252Z/result.json`)
passed **1/0/0**, compile 0 and zero drift, against the unchanged `269f3419`
runtime. Its phase summary (`_build/verification/viewer-20261113/image-startup/headless-host-startup-20261113T005252Z/phase-summary.json`)
records **7,192.133 ms** from environment preparation to process creation returning,
before counted image bytes. This authored 2x1 BMP common-host control is not PNG
decoder, production F3, UI/GPU or smooth-scroll qualification. At that earlier
snapshot, image prewarm was frozen for review/execution and no measured
improvement had been established.

<a id="image-prewarming-source-and-evidence"></a>

The reviewed 14-file prewarm manifest (`_build/verification/viewer-20261113/image-startup/prewarm-candidate-final.json`)
was SHA256 `d08cd7dfd20d875239b116ed5974e10ac259dcee096ef9e4c0c44f21a39babda`.
Root and Perf independently matched its source hashes without drift; the
source-delta review (`_build/verification/perf-20261113/prewarm-diff-review/final-deltas/reviewed-hashes.json`)
reported no source blocker before execution. The subsequent compile receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T015808Z/build-result.json`)
and strict workspace Clippy receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T015808Z/clippy-result.json`)
each exit **101**, with zero drift. Two legacy test `Service` initializers lack
new fields; the owner-correction receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T015808Z/owner-correction-needed.json`)
records **0 tests / 0 native runs** and released source/target leases. Viewer owns
the two mechanical fixture corrections after that release; both are included in
the subsequent 16-file R2 freeze (`_build/verification/viewer-20261113/image-startup/prewarm-candidate-r2.json`),
SHA256 `748cd46fc9d922f117ee5f264658d78705f2bb199106de963e6e209c5e6fb419`.
R2 execution resumed under Build. Its fresh compile receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/build-result.json`)
records exit **0** with zero drift. The first deterministic receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/deterministic-result.json`)
is **34 passed / 9 failed / 0 ignored**; all nine failures are host-fixture trust
rejections before test logic. The same-binary private-fixture rerun (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/host-final-private-result.json`)
passes the 13 host cases with zero failures/ignores/drift. Four host successes
overlap the initial 34: **34 + 13 − 4 = 43 unique deterministic cases**.
The final trusted fixture cleanup (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/trusted-fixture-cleanup.json`)
reports zero remaining entries; the trust receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/fixture-root-trust-result.json`)
confirms ancestor ACLs unchanged. Runtime/package hashes remain unchanged.

The five native headless controls (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/native-result.json`)
pass **5/0/0**, with zero drift, all **1,382 runtime files unchanged**, and owned
children released. The pinned worker package is the retained `269f3419` runtime;
these controls use the candidate host executable rather than the application UI.
The paired control trace (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/native-paired/stderr.log`)
records one same-candidate authored 2x1 BMP comparison:

| Host-control interval | Observed time |
| --- | --- |
| Cold Open to exact CPU pixels | 253.919 ms |
| Ready Open to exact CPU pixels, including claim | 139.410 ms |
| Claim identity validation within ready Open | 71.182 ms |
| Preparation before ready Open | 204.803 ms |

Preparation performs zero source validations/reads; ready Open records zero
process-creation attempts/successes. Preparation is separate from ready Open;
claim validation is already included in 139.410 ms. The
final native fixture cleanup (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/final-fixture-cleanup.json`)
reports zero remaining entries. The older 7.192-second control retains its own
build/environment scope. This host pair does not time engine manager/refill,
application F3, other images, scrolling/smoothness or deployment.

The R2 strict workspace Clippy receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/clippy-result.json`)
exits **101**, zero drift, on the unrelated copy-dialog test `type_complexity`
warning. The Root-approved alias-only correction (`_build/verification/ui-20261113/copy-dialog-clippy/applied-receipt.json`)
is applied, preserving every body/assertion; final fixture/lint qualification is
passed in the final R3 gate below. The later `fd94d85`/`c6c371bc` startup qualification is
[retained separately](#previous-windows-launch-qualified-runtime-c6c371bc); that startup
does not add application F3 or Source-gesture performance qualification.

The final 17-file R3 manifest (`_build/verification/viewer-20261113/image-startup/prewarm-candidate-r3.json`)
is SHA256 `a28d6c5c696b4183fb24285937b07c762eecc4d67ecf8ed51feda34783715054`.
Its compile receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T023426Z/build-result.json`)
and strict workspace Clippy receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T023426Z/clippy-result.json`)
both exit **0**, with zero source drift. The R3 host receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T023426Z/host14-result.json`)
records **14 selected / 14 passed / 0 failed / 0 ignored**, the child exited,
fixture root absent, no remaining entries and unchanged ancestor ACLs. Those
14 host cases replace the earlier 13: **14 + 30 = 44 unique deterministic cases**.
The retained **five native controls and paired timings are R2 evidence**; R3 does
not relabel or repeat them. Production prewarm sources and existing native-control
bodies are byte-identical to R2; R3 changes only fixture cleanup/constructor-panic
coverage and UI test aliases.

Perf's measured acceptance (`_build/verification/perf-20261113/image-worker-startup/MEASURED-ACCEPTANCE.md`)
accepts this one same-candidate ready-hit host observation. Ready steady-idle
memory/CPU, complete-run peak resources, application F3, Source scrolling and
all-image performance remain unqualified. Root committed/pushed the 17 source/test
files and two prewarm docs as `fd94d85`.
The subsequent `c6c371bc` private startup is qualified separately; this host-control
result itself does not establish application performance or tree optimization.

Earlier size/model tests confirmed that each folder's active flag is separate
from warning severity: terminal partial totals keep normal metadata number
styling, `≈`, yellow warning/details and truthful partial/cancelled state, while
only actively unfinished totals are highlighted. `size_errors` 2/0,
`panel_warning_details` 4/0 and `tree_folder_sizes` 7/0, plus the selected library
groups, are preserved in
`_build/verification/docs-20261112/ui-followup-terminal-20261112T174659Z.json`.
The latest modal gate does not rerun that entire scope. See the
[tree size contract](../developer/TREE_PANEL.md#directory-sizes). Full-control
clipping/native-layout qualification remains open.

The [12 November panel qualification](2026-11-12-panel-interaction-polish.md)
records bounded tree publication/command admission, captured actions, diagnostics,
root-level native directory comparison, current deselection defaults and the coral
PDF palette correction. It records final Windows UI library **984 passed / five
ignored**, Linux UI library **979 passed / two timing benchmarks ignored**,
additional targeted gates and final debug backend/native/software startup smokes.
Those are recorded results, not tests rerun during this documentation pass, a full
workspace/remote CI pass or accepted UAC/polkit qualification.


### Retained remote CI boundary

The pending/failure wording below is the earlier recorded boundary. Subsequent
exact-head `9176272` remote CI is 20/20 GREEN; current `174c5fd` format failures
and debug-retry state are in the [current ledger](../ACTIVE_WORK.md#retained-remote-ci-boundary).

The newer retained failure is source `02b2f2f`, run `43367992268`, job
`131699200987`: the job stops on the two worker libtest Clippy warnings. The
`cfg(test)`-only local correction is committed/pushed as `38800aa`; its default
and diagnostics `--lib --tests` checks pass locally. Its remote result remains
pending; no full remote CI pass is established.

At pushed `fe8a4177`, the CI engine gate (`_build/verification/ci-engine-warnings-20261113/gate-20261113T011857Z/result.json`)
records strict Clippy 0, compile 0 and **31/0/0**: recursive 21 + link 10, zero
source drift. These overlap earlier feature gates. The unchanged complete Clippy
step now precedes long workspace tests. Workflow validation (`_build/verification/ci-engine-warnings-20261113/workflow-static-validation.json`)
passes exact-item-move and step-sequence checks with all other YAML bytes preserved;
no full YAML parser was invoked and no full-suite pass follows.

Historical [run 43345016027](https://github.com/TomasJelinek/fm/actions/runs/43345016027)
has **18 successful / 2 failed host jobs**. Both hosts stop at `needless_range_loop`
in `crates/midnight-viewer-ui/src/image_presentation_tests.rs` lines 682/834.
The two-loop mechanical iterator correction is frozen at SHA256
`706f96f00ea7dcca842aa76ae44b596f2d2747463afe0501bdc91f5a5cf20369`;
the combined prewarm compile/Clippy attempt failed before tests at legacy `Service`
fixtures. Corrected R2 compiles with zero drift and has 43 unique deterministic
passes; workspace Clippy still exits 101 on the unrelated copy-dialog test
`type_complexity` warning. Its UI alias-only correction is applied; final
fixture/lint gate passes compile/strict Clippy 0 and host14/0/0, zero drift. This is a test-only change,
not a viewer behavior or performance correction. The earlier receipts below keep
their original source/failure scopes.

The initial local Linux UI compilation stopped on missing `pkg-config`; **no tests
executed in that attempt**. After documented CI prerequisites were resolved without
feature changes, the local Linux gate passed **13 selected / 13 passed / 0 failed /
0 ignored**, compile 0 and `source_drift=[]`: `tree_clipboard` 5, `tree_copy` 2 and
`tree_move` 6. Exact receipt:
`_build/verification/backend-tree-link-refresh-20261113/linux-green-prerequisites-retry-20261112T235508Z/result.json`.
This is WSL/tmpfs software UI/filesystem evidence, not Linux desktop or remote CI
qualification. The initial prerequisite failure is not a runtime result.


At exact `1714d99`, the [Linux CI job](https://github.com/TomasJelinek/fm/actions/runs/43324388443/job/131555743277)
records `tree_clipboard` **3 passed / 2 failed / 0 ignored**. The stale
[tree_filters helper](../../crates/midnight-ui/tests/support/tree_filters.rs) expected
a separate native child instead of the Main embedded confirmation; failure occurs
before Return and native Move/byte assertions. The helper correction is committed in `1546030`; the local **13-test Linux runtime gate passes 13/0/0** with zero drift. Available evidence
is `_build/verification/build-20261113/ci-first-failure-diagnosis.json` and
`github-checkpoint-1714d99-job-131555743277-20261112T232014Z.json`.
This job did not reach the Linux recursive target;
its separate WSL/tmpfs 1/0/0 receipt remains independent.


The older retained terminal remote receipt is
`github-checkpoint-8b18892-terminal-20261112T221914Z.json`:
[run 43312251721](https://github.com/TomasJelinek/fm/actions/runs/43312251721) at
`8b18892` completed with **18 successful / 2 failed host jobs**. Both hosts
failed three `operations_native_ui` assertions at
`tests/support/operations_native.rs:193` (`get_dialog_input_blocked()`).
The later integrated Windows target passes 4/0; Linux confirmation/current-main
remote success remains separate. No retained terminal receipt qualifies
`1714d99` as a green remote run.

Earlier `955068c` Native Jobs notice failures, `35a159c` compact-feedback
failures and `2e0e554` operation/Filter failures remain dated history. Local
selected notice/operation/compact passes do not rewrite those CI outcomes or
qualify later targets that a failed run never reached. CMD-02's source timing
question was resolved by `cmd02-corrected-provenance.json`; its 37/0 gate remains
a targeted Windows result, not a full suite.


### Completed build-script integration

For merge `28cc79add548eed0d6c9e465a6fe26a5d141f9b7`, the Orchestrator reports:

| Gate | Final reported result |
| --- | --- |
| Upstream standalone helper suite | 20 passed |
| Combined helper suite | 46 cases: 45 passed, one platform skip |
| Windows hooks | Seven passed |
| Shell syntax | Six script checks passed |

These groups are not added together as unique coverage. They are helper/hook/syntax
results, not a live package-installation, Docker-image, application or native UI
qualification. Documentation did not rerun them. Build implemented the
[cleanup mount guard](../../scripts/check-clean-mounts-linux.sh) fixes after the
Orchestrator's diagnosis and review: kernel escapes use
exactly three octal digits, and decoding preserves following filename digits and
trailing path newlines.


### QA artifact cleanup

The user explicitly authorized deleting the entire `_build/qa/` tree; Build owns
the operation and its space measurement. QA paths retained below are historical
provenance, not a promise that those files remain available or a prerequisite to
reading this durable summary. New final documentation checks and seven-file hashes
are stored under `_build/verification/docs-20261112/`, outside QA. Cleanup is
complete: the Orchestrator verified `_build/qa/` absent. Build's accessible stage
removed **681 roots**, with a reported space gain of **40,625,377,280 bytes**
(about 40.6 GB). The reviewed frozen UAC helper completed the protected remnants
at **17:15:21 CET (16:15:21Z)**, using per-directory ACL repair without traversing
reparse targets. Its final-stage free-space change was **155,648 bytes**; the final
free-space snapshot was **46,807,683,072 bytes**. These observations are not summed
because unrelated disk activity continued; the earlier four-BMP cleanup is separate.
The completion receipt is
`_build/verification/20261112-cleanup/qa-elevated-completion.json` (`completed`,
`directoryAbsent: true`). Protected debug SHA-256 before and after is unchanged:
`d731397f751647c5feb6ff2ba6ec822a598711bc48ee738e452f364363138984`.


### Original Markdown allocation handoff

The subsequent Viewer handoff records **62 independent Markdown plugin tests
passed, zero failed, one manual measurement target ignored**, strict Clippy and a
candidate DLL build. Bounded page selection now borrows retained semantic context
and clones only selected complete layout groups; cache/read/wire/Unicode bounds
are unchanged. The one-line forward allocation regression fell from **452,189 to
8,383 bytes**, with no additional source reads. Saved response streams from the
two fixture comparisons were byte-identical. Diagnostic timing was mixed, including
a slower small retained fixture; this is not an end-to-end speed or frame-pacing
improvement claim. The candidate was not installed by that handoff. Its debug DLL
SHA-256 was `1F3C04FCD6E4DF7FFC478DDDF32A1E68D7672D92349620819317A37B9F2715BD`.
The original `_build/qa/viewer-markdown-20261112/handoff.txt` and candidate may be
removed by the authorized full QA cleanup; the counts, allocation result and scope
are preserved here. The Orchestrator checked the source receipt and accepted
those five plugin files as `cc4857e`. Fresh host/native integration must rebuild
the candidate and supply its own evidence.


## Native Rename gate history

Public API/SDK Task 1 `f3f8c59` and docs `852ee0c` were integrated at `e5cba5d`.
`_build/verification/build-20261112/api-sdk-integration-result.json` records
all 33 incoming files byte-equal with zero dirty overlap; API/SDK tests, corrected
Rename tests, strict Clippy, direct Windows/network consumers and C host checks
retain their individual original scopes.

<a id="native-rename-audit-gates"></a>

The later C native Rename audit-fix snapshot passes Linux **38 selected /
38 passed / 0 failed / 0 ignored**, compile 0 and zero source drift, under real
Linux WSL/tmpfs. The exact receipt (`_build/verification/backend-rename-audit-fixes-20261113/linux-green-20261113T014351Z/result.json`)
and raw logs cover recovery 23 + prepared Rename 9 + conflicts 6; no fixture
paths remain and compile/test children stopped with source/target leases released.

The later Linux compatibility-only checkpoint `9176272` changes `fetch_update`
to `try_update` and uses test `Permissions::from_mode(0o0)`. Its strict all-targets
Clippy gate exits **0** in **13.8297 s**, including actual library `cfg(test)`,
with zero drift. Runtime tests were not rerun; capability semantics are unchanged.

The preceding Windows gate (`_build/verification/backend-rename-audit-fixes-20261113/windows-green-20261113T012845Z/result.json`)
compiled successfully with **21 passed / 1 failed / 0 ignored** from 22 selected,
zero drift. Both spelling regressions passed; the remaining failure was exact
DACL/control restoration in the test fixture. Its fixture-only correction (`_build/verification/backend-rename-audit-fixes-20261113/windows-final-green-freeze.json`)
is applied without changing production ACL/spelling code. The same 22-case repeat (`_build/verification/backend-rename-audit-fixes-20261113/windows-fixture-green-20261113T014954Z/result.json`)
passes **22/0/0**, compile 0, zero source/binary drift and no remaining temporary
entries. Recovery/unit 12 + conflicts 6 + prepared Rename 4 retain the original
selection; exact file/directory ACL readbacks are iterations of one existing test,
not additional cases. Processes exited and source/target leases were released.
The preceding 21/1 remains historical. Task 4A's exact candidate is accepted
independently by Root and Architect and applied in C. Its reservation is exactly
`admission.rs`, `ffi.rs`, `discovery.rs`, `ffi_tests.rs` and
`rename_admission_tests.rs` under `crates/midnight-native-filesystem-host/src/`.
The applied host freeze (`_build/verification/backend-native-host-task4a-20261113/applied-freeze.json`)
records all five applied hashes matching candidate manifest
`c8d2463d902a41a6a79fc0ef5834b29ec8307e2504d783a9b1976a83d6c021ae`.
The Build handoff (`_build/verification/backend-native-host-task4a-20261113/applied-build-handoff.md`)
selected **44 unique direct host tests** (admission 1 + discovery 14 + FFI 29).
The fresh C host foundation receipt (`_build/verification/backend-native-host-task4a-20261113/build44-20261113T020849Z/result.json`)
records compile **0**, Clippy **0**, test exit **0**, **44 passed / 0 failed /
0 ignored** and **252 captured inputs / 0 drift**. The test executable's before/
after SHA256 is
`a965fa7cc8e593530e6079b4ad7d6c89704a4eca991a75b690be78b0ed05f196`;
the child exited and source/target leases were released. This qualifies the C-only
host foundation, not deployed application Rename. Root checkpointed these five
reviewed/qualified host files as `eb2eca8` on C branch `codex/main-deep-audit`;
the 44/44 and scoped Clippy receipts apply unchanged. Only those five files were
committed. Root later integrated this host foundation and Task 4B `cf64a6c` in
D merge `02b2f2f`. Unfinished native-plugin sources remain separate; source
integration does not establish application deployment.

After Architect agreement, Root accepted Task 4B's finite finish seam with an
owned prepare-failure carrier. Its **11 C core/host paths** (2 core + 9 host) are
committed as `cf64a6c`; the earlier
reservation baseline (`_build/verification/backend-native-host-task4b-20261113/baseline.json`)
records those paths. The [core/host contract](../developer/NATIVE_FILESYSTEM_API.md#prepared-rename-corehost-lifecycle)
and [disposal boundary](../developer/ENGINE_LIFETIME.md#prepared-rename-disposal-boundary)
describe the implementation; engine/UI integration remains unimplemented.

The preceding Windows/Linux core/host harness snapshots each passed **79/0/0**
with compile **0**, while strict Clippy exited **101** on deprecated `fetch_update`.
The atomic-helper recheck passed one overlapping case per platform but exposed
the test-helper `large_enum_variant` lint, also Clippy **101**. After boxing that
helper, the final narrow gate (`_build/verification/backend-native-host-task4b-20261113/test-helper-boxing/build-owned/combined-result.json`)
passes the same two exact cases on each platform: **2/0/0 Windows + 2/0/0 Linux**
(**four executions**), compile/strict scoped Clippy **0**, **28 stable pins**,
zero source/pin drift, all owned compiler/test children stopped and leases
released. The earlier 79/79 remains preceding runtime evidence, not a new
83-unique-case result or a broad 79-case rerun after boxing.

Root integrated Task 4A `eb2eca8` and Task 4B `cf64a6c` into D at `02b2f2f`.
These core/host harness gates do not qualify installed-provider/native runtime Rename. Engine/UI cutover,
physical Linux desktop and installed host workflows remain open; no native
Rename deployment is established.
