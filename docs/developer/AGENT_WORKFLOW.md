# Agent responsibilities and integration

The Orchestrator coordinates seven specialist chats, the task queue, file
ownership, build leases, reviews and integration. The role chats below are
persistent points of contact; creating a chat does not authorize it to start
unrelated backlog work.

| Role | Responsibility |
| --- | --- |
| Orchestrator | Task queue, acceptance criteria, exclusive file ownership, build/test leases and Git integration; independent critic and reviewer. |
| Architect | Interface and architecture decisions, threading/ownership contracts, cross-platform boundaries and design reviews. |
| Backend agent | Engine services, filesystem operations, native platform adapters and filesystem plugins, with their tests. |
| Viewer agent | Viewer plugins and their private dependencies/tests, viewport loading, decoding and rendering performance. |
| UI agent | Slint presentation, dialogs, themes, keyboard/pointer interactions and frontend integration with implemented backend APIs. |
| GitHub build system and build tools developer | Build/test tooling, CI diagnosis, coordinated test runs, packaging and authorized publishing. |
| Perf agent | Independent performance critic reviewing changes for latency, CPU, memory, I/O, contention, algorithm/data structure suitability and hotspots. |
| Documentation guru | Current design/API/user documentation, implementation status, licenses and verified test evidence from all roles. |

## Working agreement

1. The orchestrator assigns concrete work with acceptance criteria and exclusive
   file ownership. Shared API or shared UI changes are agreed before editing;
   agents do not overwrite another role's changes.
2. The Architect resolves cross-role design questions. Existing user requirements
   take precedence; routine implementation decisions need no extra confirmation.
3. Backend operations remain asynchronous from the UI. Platform implementations
   stay in separate files. Plugins depend only on the public API and own their
   tests; a plugin task cannot edit the host without a separately assigned change.
4. Use targeted tests during implementation. Compile a compatible group once and
   run independent headless tests concurrently with bounded resource use. Desktop
   input tests are serial. One owner holds each Cargo target-directory build lease.
   Run the agreed full validation after integration; report exactly what ran.
   Perf supports findings with profiling and before/after measurements coordinated
   through Build leases. Implementation roles own fixes; the Architect owns
   architecture decisions.
5. This checkout is shared unless an isolated checkout is explicitly arranged.
   No role switches branches, merges, stages another role's files, or starts a
   competing build. The orchestrator coordinates the Git checkpoint and merge;
   the build role handles publishing when authorized.
6. Reports state changed files, behavior, test commands/results, remaining risks
   and unresolved work. Documentation records implemented behavior separately
   from plans and does not infer success from the presence of a test or CI job.
   The Orchestrator independently reviews correctness, security, concurrency,
   performance, UX and test evidence; challenges unnecessary complexity and weak
   proposals; requests owner fixes before integration; and verifies reported
   outcomes against source and actual results.
7. Generated files belong under `_build/`, tool caches under `.tools/`. Follow
   repository security, Unicode, source-size and free-dependency requirements.
8. Later accepted user refinements supersede conflicting older stage plans.
   Source establishes the current implementation boundary; dated results establish
   only their recorded source/environment. A green subset or checked plan does
   not close a requirement's remaining failure, native or platform acceptance.

## Active seven-role batch — 12 November 2026

Qualified and pushed UI source checkpoint `35a159c` commits all 33 reviewed UI files
matching the new private EXE; plugin WIP remains unstaged. Latest completed remote-CI
checkpoint remains `2e0e554`. Embedded modal gating, selection fixture and font
catalog source retain their recorded scope.
Build is the sole build/test owner. The positive-input gate passes **20 distinct
tests**, without adding exact-first replay to the Filter 17/0, Filter library 2/0
and operation-lifetime 1/0 groups. Debug compile and the new private entrypoint's
engine smoke/native UI startup pass, with fixtures unchanged. Shared D's engine
retry also passes; its initial generic hashing failure remains preserved and
unattributed, with no source/validator fix. Corrected Markdown
is not deployed in the private runtime. Remote run 43260032611 remains failed
(18 successful / 2 failed jobs); no broader native/HTML/performance completion is
inferred.
[Active work](../ACTIVE_WORK.md#current-checkpoint) retains exact recorded
scope and source/artifact identities.
The Orchestrator remains the independent critic/reviewer and integration owner.

Current scoped assignments:

- UI's reviewed positive-input correction passes the retained-focus and selected
  boundary tests. The compiled private entrypoint passes engine/native UI startup;
  the initial unattributed shared hashing anomaly and broader native acceptance
  remain separate after the shared engine retry passed.
  Find geometry now passes; earlier Delete/layout/theme receipts retain their scope.
  CMD-02 presenter and focused-test implementation is authorized; targeted qualification is pending.
  Completed inaccessible scans retain `≈`, the yellow warning icon and details,
  with normal metadata color for the number; only actively calculating per-folder
  totals stay highlighted, even while another root runs. This is implemented in
  current source, with earlier targeted size/model, warning-detail and tree-size
  passes recorded separately; cancelled/incomplete states remain truthful. Perf's prior source review
  does not replace execution. Broader native/layout qualification remains open;
  the compact-layout coverage limit is recorded in Active work.
- Architect finalized and the Orchestrator accepted explicit preparation before
  confirmation for Rename: preparation captures identity, with no listing-era
  guarantee. Backend's 30-file API/SDK Task 1 is committed in the existing
  `C:/Users/tjelinek/.codex/worktrees/main-deep-audit/FM` checkout as
  `f3f8c59a577e2b1cb70f8b8285047bd744c960f1`, not integrated into D or pushed.
  Seven corrected prepared-Rename tests, strict API/SDK Clippy and both direct
  Windows/network consumer compilation checks pass. The earlier 83 API/SDK tests
  retain their prior scope. Windows Task 2 and Linux Task 3 plugin source work are
  authorized in C; both remain unqualified and host cutover is open. D's qualified UI checkpoint is
  `35a159c`; other public actions, watches and preserved links keep separate gaps.
- Viewer's corrected partial-context candidate passes 72 tests across 12 targets;
  the newer strict Markdown Clippy receipt also exits 0. Perf found no actionable
  source issue. Measured/native performance qualification remains pending, with
  coordinated Build leases.
- Build owns follow-up compilation and affected UI retests, and every Cargo lease
  across the shared `D:/experiments/dev/FM` UI/Viewer checkout and Backend's existing
  isolated checkout. The new private runtime entrypoint and SHA-256 are recorded in
  Active work; engine/native UI startup passes there and the shared engine retry
  passes. The original generic hashing failure remains preserved/unattributed;
  later stable-handle checks do not establish a source/validator fix.
- Documentation owns the D status records plus only C's public protocol and API/SDK
  README files for ABI 1.3. No API/native code or Git/build work is included. No full
  UI suite, remote CI success or native sweep is inferred from the selected gate.

The wider goal remains active: implement all accepted features, test each deeply
and polish UI. Retired branches remain retired; no new branch report is needed.
See [Active work](../ACTIVE_WORK.md) for unmet requirements and completed versus
pending evidence. Exact file assignments and Cargo/native-input leases remain
Orchestrator-owned; a role's domain is not blanket file ownership.

Each implementation handoff records its exact source/build identity, changed files,
executed commands, test counts/ignores, native or substituted boundaries, artifacts
under `_build/`, unresolved failures and remaining acceptance. Documentation-only
checks do not qualify the application. Commit, merge and push remain coordinated
through the Orchestrator.
