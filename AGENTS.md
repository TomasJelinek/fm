# Agent guide: Midnight Explorer

Starting context for agents (and people) working in this repository: what it holds,
what to read first, the commands that build, test and validate it, and the project
rules that every change follows. Project terms are defined in the
[glossary](docs/glossary.md).

## What this repository owns

- **The application:** Midnight Explorer, a two-panel desktop file manager for Windows
  x64 and Linux x86-64, written in Rust with Slint (the `crates/` workspace; the
  executable is `midnight-explorer`).
- **The public plugin APIs and SDKs:** separate Cargo workspaces (`api/`,
  `filesystem-api/`, `image-viewer-api/`, `thumbnail-api/`, `web-viewer-api/`,
  `worker-api/`) that plugins build against.
- **The shipped plugins:** viewers, filesystems (native, Network Neighborhood and the
  archive worker) and thumbnailers under `plugins/`, each an independent Cargo
  workspace that depends only on the public API/SDK, never on `crates/`.
- **Build, packaging and release tooling:** `build.bat`, `build.sh`, `scripts/`,
  `packaging/` and the CI workflows in `.github/`.

Not owned here: third-party dependencies (listed in
[third-party.md](docs/third-party.md)), apart from the patched renderer sources in
`vendor/`; and the [Open Salamander](docs/glossary.md#salamander) sources that some
documents cite as behavioral reference, which are never copied.

## Start here

1. [README.md](README.md): the product, its first run and the build commands.
2. [docs/README.md](docs/README.md): the documentation map, with every current
   contract and reference.
3. [docs/developer/README.md](docs/developer/README.md): the architecture, the plugin
   APIs and the index of developer documents.

Before changing code, also read [CODE_STYLE.md](CODE_STYLE.md),
[SECURITY.md](SECURITY.md) and the [project rules](#project-rules) below. The
[user guide](docs/user-guide.md) describes the behavior users see, and
[feature status](docs/feature-status.md) what is implemented and verified.

## Repository layout

| Path | What it holds |
| --- | --- |
| `README.md` | Product purpose, download, first run, build and support. |
| `AGENTS.md` | This guide and the project rules. |
| `docs/README.md` | Documentation entry point. |
| `docs/developer/` | Developer and plugin API documents, with the [quick start](docs/developer/quick-start.md) for a plugin. |
| `docs/releases/` | Release notes of each preview. |
| `docs/verification/` | Dated test evidence; records, not current documentation. |
| `crates/` | The application workspace: core, engine, platform adapters, UI and the executable. |
| `api/`, `*-api/` | Public API/SDK workspaces. |
| `plugins/` | Independent plugin workspaces. |
| `scripts/` | Build, packaging, validation and test helpers, with their tests in `scripts/tests/`. |
| `packaging/` | Release package files: the packaged README texts, the Linux desktop entry, dependency license texts and the update public key. |
| `vendor/` | Patched third-party renderer sources ([vendor/README.md](vendor/README.md)). |
| `.github/` | CI workflows and their shared actions. |
| `_build/`, `.tools/` | Build output and toolchain caches; ignored, never committed. |

## Commands

Run every command from the repository root. Prerequisites for all of them: Rust
1.99.1 with rustfmt and Clippy ([rust-toolchain.toml](rust-toolchain.toml)); on Windows
the MSVC C++ build tools, PowerShell 7 and Python 3.12 or later; on Linux a C compiler,
`pkg-config`, the libraries that `scripts/setup-linux.sh` installs and Python 3.11 or
later (3.12 for validation). The [user guide](docs/user-guide.md#build-and-run) lists
them in full.

| Task | Windows | Linux |
| --- | --- | --- |
| Build the application and every plugin | `build.bat` (add `release` for an optimized build) | `./build.sh` (add `release`) |
| Format check | `cargo fmt --all -- --check` | Same |
| Lint | `cargo clippy --workspace --all-targets --locked --features midnight-acceptance/acceptance -- -D warnings` | Same |
| Workspace tests, as CI runs them | `python -B scripts/ci-workspace-tests.py --keep-test-artifacts` | Same |
| Targeted tests of a change | Build the affected tests once with `cargo test --no-run`, then run them with `scripts/run-targeted-tests.py` ([targeted test execution](docs/developer/test-execution.md#targeted-test-execution)) | Same |
| Source-size check | `pwsh scripts/check-source-size.ps1` | `bash scripts/check-source-size.sh` |
| Full validation (every CI test of the platform) | `build.bat --validate` or `pwsh -NoProfile -File scripts\validate.ps1` | `./build.sh --validate` or `bash scripts/validate.sh` |
| End-to-end acceptance suite (opt-in) | `pwsh -File scripts\acceptance.ps1` | `bash scripts/acceptance.sh` |

Validation and the acceptance suite are long; follow the
[project rules](#project-rules) on when to run broad suites, and the
[test execution guide](docs/developer/test-execution.md) for steps, options and
environment.

## Confirm the application runs

A passing build and passing tests do not prove that the application starts in this
environment. After a build, run the filesystem smoke check; it needs the native
filesystem plugin beside the executable, which the build helpers install, but no
display:

```sh
cargo run -p midnight-explorer --locked -- --smoke-test
```

It prints `volumes: N`, the number of entries in each panel's start folder (`Left: N
entries`, `Right: N entries`) and whether other accounts can change the installation
(`installation: private`, or `installation: shared; …`), and exits with status 0. Run directly, `midnight-explorer.exe` on Windows is a GUI
program whose output a console does not show, so read its exit code instead (the
[user guide](docs/user-guide.md#windows-x64) has the command). With a display,
`cargo run -p midnight-explorer --locked -- --ui-smoke-test --software` also starts the
main window with fresh in-memory settings and exits. A failing start is diagnosed in
[Troubleshooting](docs/troubleshooting.md).

## Project rules

- Third-party libraries must be usable for the intended development, application
  use and distribution **without mandatory fees, subscriptions or royalties**.
  Prefer maintained open-source libraries and compatible permissive licenses.
  Trial-only and paid-only dependencies are not acceptable.
- This applies to direct/transitive dependencies of the app, public API/SDK,
  independent plugins, build tools and tests. Free of charge does not waive
  attribution, source-distribution or other license obligations.
- On dependency changes, check the actual pinned license and enabled features,
  update the [inventory](docs/third-party.md) and distribution notices, and follow
  the [dependency policy](docs/dependency-policy.md). Do not silently change the
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
[Agent workflow](docs/developer/agent-workflow.md).

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
  coordinated through build leases. Implementation roles own fixes; the Architect
  owns architecture decisions.
- In a shared checkout, role agents do not switch branches, merge, stage another
  role's files or start competing builds. The Orchestrator coordinates Git
  checkpoints and integration, or explicitly arranges isolated checkouts.
- Independently developed plugins depend only on the public API/SDK and own their
  tests. Changes to host sources (the application in `crates/`) require a separately
  assigned host change.
- One owner holds each Cargo target-directory build lease. Compile compatible
  test groups once, run independent headless tests with bounded parallelism and
  run desktop/input tests serially. Coordinate full validation after integration.
- Handoffs state changed files, behavior, exact test commands/results, remaining
  risks and unresolved work. Documentation distinguishes implemented behavior
  from plans and records only verified evidence; a test or CI job's existence is
  not proof that it passed.

[Build, desktop and WSL leases](docs/glossary.md#lease) are coordination rules for
development only.
