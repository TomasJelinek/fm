# Stage 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** A runnable Rust/Slint application with two independent, responsive filesystem panes and switching between mounted volumes.

**Architecture:** One process, five crates. `midnight-core` defines messages and synchronous platform-provider boundaries; `midnight-engine` runs provider calls on bounded background workers under Tokio. `midnight-ui` owns Slint and the presenter; `midnight-explorer` connects them with a bounded queue and a coalesced UI wakeup.

**Tech Stack:** Rust 1.99.1, Slint 1.18.1, Tokio 1, Windows MSVC; Linux build configuration included.

**Spec:** `docs/superpowers/specs/2026-11-04-dual-pane-file-manager-design.md`, stage 1 only.

## Global Constraints

- Rust, UI ve Slintu, Tokio pro asynchronní orchestrace.
- Jeden proces, oddělené crates, komunikace přes zprávy.
- `midnight-core`, `midnight-engine` a `midnight-platform` neznají Slint.
- `midnight-ui` nevolá filesystem ani konkrétní engine.
- UI must never wait for I/O, a full command queue, a worker, or a long-held lock.
- Windows x64 and Linux Ubuntu/Mint; macOS later.
- Navigation results carry a pane and generation; stale results never replace a newer location.
- Stage 1 is read-only. Copying with mandatory verification belongs to stage 2; no unverified copy shortcut.

## Review Focus

- An unavailable path must show an actionable error while the other pane remains usable: engine failure and pane-independence tests.
- Fast A/B/C navigation must discard old results and bound outstanding work: controlled-provider generation/cancellation tests.
- Invalid UTF-8 names must remain navigable without reconstructing paths from display text: Unix provider test and presenter identity test.
- A full result queue must not block the UI or lose terminal listing state: bounded-delivery and coalesced-wakeup tests.
- Sorting and selection must preserve item identity, including empty directories: core ordering and presenter identity/history tests.

## Workspace and execution

This directory has no Git repository. Work in the explicitly supplied project directory; do not initialize version control or manufacture commits/worktrees. Preserve execution evidence in `docs/superpowers/plans/2026-11-04-stage-1-progress.md` instead of Git-dependent skill scripts. The user explicitly authorized implementation, so continue after this plan without a second approval prompt.

## File map

- Root `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`, `.cargo/config.toml` and `scripts/cargo.ps1`: reproducible build and local toolchain helper.
- `crates/midnight-core/src/{lib,entry,protocol,provider}.rs`: domain contracts and sorting.
- `crates/midnight-platform/src/{lib,volumes}.rs`: local filesystem and OS volume adapters.
- `crates/midnight-engine/src/lib.rs`: async command loop, generations, bounded workers and streaming listings.
- `crates/midnight-ui/ui/{app,pane}.slint`, `crates/midnight-ui/src/{lib,presenter,model}.rs`: visual shell, pure presenter, lazy virtualized row model.
- `crates/midnight-explorer/src/main.rs`: runtime, bounded inbox and UI lifecycle.
- `.github/workflows/ci.yml`, `README.md`: Windows/Linux checks and run instructions.

### Task 1: Core contracts and build foundation

**Produces:** `PaneId::{Left,Right}`, `NavigationId(u64)`, `EntryId(u64)`, `Entry`, `EntryMetadata`, `EntryKind`, `SortSpec`, `SortColumn`, `Volume`, `SpaceInfo`, `FsError`, `Cancellation`, `Command`, `Event`, `FileSystem`.

**Provider interface:** synchronous `read_directory(&self, path: &Path, cancel: &Cancellation, emit: &mut dyn FnMut(Entry) -> bool) -> Result<(), FsError>`; `entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError>`; `list_volumes(&self) -> Result<Vec<Volume>, FsError>`; `space_info(&self, path: &Path) -> Result<SpaceInfo, FsError>`. These calls are only made by background workers.

- [x] Write ordering tests: folders stay first in both directions; unknown sizes remain last; ties are deterministic.
- [x] Run tests against unimplemented sorting and observe assertion failures.
- [x] Implement the domain contracts and `sort_entries(&mut [Entry], SortSpec)` with cached name keys.
- [x] Run `scripts/cargo.ps1 test -p midnight-core` and retain the result.

### Task 2: Local filesystem and volumes

**Consumes:** Task 1 provider contract. **Produces:** `LocalFileSystem::new(home: Option<PathBuf>)` implementing `FileSystem`.

- [x] Write real temporary-directory tests for files, folders, unknown initial metadata, metadata enrichment, cancellation and a missing path; Unix-only non-UTF-8 path test.
- [x] Observe failure before implementing the provider.
- [x] Enumerate names/types first without statting every file; expose metadata separately. Never follow special files during metadata or listing.
- [x] Windows roots use Win32 logical-drive enumeration; Linux mounts parse escaped `/proc/self/mountinfo` fields, filter internal filesystems and include `/` plus Home. Test parsing on any host.
- [x] Run `scripts/cargo.ps1 test -p midnight-platform`.

### Task 3: Async navigation engine

