# Agent responsibilities and integration

The orchestrator coordinates the task queue, file ownership, build leases,
reviews and integration. The role chats below are persistent points of contact;
creating a chat does not authorize it to start unrelated backlog work.

| Role | Responsibility |
| --- | --- |
| Orchestrator | Task queue, acceptance criteria, exclusive file ownership, build/test leases, review and Git integration. |
| Architect | Interface and architecture decisions, threading/ownership contracts, cross-platform boundaries and design reviews. |
| Backend agent | Engine services, filesystem operations, native platform adapters and filesystem plugins, with their tests. |
| Viewer agent | Viewer plugins and their private dependencies/tests, viewport loading, decoding and rendering performance. |
| UI agent | Slint presentation, dialogs, themes, keyboard/pointer interactions and frontend integration with implemented backend APIs. |
| GitHub build system and build tools developer | Build/test tooling, CI diagnosis, coordinated test runs, packaging and authorized publishing. |
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
5. This checkout is shared unless an isolated checkout is explicitly arranged.
   No role switches branches, merges, stages another role's files, or starts a
   competing build. The orchestrator coordinates the Git checkpoint and merge;
   the build role handles publishing when authorized.
6. Reports state changed files, behavior, test commands/results, remaining risks
   and unresolved work. Documentation records implemented behavior separately
   from plans and does not infer success from the presence of a test or CI job.
7. Generated files belong under `_build/`, tool caches under `.tools/`. Follow
   repository security, Unicode, source-size and free-dependency requirements.
8. Later accepted user refinements supersede conflicting older stage plans.
   Source establishes the current implementation boundary; dated results establish
   only their recorded source/environment. A green subset or checked plan does
   not close a requirement's remaining failure, native or platform acceptance.

## Active six-role batch — 12 November 2026

The panel/viewer batch has been integrated. The user has resumed the broader goal:
implement all accepted app features, test each deeply and polish the UI. Role chats
now work on the Orchestrator's scoped assignments; onboarding is no longer the
only assigned work. See [Active work](../ACTIVE_WORK.md) for the current ownership,
requirements/evidence backlog and corrections to stale stage references.

Build owns CI run `43239253816` reporting, the useful `origin/fix/cargo` port and
`_build` cleanup. Viewer handed off the accepted `cc4857e` Markdown allocation slice;
host/native latency qualification remains separate. UI owns the filter experience
and shortcut fixtures. Architect's review proposes prepared-action/watch/link
increments, and Documentation reconciles status, requirements and evidence. The Orchestrator
reports CI ended with 18 successful jobs and two failed jobs: Windows comparator
timeout (Backend) and Linux Ctrl+D expectations (UI). Backend/Viewer completed
their historical archive/thumbnail comparisons with no code to recover. The user
authorized selective
recovery/discard and retirement; the Orchestrator removed those exact-tip branches
and clean worktrees. See the [retirement record](../verification/2026-11-12-branch-retirement.md)
for completed versus pending actions. Exact edit paths and Cargo/native-input
leases remain Orchestrator-owned; a role's domain is not blanket file ownership.

Each implementation handoff records its exact source/build identity, changed files,
executed commands, test counts/ignores, native or substituted boundaries, artifacts
under `_build/`, unresolved failures and remaining acceptance. Documentation-only
checks do not qualify the application. Commit, merge and push remain coordinated
through the Orchestrator.
