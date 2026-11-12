# Project rules

- Third-party libraries must be usable for the intended development, application
  use and distribution **without mandatory fees, subscriptions or royalties**.
  Prefer maintained open-source libraries and compatible permissive licenses.
  Trial-only and paid-only dependencies are not acceptable.
- This applies to direct/transitive dependencies of the app, public API/SDK,
  independent plugins, build tools and tests. Free of charge does not waive
  attribution, source-distribution or other license obligations.
- On dependency changes, check the actual pinned license and enabled features,
  update the [inventory](docs/THIRD_PARTY.md) and distribution notices, and follow
  the [dependency policy](docs/DEPENDENCY_POLICY.md). Do not silently change the
  project's own license or select a paid alternative of a multi-licensed library.
- Follow [CODE_STYLE.md](CODE_STYLE.md) and [SECURITY.md](SECURITY.md). Keep platform
  implementations separate and all non-UI work in backend services. Build, test
  and generated output belongs under `_build/`; toolchain caches use `.tools/`.
- During local implementation, run targeted tests for changed behavior and its
  direct integration boundaries. Defer full test suites until the planned
  implementation is complete; do not repeatedly rerun broad suites between tasks.
- Work on independent tasks in parallel with explicit file ownership. Coordinate
  shared UI integration and Cargo target directories to avoid conflicting edits
  or redundant builds.

## Agent responsibilities

The Orchestrator coordinates seven specialist chats. Each role works on scoped assignments
with acceptance criteria and exclusive file ownership; creating a role chat does
not authorize unrelated backlog work. The detailed working agreement is in
[Agent workflow](docs/developer/AGENT_WORKFLOW.md).

| Role | Responsibility |
| --- | --- |
| Orchestrator | Coordinate the task queue, acceptance criteria, file ownership, build leases and Git integration; act as an independent critic and reviewer. |
| Architect | Own interface and architecture decisions, threading/ownership contracts, cross-platform boundaries and cross-role design reviews. |
| Backend agent | Own engine services, filesystem operations, native platform adapters and filesystem plugins, with their tests. |
| Viewer agent | Own viewer plugins, their private dependencies and tests, viewport loading, decoding and rendering performance. |
| UI agent | Own Slint presentation, dialogs, themes, keyboard/pointer interactions and frontend integration with implemented backend APIs. |
| GitHub build system and build tools developer | Own build/test tooling, CI diagnosis, coordinated test runs, packaging and authorized publishing. |
| Perf agent | Independently review changes for latency, CPU, memory, I/O, contention, algorithm/data structure suitability and hotspots. |
| Documentation guru | Own current design/API/user documentation, feature status, dependency/license records and verified test evidence from all roles. |

## Coordination and handoff

- Agree on shared API or UI changes before editing. The Architect resolves
  cross-role design questions; existing user requirements take precedence.
- The Orchestrator independently reviews correctness, security, concurrency,
  performance, UX and test evidence; challenges unnecessary complexity and weak
  proposals; requests owner fixes before integration; and verifies reported
  outcomes against source and actual results.
- Perf agent supports findings with profiling and before/after measurements
  coordinated through Build leases. Implementation roles own fixes; the Architect
  owns architecture decisions.
- In a shared checkout, role agents do not switch branches, merge, stage another
  role's files or start competing builds. The Orchestrator coordinates Git
  checkpoints and integration, or explicitly arranges isolated checkouts.
- Independently developed plugins depend only on the public API/SDK and own their
  tests. Changes to host sources require a separately assigned host change.
- One owner holds each Cargo target-directory build lease. Compile compatible
  test groups once, run independent headless tests with bounded parallelism and
  run desktop/input tests serially. Coordinate full validation after integration.
- Handoffs state changed files, behavior, exact test commands/results, remaining
  risks and unresolved work. Documentation distinguishes implemented behavior
  from plans and records only verified evidence; a test or CI job's existence is
  not proof that it passed.
