Midnight Explorer 0.3.0 - Linux x86-64 preview

Extract the archive with tar, then run ./me in a graphical desktop session.
Keep the viewers, filesystems and thumbnailers folders beside me. The thumbnailers
folder includes both image/video helpers and their required runtime and notices.
Native plugins require a private user-owned
or administrator-managed installation, with no group/other-writable package or
replaceable parent directory. Native Linux filesystems are recommended; a shared
Windows mount with permissive drvfs modes will be refused.
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
(NSS/NSPR, ALSA, ATK/AT-SPI, CUPS, GBM/DRM, Pango/Cairo and X11 extensions).
Minimal systems may additionally need:
  sudo apt install libnss3 libasound2t64 libatk1.0-0t64 libatk-bridge2.0-0t64 \
    libcups2t64 libgbm1 libxcomposite1 libxdamage1 libpango-1.0-0 libcairo2

Local network discovery and mounted SMB share browsing additionally need:
  sudo apt install gvfs-backends gvfs-fuse wsdd
The wsdd helper discovers Windows computers. Allow time for discovery, then
refresh Network Neighborhood. A known server can be entered as smb://server/.

Example:
  ./me --left "$HOME" --right /tmp

X11/XWayland supports native external file drag-and-drop. Native Wayland currently
supports internal panel dragging only. --software is a diagnostic fallback with
known bitmap-emoji limitations. System fonts must cover the displayed scripts.
Fedora 44+ and openSUSE Leap 16/Tumbleweed are compatibility targets, but this
preview has not been qualified on their native desktops. See SYSTEM_REQUIREMENTS.md.

Optional per-user desktop installation from the extracted folder:
  install -Dm755 me "$HOME/.local/bin/me"
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
or the extracted folder, the retry is still offered but refused, and the message
states this requirement. Earlier packages' me used the included helper
from a folder you own, where any program running as you could replace it: replace
an earlier extracted package with this one, or at least delete its
midnight-elevated-listing. System-wide installation from the extracted folder:
  sudo install -d -m 755 /opt/midnight-explorer
  sudo cp -R . /opt/midnight-explorer/
Then run /opt/midnight-explorer/me. For the desktop launcher, install the desktop
entry and icon as above and put /opt/midnight-explorer on PATH instead. On Ubuntu
23.10+ / Mint 22, use Install... in the HTML viewer's notice, or run the root-owned copy
of the script (this moves the sandbox permission to this installation):
  sudo /opt/midnight-explorer/install-html-viewer-apparmor.sh

Settings: $XDG_CONFIG_HOME/midnight-explorer/preferences.json, normally
$HOME/.config/midnight-explorer/preferences.json.
See FEATURE_STATUS.md, KEYBOARD_SHORTCUTS.md and SECURITY.md for capabilities/limits.
The complete distribution includes all six viewers (text/hex, source, Markdown,
image, PDF and Chromium HTML), native/network/archive filesystems and both
image/video thumbnailers. Keep their complete runtimes, sources and notices.
F3 selects the internal viewer for the captured file.
Alt+F3 forces basic text; Ctrl+F searches; Space advances to the next captured
panel file; Escape closes the viewer. Selection/copy is limited to the loaded page.
F4 opens the external editor configured in Options > Editors. Options > Plugins
lists installed packages; enabling/disabling a package takes effect after restart.
Source and releases: https://github.com/TomasJelinek/fm
Dependency notices and licenses are included alongside the executable.
