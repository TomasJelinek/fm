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

Role domains are not blanket file ownership. The Orchestrator records exact file
reservations and coordinates Cargo and native-input leases across shared and
explicitly arranged isolated checkouts.

Each handoff records its exact source/build identity, changed files, behavior,
commands and test counts including failures/ignores, environment, native versus
substituted boundaries, artifacts under `_build/`, unresolved failures and
remaining acceptance. Documentation-only checks do not qualify the application.
Commit, merge and push remain coordinated through the Orchestrator.

Current assignments, integration checkpoints, receipts and open requirements live
in the canonical [Active work checkpoint](../ACTIVE_WORK.md#current-checkpoint)
and its surrounding ledger. Update that ledger when evidence changes; keep this
document focused on responsibilities and the working agreement.
