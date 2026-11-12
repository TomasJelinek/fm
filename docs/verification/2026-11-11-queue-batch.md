# Parallel queue batch integration, 11 November 2026

This batch integrates three independently implemented and cross-reviewed changes:

- [Copy/Move stage ownership and cleanup diagnostics](2026-11-11-transfer-stage-retry.md).
- [F4 from adopted inline trees](2026-11-11-tree-external-editor.md).
- [Typed Network Hot Paths](2026-11-11-network-hot-paths.md).

The earlier [function-key footer and view shortcuts](2026-11-11-function-bar-shortcuts.md)
checkpoint is included in the resulting application. This is a bounded completed
batch, not completion of the overall implementation/review goal.

## Windows verification

The pinned repository toolchain used locked offline dependencies and the normal
application feature graph. Logs, scripts and private test fixtures are under
`_build/qa/queue-batch-20261111`; no generated output was staged for Git.

| Final gate | Result | Test execution time |
| --- | --- | --- |
| Core library | 51 passed | 0.01 s |
| Engine library | 765 passed, 7 explicitly ignored | 6.66 s |
| Platform-common library | 85 passed | 0.48 s |
| UI library | 822 passed, 1 explicitly ignored | 3.27 s |
| Selected integrations | 31 passed, 1 explicitly ignored | 7.47 s across seven binaries |

The combined gates passed **1,754 tests**. The nine exclusions are existing opt-in
benchmarks, trusted installed-plugin fixtures, native-desktop ownership probing,
and the manual Recycle Bin fixture; they were not newly disabled. Focused RED and
GREEN results in the feature records overlap this total and are not added again.

Selected integrations include the actual Windows owned-file operation corpus:
both verification policies, Unicode/long paths, same-volume rename identity,
conflicts, retry versions, queued cancellation, directory merge, and self/hardlink
protection. Its binary passed 14 tests, with one opt-in case left ignored. Other
selected targets cover Find sort grouping, Network persistence/identity and actual
F4 input from each pane. These are controlled fixtures, not physical device
removal, disk exhaustion or a live SMB qualification.

Strict all-target Clippy with `-D warnings` passed for core, platform-common,
engine, UI and application. Five test-only lint findings were corrected without
suppressions, including existing operation/Find helper code. Two pre-existing
warnings remain in the vendored FemtoVG dependency; project targets passed the
strict gate. Rustfmt and the source-size check passed: 1,845 handwritten source
files, each at most 100,000 bytes. `git diff --check` also passed.

## Linux verification

Ubuntu 24.04 WSL used the existing pinned Linux toolchain and `_build/linux`
target. Fresh native fixtures used an isolated ext4 directory beneath
`/tmp/fm-interaction/_build`, while sources remained on the Windows-mounted drive.
The new transfer fixture honors TEMP/TMPDIR instead of forcing a DrvFs location.

**56 focused tests passed:** seven stage-retry cases, eight captured-editor
backend cases, three editor integration cases, nine Network/F4 integrations,
23 bookmark presenter cases and six tree-editor presenter cases. This includes
literal native paths, non-UTF-8 bookmark identities, cancellation/lifetime checks,
and real F4 dispatch in both panels. It does not qualify a Linux native desktop,
physical SMB/GVfs service or interactive authentication. External editor service
fixtures record dispatch; no new OS editor interoperability claim is made.

## Built application and startup

The debug entry point is `D:/experiments/dev/FM/_build/debug/me.exe`:

- Size: 96,190,464 bytes.
- SHA-256: `ae5544ac4cc47e86aeceda6f6b56d07a3826af58e099f80c4a0ec534c8c04c6e`.
- Matching protected runtime:
  `C:/Users/tjelinek/MidnightExplorer-20261111-queue-ae5544ac`.

Existing verified viewer/filesystem/thumbnailer packages were reused; this batch
changes no plugin ABI or independently packaged plugin. The matching executable
hash was registered before testing the original build entry point.

Backend startup, software-rendered UI startup and native-rendered UI startup all
exited zero with empty stderr. Both UI snapshots were visually inspected: panel
content, navigation and complete function-button captions render without a startup
error. The application was then launched normally, without smoke arguments or
replacement user preferences, and remained running as Midnight Explorer 0.2.

Raw evidence: `windows-libraries.log`, `windows-integration-final.log`,
`windows-clippy-final.log`, `format.log`, `source-size.log`, `linux-*.log`,
`build.log`, `deploy.log`, and `*startup.*` under this batch's QA directory.
