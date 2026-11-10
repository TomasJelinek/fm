# Explicit Windows compatibility-junction navigation

Status: native platform, backend and UI activation checks passed. The Windows
debug application was built, deployed, registered and startup-tested; see the
[final artifact record](2026-11-10-settings-polish.md#exact-windows-artifact).

`C:\Documents and Settings` is a Windows compatibility junction to `C:\Users`.
On the qualification machine its ACL explicitly denies Everyone list-directory
access on the alias. The junction's own attributes and timestamp remain readable,
and its target can be enumerated through its native path with ordinary user rights.

The installed native filesystem plugin already returned `DirectoryLink`, native
attributes 9238 and modification time 1719494790.836441000 through both Browse and Stat.
Its DLL SHA-256 was
`21E82F4DEA4588E06D2B6F372DFE54AAE3527D454825A949F37B96C08FCA9EAC`.
No native plugin patch, dependency change or ACL modification to this OS junction
was needed. The `<DIR>` marker and localized date presentation are separately
covered by the shared UI work.

## Behavior and ownership

Explicit activation of a known native directory link uses
`Command::ResolveDirectoryLink` and the existing bounded desktop worker. The
Windows `OpenService::directory_destination` implementation requires an actual
directory. If ordinary alias enumeration succeeds, navigation retains the alias.
Only `ERROR_ACCESS_DENIED` on a directory link permits reading its OS-owned reparse
target with lossless `std::fs::read_link`. The target must be a supported absolute
disk/UNC path and must independently pass ordinary metadata and directory-open
checks. Relative symbolic-link targets are resolved against their actual parent.
Denied target access, unsupported namespaces, missing destinations and native
resolution failures remain errors. No privileges or permission changes are used.

Directory-only intent never calls a host file association, including when the
captured directory link was replaced with a regular file. Generic explicit file
and unknown-symlink activation keeps its existing association behavior.

`OpenDirectoryTarget` retains the original activation path separately from its
destination. Presenter acceptance requires the exact request, pane, navigation
generation and original path; invalid destinations and stale replies cannot
redirect a newer view. Directory links activated from Enter, double click,
Ctrl+PageDown, native tree navigation and fallback context-menu Open share this
directory-only path. Enumeration, recursive sizes and mutation traversal keep
their existing no-follow rules. The Linux default keeps the original navigation
path and invokes no association.

This remains ordinary path-based explicit navigation. It does not claim a pinned
directory identity across external replacement after the checks; mutation
preparation retains its stronger independent contracts.

## Evidence

Artifacts live under `_build/qa/junction-navigation/`, with scoped source
before-images in `baseline/`. Read-only `raw-probe.log` demonstrates that native
metadata and reparse-target reads succeed while alias listing fails with OS5.
`installed-probe.log` invokes the actual installed DLL's public Browse/Stat ABI.
The installed app measured before this source change had SHA-256
`24620126053B5909EB9191559BC05A6F96F262DD0433AFAD3188583DB20A2B91`.

Observed RED results are preserved in `ui-red.log` and `platform-red.log`: Enter
previously bypassed backend activation, and native activation returned the denied
alias instead of `C:\Users`.

Focused Windows session 89906 used `check-windows.ps1`, the existing approved
offline SDK configuration and the main `_build` target:

- `cargo test ... -p midnight-platform-windows --lib open::`: **11 passed**,
  including the opt-in actual `C:\Documents and Settings` test, owned Unicode
  junctions, alias-only denial, independently denied target access, restoration
  of fixture ACLs, missing targets, replacement with a regular file and existing
  long/raw UTF-16 activation checks. The actual OS junction is read-only; ACL
  changes apply only to fresh fixtures below `_build` and are restored by guards.
- `cargo test ... -p midnight-engine --test desktop`: **10 passed**, including
  original-path/target correlation and proof that directory-only intent never
  dispatches an association. Existing worker capacity and nonblocking checks pass.
- The initial focused UI attempt did not run because a shared settings component
  failed Slint compilation. After that correction, the combined Windows UI library
  gate passed every `presenter::tests::activation` case, including target navigation
  without focus theft, seven stale/identity/invalid-destination cases and failed
  follow-up queue admission. Parent evidence is
  `_build/qa/settings-polish/windows-ui.log` retains the initial 746 passes and
  one unrelated stale Ctrl+Shift+A expectation. The corrected final Windows UI
  library run passed 747 tests, and the focused activation rerun passed 14 cases,
  as recorded in the [final qualification](2026-11-10-settings-polish.md).

`git diff --check` passed the scoped source changes. The built and deployed debug
artifact shares SHA-256
`E71ED919E0B0EAE14DEC5B3062213F9DA6978F287206AC849CB20FA0BCAC6E11`;
backend and native Windows UI startup checks passed with empty stderr and the
runtime registration was refreshed. Tested paths, snapshot and exact validation
scope are in the [artifact record](2026-11-10-settings-polish.md#exact-windows-artifact).
This is a debug deployment, not a published release or completion of the broader goal.

## References

The read-only source comparison used Salamander's
`D:/experiments/salamander-main/src/fileswn2.cpp:336` denied-junction navigation and
`salamdr2.cpp:1405` native reparse-target helper. The new implementation uses the
existing Rust API and does not copy Salamander's ANSI or fixed-length parser.

Microsoft documents the compatibility-junction ACL behavior in
[Junction Points](https://learn.microsoft.com/en-us/windows/win32/vss/junction-points).
The native operation contracts are described in
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
and
[FSCTL_GET_REPARSE_POINT](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_get_reparse_point).
