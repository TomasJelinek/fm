# Active work

This page is the current checkpoint: what main contains, what is being changed now,
and where the evidence for both is kept. What each feature area implements is in
[feature status](FEATURE_STATUS.md); open work and known limits are rows of the
[implementation backlog](IMPLEMENTATION_BACKLOG.md). Earlier dated logs of this page
remain in the Git history (`git log -p -- docs/ACTIVE_WORK.md`).

## Current checkpoint

Main of 27 November 2026, after the [0.6.2 preview](releases/0.6.2.md), published that day
from `029cdfff` (tag `v0.6.2`, with a signed `SHA256SUMS`;
[release record](verification/2026-11-27-release-0.6.2.md)): drives that follow the
system's notifications, Retry with authorization from the user's own installation and
the dual-scope Windows MSI. The [0.6.1 preview](releases/0.6.1.md) was published on 26 November
from `3a83b533` (tag `v0.6.1`, with a signed `SHA256SUMS`;
[release record](verification/2026-11-26-release-0.6.1.md)): any number of viewer and
comparator windows, larger comparisons and images, faster viewer and archive opening
and the first Windows installer (MSI). The [0.6.0 preview](releases/0.6.0.md) was
published that day from `2c3fd113` (tag `v0.6.0`, with a signed `SHA256SUMS`;
[release record](verification/2026-11-26-release-0.6.0.md)): themes that follow the system's
appearance, Harvest Navigator as the default icon, the Linux GVfs drive list and release
builds without debug symbols. The [0.5.4 preview](releases/0.5.4.md) was published on 25 November from
`510d9abc` (tag `v0.5.4`, with a signed `SHA256SUMS`;
[release record](verification/2026-11-25-release-0.5.4.md)): the installation-securing fix
for junctions and symbolic links, the ordering fixes and the faster Find sorting below.
The [0.5.3 preview](releases/0.5.3.md) was published that day
from `c7ef1e5c` (tag `v0.5.3`, with a signed `SHA256SUMS`;
[release record](verification/2026-11-25-release-0.5.3.md)): scrolling glide that keeps the
scrolling speed, Copy and Move keeping times and attributes by default, Linux SMB sign-in,
the glowing Moon, the UI performance fixes of the 24 November review and the missing-glyph
check. After that tag came three ordering fixes (an HTML viewer's last events, a closed
viewer window's slot, an archive selection's queue slot), faster tests
([test speed pass](verification/2026-11-25-test-speed-pass.md)), removed unused code and the
user documents checked against the source. The [0.5.2 preview](releases/0.5.2.md) was
published on 24 November from `1341a7ff` (tag `v0.5.2`;
[release record](verification/2026-11-24-release-0.5.2.md)) after the
[0.5.1 preview](releases/0.5.1.md): the video helpers' library permission granted at run time,
free space that follows the volume, Copy and Move preserving times and attributes, the
redesigned progress window, archives in the folder history and Hot Paths through
nested archives, Compare Directories' explained and clearable marks, Find persistence,
the alignment polish of every window, the code review fixes of 24 November and faster
acceptance and Rust tests ([acceptance time record](verification/2026-11-24-acceptance-time.md)).
The documents describe this source.

- **Themes.** Options → Appearance remembers separate dark and light palettes and follows
  the host appearance. The defaults are Blue and Light. The theme picked last is previewed while
  Options is open and OK shows the choice matching the system. The [palette catalog](THEME_PROVENANCE.md)
  lists the designs and the [settings format](SETTINGS_FILES.md) describes how choices are saved.
- **Application icons.** Options → Appearance previews [three designs](../assets/branding/README.md#icon-shortlist).
  Harvest Navigator replaced the Moon as the default application icon after the 0.6.0
  release commit; settings that named the Moon open with Harvest Navigator. The evidence
  below predates that change, when the picker offered four designs.
  Windows acceptance run `20261125-092639` passed selection, Cancel, save, restart
  and Reset. A native Windows test checked the selected artwork for both title-bar
  and taskbar icons on first display and after switching. All 103 settings tests passed.
- **Acceptance.** The last whole-suite runs are the 0.6.2 release runs: Linux 116 of 119
  passed with no failure; Windows 113 passed, and its one failure, `tools_checksum_algorithms`
  waiting for the Checksums window under load, passed when run alone
  ([release record](verification/2026-11-27-release-0.6.2.md)). The area records list every run,
  each check's latest status, the open findings and the reruns still pending:
  [panels](developer/acceptance/panels.md), [transfers](developer/acceptance/transfers.md),
  [Find](developer/acceptance/find.md), [viewers](developer/acceptance/viewers.md),
  [Options and keyboard](developer/acceptance/options-tools.md),
  [tools](developer/acceptance/tools.md), [journeys](developer/acceptance/journeys.md),
  [UI polish](developer/acceptance/ui-polish.md) and the
  [Windows run](developer/acceptance/windows.md).
- **CI.** Hosted runs pass every job on Windows and Linux at `029cdfff` (the 0.6.2 tag)
  ([release record](verification/2026-11-27-release-0.6.2.md#ci));
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
