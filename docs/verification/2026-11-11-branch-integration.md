# Branch and worktree integration audit, 11 November 2026

Baseline: `codex/viewer-plugins` at `d63ba30`; remote refs were read with
`git ls-remote --heads origin`. All four worktrees were clean, and all six local
branch tips matched their remote tips. Ignored build/toolchain output is excluded
from this source cleanliness statement. No branch or worktree was removed.

## Git inventory

| Worktree | Branch | Baseline tip |
| --- | --- | --- |
| `D:/experiments/dev/FM` | `codex/viewer-plugins` | `d63ba30` |
| `C:/Users/tjelinek/.codex/worktrees/main-deep-audit/FM` | `codex/main-deep-audit` | `dd5ad34` |
| `C:/Users/tjelinek/.codex/worktrees/midnight-archives/FM` | `codex/archive-filesystems` | `99e619c` |
| `C:/Users/tjelinek/.codex/worktrees/midnight-thumbnails/FM` | `codex/thumbnail-worktree` | `ea4b0d8` |

The other local branches are `main` (`a98b518`) and `feat/panel-prefix-search`
(`5295cd7`). Both `codex/main-deep-audit` and `feat/panel-prefix-search` are ancestors
of `main`. The viewer, archive and thumbnail branch tips are not ancestors of main.
That is an ancestry statement, not proof that each branch has missing features.

## Archive and thumbnail preservation branches

A parallel source/history audit found no missing delta to copy from these two
preserved branches into the current viewer branch. Their implementation was
incorporated through `60d2957` and subsequently changed in later commits. Keep
these branches as provenance; blindly cherry-picking their old snapshots would
risk restoring superseded resource and source-identity behavior.

Archive commits `4b07f3d`, `dd39642` and `99e619c` touch 99 paths; all exist in the
current tree. Of 156 original test names, 153 remain. The replacements preserve
changed-source capture/read/cleanup coverage, move ZIP CRC validation to the
first verified read, and explicitly accept high expansion ratios under streaming.
The latter is an intentional removal of the old browsing limit, not lost safety
coverage. Both sort rollback/unavailable saved-tab regression names remain in
`crates/midnight-ui/src/presenter/tests/archives.rs`. Worker probe/workspace files
are byte-identical where applicable; newer ResourcePolicy and SDK 0.2.0 contracts
supersede the preserved implementations.

Thumbnail commits `3ec6d66` and `ea4b0d8` touch 102 paths, all still present. Of
these, 36 match the current blobs and a further 40 match their integrated
`60d2957` versions; remaining changes are formatting or later feature/safety work.
Of 86 original test names, 85 remain. The fairness test was replaced by its
I/O-completion variant in `6921193`. Thumbnail callbacks/settings, independent
workers, cache and service paths remain connected. Current code adds typed
archive sources, size-aware cache coverage, retention and plugin catalog entries.

These are bounded source/history comparisons, not new runtime qualification or
proof of complete behavioral equivalence. Cooperative archive thumbnail staging
cancellation and archive tree/watch/nesting remain in the
[requirements audit](2026-11-11-open-requirements-audit.md); the old branches do not
supply those missing features.

## Main audit integration still required

At baseline, main has five commits not in viewer ancestry: four audit commits and
the merge commit. A non-mutating worktree merge preview (`git merge-tree`) reports
26 conflicted paths. It writes only Git objects; no merge was started. Most code
changes are in `962ebb5`, with the shell-pipe correction in `dd5ad34`.

Integrate remaining changes by behavior, preserving current provider identities,
input semantics and regression coverage:

- Bound desktop/property/folder-size event delivery and reap completed tasks;
  integrate controller cancellation without dropping live I/O permits early.
- Adapt lazy history models to current native, archive and network histories.
  The main model predates opaque provider identities/display strings; copying it
  verbatim would regress current navigation semantics.
- Retain applicable selection/mount deduplication improvements and responsive
  large-font status sizing. The old Meta double-click hunk is superseded by
  current press/release/move handling and does not need a duplicate patch.
- Preserve current asynchronous clipboard rejection delivery, while adding the
  missing controller-drop cancellation behavior with lifetime tests.
- Modernize settings size-boundary tests. Current production already checks a
  4 MiB metadata/read limit; old 64 KiB invalid-whitespace tests do not establish
  that boundary. Do not misreport an absent production bound.
- Add the explicit cross-filesystem qualification marker check without replacing
  current independent SDK/plugin CI jobs.
- Reconcile audit documents/inventories against current dependencies and features.

Do not copy main's `autotests = false` and old integration manifest as-is: its
29-module list would omit 85 of the current 113 top-level UI test files. Any test
binary consolidation must inventory and retain all existing tests, plus applicable
new regressions, and verify actual discovered test counts before/after.

## Completed integration: Windows shell regression harness

`crates/midnight-core/tests/desktop_windows.rs` now matches main's committed file.
It drains both child pipes concurrently with a 1 MiB capture limit per stream,
continues draining after overrun, retains the 15-second process deadline, and
batches PowerShell cases while checking a separate marker and sentinel per case.
No application runtime behavior, package version, dependencies or app executable
changed in this follow-up.

The new owned child writes 128 KiB to each pipe. With the old wait-before-read
helper, the test failed after 15.02 seconds. With main's helper, the entire Windows
shell integration target passes: six tests, two subprocess entry points ignored
as standalone tests but invoked by their parent tests, zero failures, 2.52 seconds
runtime. Compilation plus test command took 16.74 seconds. Strict target Clippy,
scoped rustfmt and whitespace checks pass. No larger test run is implied.

Evidence: `_build/qa/branch-integration-20261111/shell-pipe-{red,green,clippy}.log`.
The user-requested broad implementation/review goal remains active; the Git
inventory and this bounded integration checkpoint do not establish its completion.
