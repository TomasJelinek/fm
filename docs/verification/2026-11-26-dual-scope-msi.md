# Dual-purpose Windows installer (Just for me or For all users)

Recorded on 26 November 2026, Windows 11 x64, Python 3.15, with `scripts/package-msi.py`
([Windows installer](../RELEASE_BUILD.md#windows-installer-msi)) at the commit that made
the MSI dual-purpose. Nothing was installed: every check below reads the package, extracts
it administratively or costs it in a session that installs nothing.

## Input

A stand-in release ZIP of the workspace version 0.6.2 written by the test helpers of
`scripts/tests/test_package_msi.py` (10 files besides `BUILD-INFO.json`, including
`midnight-explorer.exe`, `midnight-elevated-listing.exe` and nested viewer notices),
below `_build/msi-dual`.

## Results

- `python -B -W error -m unittest scripts/tests/test_package_msi.py
  scripts/tests/test_package_release.py`: OK (7 skipped, the Linux-only cases).
- `python -B scripts/package-msi.py build --zip <zip>` wrote
  `midnight-explorer-0.6.2-x86_64-pc-windows-msvc.msi` after its built-in check, and
  `verify --msi <msi> --zip <zip>` reported "10 files match".
- An administrative installation (`msiexec /a <msi> /qn TARGETDIR=_build\msi-dual\admin`)
  ended with 0; the extracted `Midnight Explorer (MSI)` folder held exactly the ZIP's
  files except `BUILD-INFO.json`, byte for byte.
- Read through the Windows Installer COM interface: the dialog tables (ScopeDlg,
  MaintenanceDlg, ProgressDlg, CancelDlg, ExitDlg, UserExitDlg, FatalErrorDlg), their
  events and conditions as `plan` defines them, `ALLUSERS=2` and `MSIINSTALLPERUSER=1`.
- A session opened on the package (`Installer.OpenPackage`, machine state ignored, no
  UI) and costed (`AppSearch`, `CostInitialize`, `FileCost`, `SetUserInstallFolder`,
  `CostFinalize`): Windows Installer resolved the default as a per-user installation
  (`ALLUSERS` empty), `ProgramFiles64Folder` to `%LOCALAPPDATA%\Programs\`, `INSTALLDIR`
  to `%LOCALAPPDATA%\Programs\Midnight Explorer (MSI)\` (the 0.6.1 per-user MSI's folder),
  `ProgramMenuFolder` to the user's Start menu, and the dialog's all-users folder
  `[%ProgramW6432]\Midnight Explorer (MSI)\` to `C:\Program Files\Midnight Explorer (MSI)\`.
  Copies set to install per machine (`MSIINSTALLPERUSER` removed, or `ALLUSERS=1`) could
  not be opened as a session by this non-administrative account, so the per-machine
  costing was not observed.

## Not verified

The dialogs were not displayed; no installation in either scope, UAC prompt, upgrade of
an installed 0.6.1/0.6.2 per-user MSI, repair or uninstall was run; the Program Files
folder's permissions after installation, the all-users shortcut's start folder (the
literal `%USERPROFILE%`) and **Retry with authorization** from an all-users copy were not
seen; the Windows SDK's ICE validation was not run (REL-05).
