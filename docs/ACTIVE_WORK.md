# Active work

This page is the current checkpoint: what main contains, what is being changed now,
and where the evidence for both is kept. What each feature area implements is in
[feature status](FEATURE_STATUS.md); open work and known limits are rows of the
[implementation backlog](IMPLEMENTATION_BACKLOG.md). Earlier dated logs of this page
remain in the Git history (`git log -p -- docs/ACTIVE_WORK.md`).

## Current checkpoint

Main of 24 November 2026, after the [0.5.1 preview](releases/0.5.1.md), which was
published on 24 November from `c8c20f49` (tag `v0.5.1`, with a signed `SHA256SUMS`):
Compare Directories rewritten, the HTML viewer as a small browser, signed update
checks, shared-folder installations, the executable renamed to `midnight-explorer`,
and the fixes listed in the release notes. Since the tag, main has faster acceptance
and Rust tests ([acceptance time record](verification/2026-11-24-acceptance-time.md))
and a fix for PDF page steps pressed before the previous page was shown. The
documents describe this source.

- **Acceptance.** The last completed runs of the whole suite are the 0.5.0 release runs
  at `a70918e2` on 23 November: Windows 112 and Linux 113 of 116 passed, none failed
  (the rest skipped for hardware or platform reasons). Each 0.5.1 change reran its own
  scenarios, and the whole Windows suite ran again at `c8c20f49` for the acceptance time
  record. The area records list every run,
  each check's latest status, the open findings and the reruns still pending:
  [panels](developer/acceptance/panels.md), [transfers](developer/acceptance/transfers.md),
  [Find](developer/acceptance/find.md), [viewers](developer/acceptance/viewers.md),
  [Options and keyboard](developer/acceptance/options-tools.md),
  [tools](developer/acceptance/tools.md), [journeys](developer/acceptance/journeys.md),
  [UI polish](developer/acceptance/ui-polish.md) and the
  [Windows run](developer/acceptance/windows.md).
- **CI.** Hosted runs pass every job on Windows and Linux at `c8c20f49` and `c6fc88e7`
  ([documentation audit](verification/2026-11-24-docs-audit.md#ci-and-release-facts));
  the CI time record is at `2b901122` ([CI time record](verification/2026-11-21-ci-time.md)).
- **Not qualified.** Native and manual qualifications that automated runs cannot
  provide are listed under PLAT-02; macOS is PLAT-01.

## In flight

These changes are being made now. The documents named describe the current behaviour
until they land.

- **Acceptance:** reruns that the area records mark as pending, and the scenarios they
  list as not yet written.
- **Release validation:** whole-suite acceptance runs on Windows and Linux with the
  published 0.5.1 packages are not recorded yet ([release build](RELEASE_BUILD.md)).

## Where evidence lives

- **Dated records** in [verification/](verification/): one file per date and topic,
  each stating the commit and environment it describes. They are not updated later.
- **Acceptance records** in [developer/acceptance/](developer/acceptance/): per-check
  statuses with run ids, platform and scale; the suite and its options are described in
  [acceptance](developer/ACCEPTANCE.md). Run output stays in each machine's `_build`
  folder and is not part of the repository.
- **Commit messages** list the exact tests each change ran and on which system.
- **CI** runs [the workflow](../.github/workflows/ci.yml) on GitHub; a run counts only
  once it is recorded.
- A test, scenario or CI job that exists is not evidence that it passed.

## How work is organised

Roles, isolated exports, build slots, desktop and WSL leases and hand-ins follow the
[agent workflow](developer/AGENT_WORKFLOW.md) and [AGENTS.md](../AGENTS.md).