**Consumes:** Task 1 messages/provider. **Produces:** `Engine::new(Arc<dyn FileSystem>, mpsc::Sender<Event>)`, `Engine::run(mpsc::Receiver<Command>)`.

- [x] Add a controlled provider test proving a slow left pane does not prevent right-pane completion; add stale-generation cancellation and queue saturation tests.
- [x] Observe failures, then implement a bounded worker budget, cooperative cancellation and per-pane generation ownership.
- [x] Emit `ListingStarted`, bounded `EntriesBatch`, bounded `MetadataBatch`, final sorted `ListingFinished` with a shared immutable entry snapshot, or `ListingFailed`. UI must consume snapshots lazily.
- [x] Discover volumes independently; retrieve free-space information independently from directory completion, with generation checks. Stop queued work when superseded or shutting down.
- [x] Run `scripts/cargo.ps1 test -p midnight-engine`.

### Task 4: Slint panels and presenter

**Consumes:** Task 1 messages. **Produces:** public `MainWindow`, `Presenter::new(left: PathBuf, right: PathBuf)`, `Presenter::apply(Event)`, and a callback-based `install(window: &MainWindow, presenter: Rc<RefCell<Presenter>>, send: Rc<dyn Fn(Command) -> bool>)` plus UI refresh API.

- [x] Write presenter tests for ignoring stale generations, cursor/selection identity after sorting, independent panes, root navigation and back/forward branching.
- [x] Observe failures and implement the pure presenter without filesystem access. Explicit navigation errors preserve the last usable listing/history; successful navigation commits history.
- [x] Build two dense panels with independent volume selector/path entry, virtualized rows, name/size/date/type headers, loading/error state and status. Keep all model reads in memory.
- [x] Wire Tab, Enter, Backspace, arrows, Page Up/Down, Home/End, Space/Insert, refresh and history navigation. Sorting is a new engine request; old results are ignored.
- [x] Run presenter/model tests and compile `.slint` via `cargo check -p midnight-ui`.

### Task 5: Composition, smoke checks and delivery

**Consumes:** Tasks 2–4. **Produces:** runnable `midnight-explorer`, README and Windows/Linux CI.

- [x] Test bounded/coalesced wakeup behavior before implementing the bridge: multiple events produce at most one outstanding wake; draining cannot lose a wake during a producer race.
- [x] Wire an explicit multi-thread Tokio runtime; bounded command/result channels; UI work budget; cancellation and bounded shutdown. No unbounded callback forwarding.
- [x] Support optional left/right startup paths and a noninteractive `--smoke-test` that loads both real directories and reports results; UI smoke mode closes itself after real data arrives.
- [x] Add a Windows/Linux CI matrix with fmt, tests, clippy and build; document Linux prerequisites and Ubuntu/Mint manual validation separately from checks actually executed here.
- [x] Run `cargo fmt --check`, workspace tests, clippy and Windows application build. Run headless provider/engine smoke and native window smoke. Record platform limitations honestly.
- [x] Fresh independent review of the finished implementation, fix material findings and rerun affected checks.

## Deferred work

Copy/move/delete, checksum implementation, reparse-point traversal, thumbnails, shell icons, mount/unmount, filesystem watchers and macOS adapters are deliberately later stages. Per-pane remembered volume paths are included in the presenter; Salamander sources are behavioral reference only, without copying GPL code/assets.

### Task 6: Isolate operating-system implementations

User-requested architecture refinement, specified in design section 15.

**Files:** core provider traits; new `midnight-platform-common`, `midnight-platform-windows`, `midnight-platform-linux` crates; existing `midnight-platform` becomes facade; provider integration tests split by OS.

**Interfaces:** `DirectoryService` has existing `read_directory`/`entry_metadata` signatures; `VolumeService` has `list_volumes`; `StorageService` has `space_info`. `FileSystem` is their object-safe aggregate with a blanket implementation. OS adapters implement all three; `LocalFileSystem::new(Option<PathBuf>)` remains the facade API. `home_directory()` comes from the selected OS adapter, removing mixed OS environment logic from the application.

- [x] Split the service contract, update controlled test providers to implement it, preserve existing behavioral tests.
- [x] Move portable std filesystem/capacity code to common, Win32 and drive logic to Windows, mountinfo and Linux home logic to Linux. Only the facade selects by cfg. No new dependencies beyond existing workspace crates.
- [x] Split OS-specific tests into OS files and add a shared service-conformance test over the selected adapter.
- [x] Run Windows workspace tests/strict Clippy/build and Linux backend checks including the native Linux adapter crate.

### Task 7: Public Rust style and source size

**Files:** `CODE_STYLE.md`, `rustfmt.toml`, source-size checker and CI; UI tests split by responsibility/platform.

- [x] Document official Rust Style Guide and Rust API Guidelines, 100-column rustfmt target and 100,000-byte handwritten source-file maximum; generated/third-party/media/lock artifacts excluded.
- [x] Enforce the size limit in CI and run it over the current source tree.
- [x] Split large UI test groups and Windows-only path tests into separate files; keep generic code free of OS implementations.
- [x] Verify formatting, workspace tests, public interfaces and unchanged behavior after refactoring; review the platform boundary once more.
