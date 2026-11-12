# Explicit authorization for one denied directory listing

A native folder navigation that returns PermissionDenied can offer **Retry with
authorization**. Clicking it asks
the operating system to authorize a separate, one-shot directory reader. The main
application remains unelevated. Background refresh, tree disclosure probes,
metadata reads and ordinary file operations never invoke this service.

The engine retains the denied path and sort specification. The UI sends only the
pane, failed navigation ID and next navigation ID. Each receipt is consumed once;
newer navigation invalidates it. A failed/cancelled retry preserves the existing
listing. Helper results reuse the normal bounded listing worker, cancellation,
snapshot and UI generation checks. The service returns available listing metadata;
it does not replace native providers, operation captures, file read services or
mutation permissions. Opening or modifying a listed file still uses ordinary
privileges and may report access denied.

## Dedicated process and channel

`midnight-elevated-listing` has one entry point: read a single native directory and
return basenames plus metadata. It loads no plugins, invokes no shell, edits no
ACLs and stores no credentials. Paths are sent inside the channel, never as shell
text or command arguments. Windows uses UTF-16LE code units; Linux uses raw Unix
filename bytes. The host rejects returned names that are not single basename
components, then joins them to its retained destination.

The protocol has an 8-byte version marker, length-prefixed frames, a 64 MiB maximum
frame, 250,000 entries, 256 KiB maximum path/name field and 16 KiB maximum error.
Exceeding a limit reports failure; it never publishes a truncated successful
listing. Metadata contains no file contents. Output errors are bounded and the
helper writes no privileged paths to stdout/stderr.

Windows uses `ShellExecuteExW` with the fixed `runas` verb and a random first-instance
local named pipe that rejects remote clients. Its ACL admits the owner, Administrators
and SYSTEM. The parent retains the returned process handle and verifies the pipe
client PID. The helper verifies the pipe server PID and retained process creation
time. A 256-bit nonce handshake precedes the request. Executable and ancestor
permission checks are repeated while Windows read handles prevent replacement.

Linux uses fixed `/usr/bin/pkexec --disable-internal-agent` and a random private
0700 Unix socket directory under root-owned sticky `/tmp`. SO_PEERCRED binds the
connection to the exact launched PID/root UID; the helper verifies the original
parent PID/UID, `/proc` creation time and PKEXEC_UID. The same nonce handshake
precedes the request. Authentication belongs to the desktop polkit agent; there
is no terminal password fallback or stored password.

Each platform permits at most two pending/live authorization helpers. Cancellation
closes the channel and the helper watchdog exits its dedicated process on parent
disconnect, death or the five-minute deadline, including if a native directory
read is blocked. Parent reservations survive cancellation until the actual child
exits; Linux reaps children with bounded worker threads. The OS authentication
prompt itself remains under OS control. No test automatically approves a UAC or
polkit prompt.

## Installation and review boundary

Build entry points and CI release builds produce the helper next to `me`. Windows
private deployment copies it into the same protected runtime; release packaging
requires the exact sibling helper, records its hash and preserves executable mode.
The host resolves the registered runtime and validates helper ownership/permissions
and all ancestors before launch, refusing symlink/reparse redirection. A standalone
copy without its helper reports an honest error. Same-user/root interference and
OS authorization-agent behavior remain outside the private-runtime trust boundary;
no universal filesystem race-freedom claim is made.

## Evidence

Nonprivileged Windows checks cover bounded/malformed frames, native UTF-16 identity,
invalid startup arguments, an actual nonce/process-checked named-pipe Unicode
listing, buffered reply closure, empty-read cancellation and helper-slot bounds.
Engine tests cover explicit single-use receipts, exact captured destinations,
complete metadata and the absence of prompts for background refresh. UI tests
cover backend-offer gating, stale offers and sending only receipt IDs.

Actual UAC/polkit consent and resulting privileged directory access require an
interactive OS session and are not claimed by the nonprivileged checks. Linux
transport tests and final combined UI qualification are recorded separately by
the current verification run.

## 12 November CSC warning-retention regression

The targeted Windows gate compiled with exit 0 and zero captured source drift.
Presenter elevation (9) and folder-presence (8) tests passed: **17 passed /
0 failed / 0 ignored**. The corrected source retains the exact committed row's
warning across retry admission, failed listing and Interrupted cancellation,
restores cursor/scroll on failed return, and rejects stale/reused-row diagnostics.
Matching current listing success retires the warning; action gates remain intact.

A separate Engine boundary gate compiled with exit 0 and no test-source drift,
then passed **5 tests / 0 failed / 0 ignored**. It covers mocked helper denial,
cancellation and transport in `crates/midnight-engine/tests/elevation.rs`.
The receipt is `_build/verification/build-20261112/csc-engine-elevation-runtime-result.json`
(12 November, 22:39:50 CET).

This qualifies mocked authorization and Engine-event retention, with no real
Windows UAC/CSC-access or fresh-EXE qualification. The overall batch remains
incomplete: `copy_dialog` recorded 6 passed / 2 failed Unicode assertions;
ViewerFit recorded 0 passed / 1 failed at its `content_width = 400` assertion.

The UI receipt is `_build/verification/build-20261112/csc-compact-green-runtime-result.json`
(12 November, 21:33:20 UTC), with
`csc-compact-green-runtime/ui-authorization-folder-presence/summary.json` and
`csc-compact-green-source-and-command.json` retaining counts and source/command identity.

References: [Microsoft least-privilege process guidance](https://learn.microsoft.com/en-us/windows/win32/secbp/running-with-administrator-privileges),
[polkit pkexec manual](https://polkit.pages.freedesktop.org/polkit/pkexec.1.html).
