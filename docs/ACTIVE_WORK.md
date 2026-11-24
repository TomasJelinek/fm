# Active work

This page is the current checkpoint: what main contains, what is being changed now,
and where the evidence for both is kept. What each feature area implements is in
[feature status](FEATURE_STATUS.md); open work and known limits are rows of the
[implementation backlog](IMPLEMENTATION_BACKLOG.md). Earlier dated logs of this page
remain in the Git history (`git log -p -- docs/ACTIVE_WORK.md`).

## Current checkpoint

Main of 25 November 2026, prepared as the [0.5.3 preview](releases/0.5.3.md): scrolling
glide that keeps the scrolling speed, Copy and Move keeping times and attributes by
default, Linux SMB sign-in, the glowing Moon, the UI performance fixes of the 24 November
review and the missing-glyph check. Before it, the [0.5.2 preview](releases/0.5.2.md) was
published on 24 November from `1341a7ff` (tag `v0.5.2`, with a signed `SHA256SUMS`;
[release record](verification/2026-11-24-release-0.5.2.md)) after the
[0.5.1 preview](releases/0.5.1.md): the video helpers' library permission granted at run time,
free space that follows the volume, Copy and Move preserving times and attributes, the
redesigned progress window, archives in the folder history and Hot Paths through
nested archives, Compare Directories' explained and clearable marks, Find persistence,
the alignment polish of every window, the code review fixes of 24 November and faster
acceptance and Rust tests ([acceptance time record](verification/2026-11-24-acceptance-time.md)).
The documents describe this source.

- **Acceptance.** The last whole-suite runs are the 0.5.2 release runs at `1341a7ff`:
  Windows 104 and Linux 104 of 117 passed; their 9 and 10 failures were scenarios not
  yet updated for that release's Find and Compare changes, plus Linux-only alignment
  flags, and all of them pass with the updates in the reruns of 24 November
  ([release record](verification/2026-11-24-release-0.5.2.md)). The area records list every run,
  each check's latest status, the open findings and the reruns still pending:
  [panels](developer/acceptance/panels.md), [transfers](developer/acceptance/transfers.md),
  [Find](developer/acceptance/find.md), [viewers](developer/acceptance/viewers.md),
  [Options and keyboard](developer/acceptance/options-tools.md),
  [tools](developer/acceptance/tools.md), [journeys](developer/acceptance/journeys.md),
  [UI polish](developer/acceptance/ui-polish.md) and the
  [Windows run](developer/acceptance/windows.md).
- **CI.** Hosted runs pass every job on Windows and Linux at `1341a7ff` (the 0.5.2 tag)
  and `a7b12855` ([release record](verification/2026-11-24-release-0.5.2.md#ci));
  the CI time record is at `2b901122` ([CI time record](verification/2026-11-21-ci-time.md)).
- **Not qualified.** Native and manual qualifications that automated runs cannot
  provide are listed under PLAT-02; macOS is PLAT-01.

## In flight

These changes are being made now. The documents named describe the current behaviour
until they land.

- **Acceptance:** reruns that the area records mark as pending, and the scenarios they
  list as not yet written.

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
