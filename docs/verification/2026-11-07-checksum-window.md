# Checksum window qualification

This report covers the modeless Slint presentation and its tests. Backend/native
stream, SHA and manifest qualification is in
[checksum backend](2026-11-07-checksum-backend.md). The installed release/native
Windows desktop pass and final Linux UI pass remain pending for this increment.

## Windows software-window evidence

The combined profile0 test run in `_build/goal/checksums/ui-combined-4.log` passed
405 UI library tests, including four checksum presenter tests. It then passed two
of three checksum input tests; the context-menu test incorrectly searched only
the main component's accessibility tree. The shared modeless harness now searches
active popup components before the root, as the existing file-action harness does.

`windows-input-green.log` records all three checksum-window tests passing against
the newly built test executable:

- Captured Unicode filename, streamed results, Stop, verified-manifest request,
  refusal to discard a live window when Release is rejected, Escape and main-host
  shutdown without resurrection.
- Results/Details/Save and reachable actions at 860x620 and320x200 across all nine
  themes and both input densities (108 layout/action combinations).
- Accessible selection, all four per-algorithm context copies and full-row copy,
  plus closing a pending context menu when the generation changes. Callbacks use
  the originally captured generation/row token.

The tests use real Slint components and software window adapters with dispatched
pointer/key events. They do not establish native OS-window/GPU behavior.
Screenshots in `_build/goal/checksums/ui/` were inspected. That inspection found a
header with default vertical centering; explicit top/left geometry corrected it.
The corrected table/header and context menu were inspected again. Algorithm
controls now use their required height instead of a fixed148-pixel allocation.

## Additional regressions and pending checks

`_build/goal/transfers/ui-checksum-root-profile0-red.log` records a further causal
regression: copying a full row with SHA-256 calculated but an uncalculated MD5
output selector produced no clipboard request. The whole-row branch and button
availability are corrected; the new regression passed in the combined435-test
Windows UI library run (`_build/goal/commands/windows-ui-final.log`). Single-hash
copy still refuses uncalculated algorithms. That run also passed all three
checksum input tests with the compact algorithm controls, leaving the results
and export controls reachable at the 320×200 minimum.

The shared modeless creation-failure helper was qualified in
`_build/goal/selection/ui-fixes-final.log`: both tests pass, proving failed native
adapter creation is not retried on every refresh, release retries wait250ms,
reentrant refresh does not panic, accepted release closes ownership, and host
shutdown stops retries. The earlier failing probe is retained separately.
The tests also found the drag poll timer taking a conflicting Presenter borrow;
its narrow retry-on-next-tick guard passed the same stress scenario.

Final combined Clippy, Linux UI tests, native desktop interaction, release build
and deployment are not yet claimed. No normal application preferences were changed
by these software tests. Generated artifacts and logs remain under `_build/`.

Independent review found that changing only the output filename in a lossy full
path could reconstruct a different parent containing U+FFFD. The destination
resolver now preserves an untouched native suggestion and accepts a filename
alone relative to its exact captured parent. An ambiguous full-path edit is
refused with an explanation; explicitly choosing another unambiguous absolute
path remains supported. Existing backend staging/no-replace checks remain in force.
The extracted original resolver failed two of six direct Windows regressions;
the fix passed all six on Windows and Linux, including raw UTF-16/Unix parent
names. Direct strict Clippy passed on both systems. These isolated checks are
recorded in `_build/goal/checksums/destination-*.log`; the next combined UI build
also includes this resolver through the production Save callback.
