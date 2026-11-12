# Custom Commands separators — 12 November 2026

## Backend contract

`UserCommandInvocation::Separator` is an inert ordered configuration item. It keeps
its stable ID and name, participates in existing 128-item / 1 MiB configuration
budgets, and retains label/icon validation. It skips executable and working-directory
template requirements. `UserCommandProcessInvocation` remains Program/Terminal only;
there is no native separator plan or new process launch mode.

The strict settings DTO adds `mode: "separator"` without process fields. Existing
Program/Terminal records and schema behavior remain readable. Converting a command
to a separator removes that exact shortcut override in the accepted list edit. On
load, only stale bindings pointing to known separator IDs are pruned before final
preference validation; unrelated settings and overrides survive.

Both actor admission and direct preparation refuse separators before reserving a
worker job, capturing native directories/entries or launching any process. UI kind 2,
settings controls, toolbar rendering and shortcut registration are integrated. Their
source integration was reviewed separately for stable IDs, draft preservation, stale
token/configuration checks and preview refusal; no additional issue was found in
that review.

## Verification

Unless an explicit path is shown, logs are under
`_build/qa/command-separator-20261112/`.

| Evidence | Result |
| --- | --- |
| `red.log` | Shared compile transition: root shortcut registration referenced the new variant before its declaration. Completed the agreed enum/matches before rerunning. |
| `persistence-red.log` | Expected missing separator decoder failure. A second fixture used an already-reserved hot-path shortcut and was corrected to an unused chord before GREEN. |
| `settings-green.log` | **5 passed, zero failed**: existing persistence compatibility plus ordered separator round-trip, strict process-field refusal and stale-shortcut pruning. |
| `core-green.log` | **2 passed, zero failed**: inert validation still enforces name/ID/count/byte rules, and conversion removes only the affected shortcut. |
| `engine-green.log` | Initial attempt stopped before compilation because concurrent icon work required a shared lockfile update; no engine test result claimed by that log. |
| `engine-final.log` | **20 passed, zero failed** in the Linux engine Custom Commands runtime/actor suite, including both separator refusal regressions and existing cancellation/retirement tests. |
| `_build/qa/find-actions-20261112/linux-final/engine.log` | Earlier Linux engine all-target gate: **1329 passed, zero failed, 11 ignored** across 44 suites. The later separator-specific core, persistence and runtime gates above supply the new feature evidence. |
| `_build/qa/find-metadata-scroll-20261112/windows-affected.log` | **1844 passed, zero failed, 44 ignored** across 30 Windows suites. Includes both core separator tests, all five persistence tests, 934 UI library tests, all 17 Custom Commands settings UI tests and all three toolbar tests. |
| `_build/qa/find-metadata-scroll-20261112/windows-clippy.log` | Strict Clippy completed for the root runner's affected app/UI/native/thumbnail/viewer packages and engine with `--all-targets --no-deps -- -D warnings`. The log records two `vendor/femtovg` dependency warnings; this is not a warning-free dependency or whole-workspace claim. |
| Owned `git diff --check` | Passed. |

The engine tests prove separator preview/execution never enters the worker queue,
and direct preparation never calls a native capture spy for either intent. The
Linux engine build recorded three temporary dead-code warnings in the parallel,
then unwired icon-storage module; no separator warnings or errors occurred.

The completed Windows settings gate covers the separator form's retained mode
values, disabled preview/execution, stable command edits and ordering. The toolbar
gate verifies a narrow noninteractive separator between command buttons, stale-click
rejection and overflow. The earlier direct-run footer-drop fixture failure is
superseded by the final 17-of-17 settings result. Software-rendered settings and
toolbar evidence is retained under
`_build/qa/find-metadata-scroll-20261112/ui-render/`.

Protected deployment remains with the root task. The automated tests, strict
affected-package Clippy gate and software renders do not establish an installed
native desktop separator workflow; no such workflow is claimed here.
