Midnight Explorer 0.5.0 - Windows x64 preview

Extract the complete ZIP to a folder, then run me.exe.
Keep the viewers, filesystems and thumbnailers folders beside me.exe, including
the thumbnail helpers' adjacent runtime libraries and notices. Use a private folder in your user profile
or an administrator-managed installation: native plugins are refused if other
accounts can replace their files or installation directory.
Retry with authorization (UAC) for a folder you cannot open runs the included
midnight-elevated-listing.exe as Administrator. It is used only when an administrator
installs the complete folder where standard users cannot change it, for example
below C:\Program Files. From a folder in your user profile the retry is still
offered but refused, and the message states this requirement. Earlier packages'
me.exe used the included helper from such a
folder, where any program running as you could replace it: replace an earlier
extracted package with this one, or at least delete its
midnight-elevated-listing.exe.
Rust and Visual Studio are not needed to run the application.
Install Microsoft's Visual C++ x64 Redistributable if VCRUNTIME140.dll is missing:
https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist

Targets Windows 10 22H2 / Windows 11 x64; tested on Windows 11 25H2.
The executable is unsigned. No installer or automatic updater is included.
Default graphics use Direct3D 12. --software is a diagnostic fallback with known
bitmap-emoji limitations, not equivalent to the GPU renderer's Unicode coverage.

Example:
  me.exe --left "C:\Users" --right "C:\Windows"

Settings are stored in %LOCALAPPDATA%\Midnight Explorer\preferences.json.
See FEATURE_STATUS.md for implemented and pending functionality, and
SYSTEM_REQUIREMENTS.md, KEYBOARD_SHORTCUTS.md and SECURITY.md for details.
F1 opens keyboard help; Options allows changing shortcuts and appearance.
The complete distribution includes all six viewers (text/hex, source, Markdown,
image, PDF and Chromium HTML), native/network/archive filesystems and both
image/video thumbnailers. Keep their complete runtimes, sources and notices.
F3 selects the internal viewer for the captured file.
Alt+F3 forces basic text; Ctrl+F searches; Space advances to the next captured
panel file; Escape closes the viewer. Selection/copy is limited to the loaded page.
F4 opens the external editor configured in Options > Editors. Options > Plugins
lists installed packages; enabling/disabling a package takes effect after restart.

License: GNU General Public License, version 3 or (at your option) any later
version, with no warranty; see LICENSE.txt.
The plugins are licensed under the same terms.
Source of this release (its tagged commit; BUILD-INFO.json names the revision):
https://github.com/TomasJelinek/fm/tree/v0.5.0
Releases: https://github.com/TomasJelinek/fm/releases
Dependency notices and licenses are included alongside the executable.
