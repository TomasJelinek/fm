# Agent workflow

Development is split among agent roles that an Orchestrator coordinates. This page
describes the working agreement as it is practised; [AGENTS.md](../../AGENTS.md) holds
the binding rules, and [test execution](test-execution.md) the commands.

## Roles

| Role | Responsibility |
| --- | --- |
| Orchestrator | Task queue, acceptance criteria, exclusive file ownership, build slots and leases, and Git integration; independent critic and reviewer. |
| Architect | Interface and architecture decisions, threading and ownership contracts, cross-platform boundaries and cross-role design reviews. |
| Backend agent | Engine services, filesystem operations, native platform adapters and filesystem plugins, with their tests. |
| Viewer agent | Viewer plugins with their private dependencies and tests, viewport loading, decoding and rendering performance. |
| UI agent | Slint presentation, dialogs, themes, keyboard and pointer interaction, and frontend integration with implemented backend APIs. |
| GitHub build system and build tools developer | Build and test tooling, CI diagnosis, coordinated test runs, packaging and authorized publishing. |
| Perf agent | Independent review of latency, CPU, memory, I/O, contention, algorithms, data structures and hotspots, backed by measurements. |
| Documentation guru | Current design, API and user documentation, feature status, dependency and licence records, and verified test evidence. |

A role works only on assignments that name their acceptance criteria and the files it
owns; having a role does not authorize other backlog work. Shared API or UI changes are
agreed before editing, the Architect settles cross-role design questions, and existing
user requirements take precedence.

## Isolated exports and integration

- Each assignment starts from an export of the current main: `git archive <base>`
  extracted into a folder outside the repository, with the base commit recorded beside
  it. Larger work may use its own clone or worktree, arranged by the Orchestrator.
- An agent edits only the files it owns. It never edits, stages, commits, stashes or
  switches branches in the shared checkout, and it does not commit; the Orchestrator
  integrates.
- Work is handed in in increments. After a hand-in the agent continues in a copy of its
  export, so the handed-in export stays unchanged while it is integrated.
- When the Orchestrator announces a new main, the agent merges its work onto it file
  by file (`git merge-file`) before the next hand-in.
- The Orchestrator reviews each increment for correctness, security, concurrency,
  performance, UX and test evidence, asks the owner for fixes, checks the reported
  results against the source, and then commits it.

## Build slots

- Every Cargo build, Clippy run and test build goes through the machine-wide build-slot
  queue: a waiter takes a ticket, and the oldest live ticket gets the next free slot
  (first in, first out). Slots are few because memory is the limit; a slot whose
  process has died is reclaimed.
- Each agent uses its own Cargo target directory and builds with `--locked --offline`.
  It compiles compatible test groups once (`--no-run`) and runs the printed test
  executables outside the slot, with bounded parallelism.
- The Orchestrator's integration checks have their own lane, taken only while memory
  allows.

## Desktop and WSL leases

- Runs that use the Windows desktop (the acceptance suite, desktop input tests and
  performance measurements) are serial machine-wide under the desktop lease. Linux runs
  in WSL take the WSL lease, one at a time per machine.
- The long-lived process that runs the work takes the lease and releases it when it
  ends, also on failure. A lease is never held while editing or while building on
  Windows. The Orchestrator removes a lease whose process is gone and whose resource is
  idle.
- In WSL each agent works in its own clone, copied from its export without repository
  refs, with the offline toolchain, and deletes the clone's build output when it pauses
  for long.
- Agents never touch processes they did not start, including a running copy of the
  application.

## Hand-ins

Each hand-in states:

- the base commit, the export, the changed files and a patch;
- what changed in behaviour or documentation, and why;
- the exact commands with their results: pass, fail, skip and ignore counts, run ids,
  platform and scale, including the red runs that showed a new check can fail;
- product defects found, with a reproduction, and the remaining gaps and risks.

Documentation records only verified evidence and separates implemented behaviour from
plans: a test or CI job that exists is not evidence that it passed. Dated records are
not edited after the fact, and current documents name no local paths or user names.
