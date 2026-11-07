# Redirected Windows Desktop shortcut

The old deployment shortcut block used the executable/package validator on an
existing `.lnk`. The actual OneDrive Desktop has `IO_REPARSE_TAG_CLOUD_6`
(`0x9000601a`), so the validator rejected it as a symlink/junction. The read-only
regression failed before the change; evidence is `_build/goal/desktop/red.log`.

`scripts/windows-shortcut.ps1` now has a separate shortcut namespace policy;
`scripts/windows-shortcut-native.cs` holds its Windows calls. Only the documented
CLOUD/CLOUD_1..F tags are permitted in the Desktop namespace. Directory links,
junctions and unknown tags are refused. Executable/plugin/runtime-registration
policies still reject all reparses; their existing validation functions did not
change.

The helper validates private runtime owner/DACL and exact caller-supplied EXE
SHA-256, retains a no-follow read handle, and creates the `.lnk` inside the private
installation through Unicode `IShellLinkW`/`IPersistFile`. Shell automation's
TargetPath setter rejected the Unicode installation fixture, so it is no longer
used for creation. No Shell save targets the Desktop path. Native directory
handles retain the namespace throughout staging and publication. Ancestors deny
write/delete sharing. The destination leaf permits write sharing because the
kernel needs to open that directory during rename, but denies delete sharing.
Publication uses the retained leaf handle and a source handle with DELETE access.
If the leaf is retagged to a junction, the tested native rename fails closed;
it does not write to the junction's target.

`NtSetInformationFile(FileRenameInformation)` publishes one directory-entry
replacement relative to the retained destination handle. No-replace is also
tested. This is replace-current-name semantics for this explicitly chosen
shortcut, not identity-conditional replacement for the file-copy engine. No old
shortcut is truncated before publication. Cross-volume requests fail explicitly;
there is no copy/delete fallback or promise of power-loss durability.

The API ownership/layout/rename rules were checked against Microsoft's
[FILE_RENAME_INFORMATION](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information)
and [NtSetInformationFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile)
contracts. No-follow opens and handle sharing follow
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew);
the tag allowlist follows
[Reparse Point Tags](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-tags).
Unicode COM uses [IShellLinkW](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ishelllinkw)
and [IPersistFile::Save](https://learn.microsoft.com/en-us/windows/win32/api/objidl/nf-objidl-ipersistfile-save).
These are Windows-specific contracts; no Linux/macOS implementation is implied.

Validation on Windows 11 x64, PowerShell 7.6.5 and Windows PowerShell 5.1:

- Real Unicode installation/Desktop fixture create and update, COM target/icon
  readback, empty arguments, runtime SHA-256 validation and staging cleanup.
- Wrong hash, unsafe runtime ACL and runtime junction refusal.
- Desktop ancestor/final-name junction refusal with outside sentinel unchanged.
- Directory rename refused while namespace pins are live; native atomic
  no-replace preserves both source and destination.
- A hard link inserted at the final destination after validation is replaced as
  a directory entry; its outside alias remains unchanged.
- Direct FSCTL junction retag of the retained leaf fails closed without changing
  the outside sentinel; actual distinct C/D volumes reject cross-volume moves.
- All sixteen cloud tags accepted, name-surrogate and unknown tags refused;
  actual OneDrive Desktop CLOUD_6 read validation passes.
- Existing private-deployment and runtime-registration suites pass on both
  PowerShell runtimes. No Cargo run was required for this script-only change.

A file-symlink creation test could not run because the test token lacks the
required Windows privilege. The actual junction, hard-link and retagging tests
above ran natively; this document does not claim an unperformed symlink test.

The existing user's Desktop shortcut was updated to
`C:\Users\tjelinek\MidnightExplorer-20261107-120804-5c2b92b9\me.exe`, SHA-256
`559ce0249758e6e2e2d961b0efa4a2521b2c8ab7d083377faa0539098e45d1fb`.
COM readback verified target, icon, working directory and empty arguments. The
application was not launched and preferences were not written. A previous-link
backup and before/after evidence are under `_build/goal/desktop/`.

To update an existing private installation without redeploying it:

```powershell
./scripts/update-desktop-shortcut.ps1 -InstallationDirectory 'C:\path\to\private-runtime' `
    -ExpectedExecutableSha256 '<verified 64-digit SHA-256>'
```

This command updates only `Midnight Explorer.lnk` in the resolved Desktop. It
fails if the Desktop cannot be resolved or safely pinned, or is on another
volume than the private installation. Runtime trust is never repaired silently.
