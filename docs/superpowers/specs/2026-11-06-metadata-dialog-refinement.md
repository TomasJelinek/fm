# Metadata dialog refinement

The attributes dialog distinguishes the editable draft from the last backend
preview. Opening the case action, editing any field, or awaiting a new preview
shows an explicit not-previewed/stale state. Only a matching completed preview
can describe requested changes or authorize Apply. Late revisions remain ignored.

Attribute rows expose mutually exclusive Leave, Set and Clear buttons;
compression uses Leave, Compress and Uncompress. The timestamp section uses a
per-field enable checkbox, an explicit-offset date/time entry and a Now button.
Now remains a typed backend request resolved once for all selected fields and
files. Unchecked fields preserve their native values. Offset-free ambiguous
local timestamps remain invalid, including daylight-saving transitions.

The bottom pane is a virtualized file list with current and proposed names and
current/proposed metadata. Refresh and Preview changes both validate the current
draft and re-read metadata from the same pinned native targets. All selected
roots fit the existing 256-target backend budget. Counts explicitly distinguish
selected roots from recursive descendants, which are discovered during execution.
Native paths remain identities; Unicode display strings never reconstruct them.
The UI model reads visible rows from the presenter without retaining another
snapshot owner. Replaced and closed snapshots enter the existing backend
reclamation queue. A failed native refresh reports the identity/read error and
unavailable current values, rather than relabelling a cached snapshot as fresh.

The Salamander reference was inspected only for interaction concepts: grouped
attributes, independently enabled timestamp rows, a current-time action and a
subdirectories option. No source code or visual assets are copied. No dependency
or feature changes are required.

Validation covers actual Slint row-button exclusion and timestamp toggles,
unpreviewed/stale status, exact-revision Apply, refreshed native timestamps, all
256 rows, Unicode case display, cancellation/conflicts and large-font layout.

## Verification recorded before the requested build-directory reset

On 2026-11-06, Windows `midnight-ui --lib` passed all 327 tests and the actual
`metadata_editor` integration test passed. That integration exercised exclusive
attribute buttons (including clicking the selected button again), timestamp
enable/Now actions, Refresh, exact Apply gating, virtualization of 256 rows,
headers above rows, desktop/touch densities, fonts 11 and 32, and a long Unicode
error using the card width. Rendered attribute/time/error layouts were inspected.

Windows `midnight-engine --lib metadata` passed 15 matching tests after the final
backend projection change, including fresh metadata, all 256 roots, preserved
Unicode names, replacement rejection and preview-without-mutation. The Linux
metadata filter passed 15 tests before the final proposed-value projection field;
the final Linux run remains part of the root task's verification.

Red tests had first reproduced the false no-changes message, blocked empty-draft
Refresh, the old 32-row cap, stale timestamps, and replacement-target acceptance.
The actual new controls' initial red build was interrupted by disk exhaustion;
their completed runtime checks passed after implementation. The user subsequently
requested removal of the entire `_build` directory, including screenshots/logs.
No artifact copy was retained elsewhere; these notes record the observed results.

## Verification after the clean rebuild

The actual metadata-editor integration test passed again on Windows after the
clean rebuild and subsequent shared UI integration. Linux engine metadata tests
passed 15/15 on the final proposed-value projection and identity-refresh behavior.
The final Linux UI library passed 357/357 and all 28 requested rendered interaction
tests passed, including the metadata editor. Linux attributes and long-error
screenshots at font size 32 were inspected; buttons, bounded scrolling and the
file-list headers remain readable.
