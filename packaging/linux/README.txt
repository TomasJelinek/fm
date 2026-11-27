Midnight Explorer 0.6.3 - Linux x86-64 preview

Extract the archive with tar, then run ./midnight-explorer in a graphical desktop
session. Keep the viewers, filesystems and thumbnailers folders beside
midnight-explorer. The thumbnailers
folder includes both image/video helpers and their required runtime and notices.
Prefer a folder that only you (or root) can change. If other accounts can write
to the extracted folder (group/other-writable modes, or a Windows drive mounted with
permissive drvfs modes), the application still works and says so once: "Secure
this folder" removes the group and other write permissions from the files you own,
and "Move to my profile..." copies the complete installation to
$XDG_DATA_HOME/midnight-explorer/app (by default ~/.local/share/midnight-explorer/app),
verifies it and offers to restart from the copy. Plugins are still refused when a
file belongs to another account or a symbolic link leads out of the folder.
Built on Ubuntu 24.04 / glibc 2.39. Requires GNU/glibc, not Alpine/musl.
Rust is not needed to run the application.

Ubuntu 24.04 / Mint 22 runtime packages:
  sudo apt install libfontconfig1 libfreetype6 libxkbcommon0 libxkbcommon-x11-0 \
    libx11-6 libxcursor1 libxi6 libxrandr2 libxcb1 libwayland-client0 libudev1 \
    libvulkan1 xdg-utils xterm
Use the graphics driver and Vulkan implementation supplied for your hardware.
Ubuntu 23.10+ / Mint 22 deny Chromium's sandbox the user namespaces it needs. When
the HTML viewer reports this, choose Install... in its notice: once an administrator
authorizes it, it installs an AppArmor profile that lets this installation's HTML
viewer use them (see SYSTEM_REQUIREMENTS.md). Never run a script from a folder that you
own, such as this extracted one, with sudo: any program you run could have replaced
it. To install the profile by hand, install system-wide first (below) and run the copy
that only root can change. The permission serves one installation at a time:
installing it for another installation moves it there, and the HTML viewer of the
previous one stops working until it is installed for that one again.
The HTML viewer's Chromium (CEF) runtime also loads common desktop libraries
(NSS/NSPR, ALSA, ATK/AT-SPI, CUPS, D-Bus, GBM/DRM, Expat, GLib, Pango/Cairo, X11/XCB
and X11 extensions). When one is missing, the HTML viewer names the missing libraries
and the command that installs them. Minimal systems may additionally need:
  sudo apt install libnss3 libnspr4 libasound2t64 libatk1.0-0t64 libatk-bridge2.0-0t64 \
    libatspi2.0-0t64 libcups2t64 libdbus-1-3 libdrm2 libexpat1 libgbm1 libglib2.0-0t64 \
    libpango-1.0-0 libcairo2 libxcomposite1 libxdamage1 libxext6 libxfixes3

Local network discovery and mounted SMB share browsing additionally need:
  sudo apt install gvfs-backends gvfs-fuse wsdd
The wsdd helper discovers Windows computers. Allow time for discovery, then
refresh Network Neighborhood. A known server can be entered as smb://server/.

Example:
  ./midnight-explorer --left "$HOME" --right /tmp

X11/XWayland supports native external file drag-and-drop. A native Wayland window
drags between its own panels only, unless MIDNIGHT_WAYLAND_DND=1 enables its native
transport, which has not been qualified on a compositor yet. --software is a
diagnostic fallback with known bitmap-emoji limitations. System fonts must cover the
displayed scripts.
Fedora 44+ and openSUSE Leap 16/Tumbleweed are compatibility targets, but this
preview has not been qualified on their native desktops. See SYSTEM_REQUIREMENTS.md.

