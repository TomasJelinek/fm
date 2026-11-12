# Inline expansion: obsolete tree observations

## Reproduction and cause

The reported `C:\Program Files\Git\bin` contains `bash.exe`, `git.exe`, and
`sh.exe`, with no subfolders. Inline branches contain folders only, so expansion
must confirm an empty branch and remove the disclosure control without an error.

A production Engine regression reproduced the exact message:
`Tree command queue is full or its view/generation changed.` The queue was not
full. A fast branch completion published a new projection before an automatic
Commit or Viewport command for the previously displayed projection arrived.
Those observations were rejected like explicit user actions. The UI could display
the rejection and retry it even though a newer publication was already pending.
A second regression reproduced the same lifecycle problem after admission, where
completion takes priority over commands already in the mailbox.

## Change

Only Commit/Viewport observations whose revision is older within the same pane,
navigation, and tree session are retired without an error. This happens at both
mailbox admission and model-worker dispatch. The mailbox keeps a copied stamp
beside its weak projection reference; checking it cannot accidentally acquire
and destroy the final large projection on the dispatcher. Retirement is awaited
on a worker, preserving bounded ownership. A new publication still requires its
own exact Commit. The eight-command capacity and all explicit-action identity
checks remain unchanged. No filesystem permission or plugin trust check changed.

## Verification (Windows, 11 November 2026)

Both regressions failed against the prior implementation before the fix:

- The files-only fixture produced the exact reported queue/generation error.
- The gated worker completion emitted an unwanted rejection for a queued old
  observation.

After the fix:

- Engine tree-controller unit tests: **6 passed** (0.10 s).
- Production tree-controller integration tests: **18 passed**, one opt-in test
  excluded from the default run (0.43 s).
- Opt-in integration using the protected native filesystem plugin and the actual
  `C:\Program Files\Git\bin`: **1 passed** (0.22 s). This test is read-only and
  also verifies current-view selection proceeds and an explicit stale expansion
  remains rejected. Reproduce with `MIDNIGHT_NATIVE_TEST_INSTALLATION`,
  `MIDNIGHT_TREE_TEST_PARENT`, and `MIDNIGHT_TREE_TEST_CHILD`, then run
  `cargo test -p midnight-engine --test tree_controller installed_native_files_only_expansion -- --ignored`.
- Installed Slint disclosure/bridge tests: **4 passed** (0.27 s combined), covering
  both panes, all three layouts, replaced rows, canceled presses, selection,
  navigation, and disclosure geometry.
- Strict Clippy for engine library and tree-controller integration passed.
- Workspace formatting and whitespace checks passed.
- Debug `me.exe` rebuilt successfully; no plugin API or package change was needed.

Independent review found no actionable issue in ownership, admission bounds,
stale-generation handling, or current-view progress. These are focused tests of
this fix, not a complete application or cross-platform test run.

## Fresh debug deployment

`D:\experiments\dev\FM\_build\debug\me.exe` (0.2.0, 94,733,312 bytes) has
SHA-256 `368d5d9f4ac6ad62da042170bdddd6804dc9c79087b07f88c2f41b2da2ef3a34`.
It is registered against the new protected runtime
`C:\Users\tjelinek\MidnightExplorer-20261111-tree-expansion`; existing private
installations are retained. Both backend and native software-rendered UI startup
checks passed from the D: executable with the two panels set to the reported Git
folder and its bin folder (exit 0, empty stderr). The startup screenshot was
visually inspected. Expansion itself was exercised through the real Engine and
installed native plugin, rather than claimed from this flat startup screenshot.
Local evidence is under the audit worktree's `_build/qa/tree-expansion/`.
