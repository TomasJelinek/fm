Midnight Explorer 0.6.1 - Windows x64 preview

Extract the complete ZIP to a folder, then run midnight-explorer.exe.
Keep the viewers, filesystems and thumbnailers folders beside midnight-explorer.exe,
including the thumbnail helpers' adjacent runtime libraries and notices. No script or
installer is needed; the application runs from any folder you extract it to.
Prefer a folder in your user profile, for example
%LOCALAPPDATA%\Programs\Midnight Explorer, or an administrator-managed
installation. A folder created directly below a drive root, such as C:\Tools,
lets every account on the PC change its files. The application still works there
and says so once: "Secure this folder" limits the folder to you, the system and
administrators (no administrator rights needed), and "Move to my profile..." copies
the complete installation into your profile, verifies it and offers to restart
from the copy. Moving the extracted folder yourself within the same drive keeps
its old permissions; extract or copy it instead. Video previews and playback run in
a sandbox that must be allowed to read the bundled FFmpeg libraries; a ZIP cannot
carry that permission, so the application adds it to those files itself in a folder
that only you can change (in a shared folder, secure it or move it first). Plugins are still refused when a
file in the folder belongs to another account or a link leads out of the folder.
Retry with authorization (UAC) for a folder you cannot open runs the included
midnight-elevated-listing.exe as Administrator. It is used only when an administrator
installs the complete folder where standard users cannot change it, for example
below C:\Program Files. From a folder in your user profile the retry is still
offered but refused, and the message states this requirement. Earlier packages'
application (then named me.exe) used the included helper from such a
folder, where any program running as you could replace it: replace an earlier
extracted package with this one, or at least delete its
midnight-elevated-listing.exe.
Rust and Visual Studio are not needed to run the application.
Install Microsoft's Visual C++ x64 Redistributable if VCRUNTIME140.dll is missing:
https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist

Targets Windows 10 22H2 / Windows 11 x64; tested on Windows 11 25H2.
The executable is unsigned. No installer is included. Check for updates (toolbar
and About) asks GitHub for a newer release, at startup too (at most once in 20
hours; Options > General > Updates turns it off). Download and install accepts only
an archive whose checksums are signed with the project's release key, installs it
on restart and restores this version if the new one does not start. It updates
only a folder you can change; for a folder that only administrators can change it
offers the release page instead. See SECURITY.md (Update check).
Default graphics use Direct3D 12. --software is a diagnostic fallback with known
bitmap-emoji limitations, not equivalent to the GPU renderer's Unicode coverage.

Example:
  midnight-explorer.exe --left "C:\Users" --right "C:\Windows"

Settings are stored in %LOCALAPPDATA%\Midnight Explorer\preferences.json.
Releases up to 0.5.0 named the executable me.exe. Settings stay where they were;
point shortcuts, pinned taskbar items, "Open with" choices and firewall rules
that named me.exe at midnight-explorer.exe instead.
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
https://github.com/TomasJelinek/fm/tree/v0.6.1
Releases: https://github.com/TomasJelinek/fm/releases
Dependency notices and licenses are included alongside the executable.
