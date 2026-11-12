# Panel feedback, Options and thumbnail cache — 2026-11-09

This pass covers full-width panel errors with Copy at bottom-left and Close at
top-right, localized physical bracket shortcuts, reversible Options reset,
filename-color priority selection, pending Network/native loading presentation,
configurable thumbnail size, resolution-aware reuse and private cache buckets.
No third-party dependency or plugin ABI changed.

## Behavior and review

- Options → Previews offers 48, 64, 72, 96, 128, 192 and 256 logical pixels;
  default 72 preserves the prior geometry. Both panes preview changes; Cancel
  reverts, OK persists and Reset stages defaults. Schema v25 stores nondefault
  thumbnail sizes; legacy settings retain the default.
- Larger requests upgrade individual frames. Larger cached results satisfy
  smaller requests, including intrinsically tiny sources. Visible fallback
  pixels stay available while upgrades run; offscreen images are evicted first.
- New cache records use `v1/ab/<hash>.mtc`. The 256 possible buckets are lazy;
  existing flat records remain readable. Byte/record budgets, Clear and eviction
  cover both layouts and count duplicate physical records. Windows/Linux storage
  retains private ownership, pinned parents and no-link/reparse rules.
- Reset preserves tabs, history, hot paths and custom commands, including an
  unfinished command draft. Network navigation immediately shows its requested
  location and loading state without exposing stale native operations.

Independent review found two recovery defects. A thumbnail upload rejected by the
UI pixel budget had already been marked complete by the scheduler. The bridge now
records retained coverage and bounded retry metadata; a changed viewport retries
from the validated cache using the same source token. Unchanged timer pulses do
not repeatedly upload or decode. The second defect could lock Options after an
accepted write failed while a newer local session write was rejected by the
queue. Tracking actually admitted revisions fixes that distinction. Both fixes
received a second review with no remaining actionable findings.

Visual review also caught the Fluent text button's padding clipping Copy inside
a 24-pixel square. The error banner now uses a centered square image control;
tests check glyph bounds, mouse activation, Enter/Space and accessibility action.

## Verification evidence

Logs and rendered snapshots are under `_build/qa/panel-options-polish/`.

| Area | Passing checks | Evidence |
| --- | --- | --- |
| Core preferences/contracts | 30 tests | `core-final.log` |
| UI presenter and retention | 626 tests | `ui-review-final.log` |
| Actual Options/thumbnail controls | 7 + 1 tests | `ui-review-final.log` |
| Error control layout and activation | 3 tests | `error-copy-final.log` |
| App thumbnail bridge, DPI and startup failure | 3 tests | `app-thumbnails-final.log` |
| Input mappings | 21 tests | `ui-final-3.log`, input target |
| Color picker/priority/color presentation | 3 + 3 + 2 tests | `ui-final-3.log`, corresponding integration targets |
| Command bindings/loading presentation | 2 + 1 tests | `ui-final-3.log`, corresponding integration targets |
| Thumbnail host, Windows | 41 tests | `windows-host-review-final.log` |
| Thumbnail host, Linux | 40 tests | `linux-host-review-final.log` |
| Private storage/worker platform, Windows/Linux | 24 / 23 tests | `windows-cache-native.log`, `linux-cache.log`, worker-platform targets |
| Settings serialization, Windows/Linux | 4 / 4 tests | `windows-settings.log`, `linux-settings.log` |
| Source size | 1487 files at most 100,000 bytes | `source-size-final.log` |

The earlier combined `ui-final-3.log` includes two subsequently corrected test
fixture failures; its successful individual targets above are the only results
cited from that run. The newer UI log supersedes the failed unit/control targets.
The app pressure fixture was corrected to use the public window navigation
callback before its successful run; no production API was exposed for testing.
The Options lockup, rejected thumbnail retry and clipped Copy regressions were
each observed failing before their production fixes (`options-review-red.log`,
`thumbnails-review-red.log`, `error-copy-red.log`).

Native Windows private-storage tests ran outside the enclosing sandbox so their
own ACL and AppContainer boundaries could be exercised. Linux tests ran in
Ubuntu 24.04 WSL with owned native fixtures under `/tmp/fm-interaction/_build/`;
using the NTFS workspace for Linux permission assertions would be misleading.

Software-rendered fixtures cover compact 320×200 layouts, full-width errors,
scrolling with fixed error controls, color priority and picker layouts, Reset,
thumbnail size changes and pending Network/native/refresh states. Screenshots
were inspected. This is not a claim of real SMB discovery acceptance or new
image/video decoder acceptance: the existing opt-in decoder/deployment/benchmark
tests remain ignored (7 Windows, 5 Linux). Unrelated dirty-tree features and a
full workspace test sweep are outside this scoped report.

## Final build and native startup

`cargo fmt --all -- --check` and targeted all-target Clippy passed. Clippy covered
core, platform-common, worker-platform, thumbnail-host, UI, UI-input and the app
with `--no-deps -- -D warnings`; pre-existing unused-code warnings in the engine
dependency remain outside that gate. This is not a clean whole-workspace Clippy
claim. The app thumbnail test filter passed all three tests.

`cargo build -p midnight-explorer --offline --locked -j2` completed successfully
with dev debug information and incremental compilation disabled. The prepared
local SDK registry override used in the tests was also used for this build.
Evidence: `fmt-final.log`, `clippy-final.log`, `build-final.log`.

Executable: `_build/debug/me.exe`, 86,270,464 bytes, SHA-256:

```text
70AAEB2367DF1B1953225514D8AAAD61CEA8A40416A267F8080BBDAB4DBE6381
```

The exact executable and 579 previously verified plugin/notice files were copied
to `C:\Users\tjelinek\MidnightExplorer-20261109-options-candidate`. Deployment
checked every installed hash, protected private ACLs, the prior frozen runtimes
and current/legacy preference files. It registered the executable's exact hash
under `%USERPROFILE%\.midnight-explorer\plugin-runtimes`; it did not relax plugin
trust checks or modify the checkout's permissions. No shortcut was changed.
Evidence: `runtime-deployment.json`, `runtime-plugin-inventory.json`,
`runtime-frozen.json`, `deploy-final.log` and the three runtime helper scripts.

Both the private executable and the matching `_build/debug/me.exe` passed the
bounded native startup smoke with exit code 0, empty stderr, no timeout and
unchanged user settings. The owned processes ran for about 20 seconds each
(16:55:24–16:56:03 UTC). They displayed the repository on the left and
`C:\Windows\System32` on the right. The 3015×1845 build-directory screenshot was
visually inspected: both listings, drive bars, command line and bottom buttons
rendered without a startup error. Smoke mode uses in-memory preferences, so this
does not claim acceptance of every existing user-profile setting.

Evidence: `native-candidate.json`, `native-debug-build.json`, their `.png`,
`.out`/`.err` and preference snapshots, and `smoke-final.log`.
