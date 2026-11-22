# Active work

This page is the current checkpoint: what main contains, what is being changed now,
and where the evidence for both is kept. What each feature area implements is in
[feature status](FEATURE_STATUS.md); open work and known limits are rows of the
[implementation backlog](IMPLEMENTATION_BACKLOG.md). Earlier dated logs of this page
remain in the Git history (`git log -p -- docs/ACTIVE_WORK.md`).

## Current checkpoint

Main of 22 November 2026 at `ad91634e`, with the documentation pass that made the
current documents true to the source. Since `5cc55222` it also carries link following
in the HTML and Markdown viewers, faster archive copy-out with a quick Cancel, the
first archive mount-limit fixes and the archive password-dialog fixes.

- **Acceptance.** The last runs of the whole suite are Windows `20261121-144422` and
  Linux `20261121-144246`, both at `3f6fff8a`. On 22 November the acceptance work
  packages reran their own scenarios on both systems. The area records list every run,
  each check's latest status, the open findings and the reruns still pending:
  [panels](developer/acceptance/panels.md), [transfers](developer/acceptance/transfers.md),
  [Find](developer/acceptance/find.md), [viewers](developer/acceptance/viewers.md),
  [Options and keyboard](developer/acceptance/options-tools.md),
  [tools](developer/acceptance/tools.md), [journeys](developer/acceptance/journeys.md),
  [UI polish](developer/acceptance/ui-polish.md) and the
  [Windows run](developer/acceptance/windows.md).
- **CI.** The last hosted run recorded as passing every job on Windows and Linux is at
  `2b901122` ([CI time record](verification/2026-11-21-ci-time.md)). No hosted run of
  this checkpoint is recorded yet.
- **Not qualified.** Native and manual qualifications that automated runs cannot
  provide are listed under PLAT-02; macOS is PLAT-01.

## In flight

These changes are being made now. The documents named describe the current behaviour
until they land.

- **Archive mount-limit prompt:** an offer to close an open archive when two are
  already open, beyond the refusal that asks for it ([archive formats](ARCHIVE_FORMATS.md),
  [archive API](developer/FILESYSTEM_API.md),
  [engine lifetime](developer/ENGINE_LIFETIME.md); backlog ARCH-01 to ARCH-09).
- **Security fixes without a backlog row:** the folder permission check of the HTML
  viewer's AppArmor installer, and the link commands of three engine test fixtures.
- **Acceptance:** reruns that the area records mark as pending, and the scenarios they
  list as not yet written.
- **Final builds and release:** the release builds and packages
  ([release build](RELEASE_BUILD.md)).

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
