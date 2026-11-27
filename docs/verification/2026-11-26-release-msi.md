# Windows installer (MSI) from a release ZIP

Recorded on 26 November 2026, Windows 11 x64, Python 3.15, with `scripts/package-msi.py`
([Windows installer](../release-build.md#windows-installer-msi)) at the commit that
added it.

## Input

The published 0.6.0 release ZIP, `midnight-explorer-0.6.0-x86_64-pc-windows-msvc.zip`
(245,891,753 bytes, SHA-256
`1daf874dc7ee07c9956d2fa5a153e9b4166141773b5600b3c92a53b40f222a45`, the archive of the
[0.6.0 release record](2026-11-26-release-0.6.0.md)), read only. Because the workspace
version is already 0.6.1, the script's functions were called directly without the
workspace-version check; every other step is the one `build` runs.

## Results

- 2,373 ZIP entries; 2,372 files in the MSI (all but `BUILD-INFO.json`) in 1,259
  folders and 1,260 components (one per folder and the Start menu shortcut).
- `makecab.exe` (LZX, 21-bit window) took 315 s for 533,029,568 bytes of files; the
  cabinet is 192,526,273 bytes and the MSI 193,335,296 bytes (SHA-256
  `1ed3118f66111b33ab8eb0f6ccda2fa8b26098767d2c11a9bbed5f561b35c499`), against
  245,891,753 bytes for the ZIP. Writing the database took 3.6 s, the built-in check
  53 s; the check passed (summary information, every table, per-user rules, installed
  paths, and each cabinet file against its ZIP entry after `expand.exe`).
- Read independently through the Windows Installer COM interface (read only): product
  0.6.0 by Tomas Jelinek with the fixed upgrade code, the two upgrade rows (older
  versions replaced, newer ones detected and refused by the launch condition), the
  launch conditions `VersionNT64`, `NOT ME_DOWNGRADE_DETECTED` and `NOT ALLUSERS`, the
  Start menu shortcut to `[#MainExecutable]` starting in `MEUSERPROFILE`, only the two
  property-setting custom actions (type 51), and summary information x64;1033,
  schema 500, word count 10 (compressed, no elevation).
- An administrative installation (`msiexec /a ... /qn TARGETDIR=<folder below _build>`),
  which extracts the files without installing or registering the product, ended with
  0. The extracted `Programs\Midnight Explorer (MSI)` folder held exactly the ZIP's files
  except `BUILD-INFO.json`, each byte-identical to its ZIP entry. A first attempt with a
  target folder whose path was already about 150 characters long failed with error 1304
  at a path longer than 260 characters, which led to the 160-character limit for payload paths (the
  longest 0.6.0 path has 140).

## Not verified

No per-user installation, upgrade, repair or uninstall was run, the Start menu shortcut
and the Add/Remove Programs entry were not seen, and the Windows SDK's ICE validation
was not run (REL-05).