Optional per-user desktop installation from the extracted folder:
  install -Dm755 midnight-explorer "$HOME/.local/bin/midnight-explorer"
  mkdir -p "$HOME/.local/bin/viewers" "$HOME/.local/bin/filesystems" "$HOME/.local/bin/thumbnailers"
  cp -R viewers/. "$HOME/.local/bin/viewers/"
  cp -R filesystems/. "$HOME/.local/bin/filesystems/"
  cp -R thumbnailers/. "$HOME/.local/bin/thumbnailers/"
  install -Dm644 io.github.TomasJelinek.MidnightExplorer.desktop \
    "$HOME/.local/share/applications/io.github.TomasJelinek.MidnightExplorer.desktop"
  install -Dm644 midnight-explorer.png \
    "$HOME/.local/share/icons/hicolor/256x256/apps/io.github.TomasJelinek.MidnightExplorer.png"
Ensure "$HOME/.local/bin" is on PATH when using the desktop launcher. On Ubuntu
23.10+ / Mint 22, that copy's HTML viewer needs the sandbox permission for itself: use
Install... in its notice. Any program you run can replace files below your home folder,
and with the permission a program placed at the viewer's path there can use user
namespaces too, so prefer the system-wide installation below on shared computers.

Retry with authorization for a folder you cannot open runs the included
midnight-elevated-listing as root through pkexec and the desktop's polkit agent.
It is used only from an installation that only root can change (root-owned, not
group- or world-writable, not on a nosuid mount). In the per-user installation above
or the extracted folder, the message for a denied folder offers no retry and
states this requirement. Earlier packages' application (then named me) used the
included helper
from a folder you own, where any program running as you could replace it: replace
an earlier extracted package with this one, or at least delete its
midnight-elevated-listing. System-wide installation from the extracted folder:
  sudo install -d -m 755 /opt/midnight-explorer
  sudo cp -R . /opt/midnight-explorer/
Then run /opt/midnight-explorer/midnight-explorer. For the desktop launcher, install the desktop
entry and icon as above and put /opt/midnight-explorer on PATH instead. On Ubuntu
23.10+ / Mint 22, use Install... in the HTML viewer's notice, or run the root-owned copy
of the script (this moves the sandbox permission to this installation):
  sudo /opt/midnight-explorer/install-html-viewer-apparmor.sh

Settings: $XDG_CONFIG_HOME/midnight-explorer/preferences.json, normally
$HOME/.config/midnight-explorer/preferences.json.

Check for updates (toolbar and About) asks GitHub for a newer release, at startup
too (at most once in 20 hours; Options > General > Updates turns it off). Download
and install accepts only an archive whose checksums are signed with the project's
release key, installs it on restart and restores this version if the new one does
not start. It updates only a folder you can change, such as the extracted folder;
for a folder that only root can change it offers the release page instead. See
SECURITY.md (Update check).

Releases up to 0.5.0 named the executable me. Settings stay where they were;
install the desktop entry again as above and delete an earlier per-user copy,
such as "$HOME/.local/bin/me".
See FEATURE_STATUS.md, KEYBOARD_SHORTCUTS.md and SECURITY.md for capabilities/limits.
The complete distribution includes all six viewers (text/hex, source, Markdown,
image, PDF and Chromium HTML), native/network/archive filesystems and both
image/video thumbnailers. Keep their complete runtimes, sources and notices.
F3 selects the internal viewer for the captured file.
Alt+F3 forces basic text; Ctrl+F searches; Space advances to the next captured
panel file; Escape closes the viewer. Selection/copy is limited to the loaded page.
F4 opens the external editor configured in Options > Editors. Options > Plugins
lists installed packages; enabling/disabling a package takes effect after restart.
License: GNU General Public License, version 3 or (at your option) any later
version, with no warranty; see LICENSE.
The plugins are licensed under the same terms.
Source of this release (its tagged commit; BUILD-INFO.json names the revision):
https://github.com/TomasJelinek/fm/tree/v0.6.3
Releases: https://github.com/TomasJelinek/fm/releases
Dependency notices and licenses are included alongside the executable.
