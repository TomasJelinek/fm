# Commander Controls Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans task by task. Preserve independent ownership and verification evidence.

**Goal:** Add commander controls, external terminal input, parent row, drive bars,
native properties/icons and native scrolling behavior to the read-only app.

**Architecture:** Extend core with separate desktop service traits and messages.
Native per-OS adapters do blocking integration; engine owns bounded workers,
type-icon cache and process reaping. UI owns only presentation and input.

**Tech Stack:** Existing Rust1.99.1/Slint1.18.1/Tokio workspace.

**Spec:** `docs/superpowers/specs/2026-11-04-commander-controls-design.md`.

## Global constraints

Unicode/raw identities; no MAX_PATH; no mixed OS implementation files; no UI I/O;
100-column rustfmt/UTF-8/LF; handwritten source <=100000 bytes; read-only internal
file actions; explicit terminal submission only; relevant Salamander source audit.

## Review focus

Editor focus and literal quoting must survive metadata refresh; parent row must
not become a real selected ID; AltEnter must not navigate; native handle lifetime
and resource budgets must hold on failure; slow desktop work must not block panes.

### Task 1: Core desktop contracts and literal serialization

- [x] Add desktop types/traits, request IDs, context/icon/action messages; preserve
      existing FileSystem and public engine constructors.
- [x] RED→GREEN tests for PowerShell/Posix quoting, smart apostrophes, Unicode,
      invalid native Unicode and bounded RGBA shape validation.

### Task 2: Native desktop services

- [x] Windows isolated modules for icons, COM/GDI RAII, known folders, PowerShell
      new-console launch, native properties; synthetic icon names avoid path limits.
- [x] Linux isolated modules for XDG user dirs/theme PNGs, safe terminal argv,
      native properties where available, otherwise metadata fallback signal.
- [x] Tests for process arguments/literal paths, config parsing, icon limits,
      failures/cleanup and generic fallback, no automated arbitrary shell execution.

### Task 3: Engine desktop orchestration

- [x] Optional `.configure_desktop()?.install(Arc<dyn DesktopServices>)`; independent budgets for
      properties, terminal launches, locations and icon requests.
- [x] Bounded icon queue/cache, background pixels and progressive delivery; icon
      requests never delay listing. Bound retained terminal children and reap them.
- [x] Tests for async isolation, fallback/terminal errors and frozen request data,
      capacity bounds, shutdown and no terminal execution on context/icon requests.

### Task 4: UI controls

- [x] Parent cursor/row mapping with selection/sort/rollback tests.
- [x] Command editor, global bracket insertion, submit, F1–F10/menu/help and
      disabled unsupported actions, AltEnter with real-item targeting.
- [x] Per-pane location bars, Windows metadata-driven Shift-letter shortcuts,
      cached host images, virtualized scrolling and scrollbar/keyboard behavior.
- [x] Actual Slint integration tests for editor caret/focus, no disabled dispatch,
      parent navigation, per-pane location/drive keys and continued wheel movement.

### Task 5: Compose, verify and show

- [x] Inject desktop services in app, update docs/Salamander trace.
- [x] Windows/Linux tests/format/Clippy/source-size and native screenshot/smoke.
- [x] Fresh review and fix material findings; rebuild and relaunch under actual
      user account (the sandbox's visible HWND is not the user's desktop).

No Git repository exists; keep evidence in `.superpowers/sdd/commander-controls`
and progress in the companion progress document instead of Git-dependent scripts.

### Task 6: User-added automatic filesystem refresh

- [x] Native WatchService per OS, Unicode/long-path nonrecursive notifications,
      error/rescan hints, access-event filtering, guard cleanup and real mutations.
- [x] Backend 1s quiet/3s maximum debounce, bounded latest target/dirty hints and
      15s exact-metadata polling fallback; independent pane budgets and shutdown.
- [x] UI-accepted snapshot acknowledgements with bounded-queue retry, preserving
      rollback identity even when an intermediate completed listing became stale.
- [x] Same-location staged refresh preserves selection/cursor/scroll/editor;
      tests for stale hints, ignored/error retries, bursts, continuous writes,
      notification failure, missing events, Unicode and nonblocking navigation.
- [x] Repeat final platform verification and review after these authorized changes.

### Task 7: User-added appearance and persistent preferences

- [x] Remove the top branding row; Options beside Run/menu with Blue/Dark/Light
      and Desktop/Touch modes. Semantic palette/metrics and44px touch hit targets.
- [x] Native SettingsService per OS, strict versioned bounded codec, no special
      file/symlink reads, atomic exclusive-temp publication and private Linux files.
- [x] Serial latest-wins settings worker, revisions, late-load protection, retries,
      bounded graceful close after the window ends. No configuration I/O on UI.
- [x] Real theme/touch render tests, all6 combinations, errors/races/FIFO/long-path
      native storage tests, fresh review and platform verification.

### Task 8: User-added fair, storage-aware folder sizing

- [x] Conservative native storage hints: only proven local SSD gets parallelism;
      HDD/network/unknown stays one actual I/O stream, including across panes.
- [x] Bounded root worker batch, shared weighted I/O budget, separate classification
      capacity,100ms/256-entry fair yields, cancellation and bounded progress.
- [x] Blocked large SSD versus small result, nested paths, max concurrency,
      network/HDD/unknown serialization, two-pane sharing and cleanup tests.
- [x] Update main design status, verify all changed platforms and relaunch Windows.
