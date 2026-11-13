Midnight Explorer 0.2.0 - Linux x86-64 preview

Extract the archive with tar, then run ./me in a graphical desktop session.
Keep the viewers and filesystems folders beside me. Native plugins require a private user-owned
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
  mkdir -p "$HOME/.local/bin/viewers" "$HOME/.local/bin/filesystems"
  cp -R viewers/. "$HOME/.local/bin/viewers/"
  cp -R filesystems/. "$HOME/.local/bin/filesystems/"
  install -Dm644 io.github.TomasJelinek.MidnightExplorer.desktop \
    "$HOME/.local/share/applications/io.github.TomasJelinek.MidnightExplorer.desktop"
  install -Dm644 midnight-explorer.png \
    "$HOME/.local/share/icons/hicolor/256x256/apps/io.github.TomasJelinek.MidnightExplorer.png"
Ensure "$HOME/.local/bin" is on PATH when using the desktop launcher.

Settings: $XDG_CONFIG_HOME/midnight-explorer/preferences.json, normally
$HOME/.config/midnight-explorer/preferences.json.
See FEATURE_STATUS.md, KEYBOARD_SHORTCUTS.md and SECURITY.md for capabilities/limits.
F3 opens text/hex, source or Markdown according to the installed viewer packages.
Alt+F3 forces basic text; Ctrl+F searches; Space advances to the next captured
panel file; Escape closes the viewer. Selection/copy is limited to the loaded page.
F4 opens the external editor configured in Options > Editors. Options > Plugins
lists installed packages; enabling/disabling a package takes effect after restart.
Source and releases: https://github.com/TomasJelinek/fm
Dependency notices and licenses are included alongside the executable.
