Midnight Explorer 0.1.0 - Linux x86-64 preview

Extract the archive with tar, then run ./me in a graphical desktop session.
Built on Ubuntu 24.04 / glibc 2.39. Requires GNU/glibc, not Alpine/musl.
Rust is not needed to run the application.

Ubuntu 24.04 / Mint 22 runtime packages:
  sudo apt install libfontconfig1 libfreetype6 libxkbcommon0 libxkbcommon-x11-0 \
    libx11-6 libxcursor1 libxi6 libxrandr2 libxcb1 libwayland-client0 libudev1 \
    libvulkan1 xdg-utils xterm
Use the graphics driver and Vulkan implementation supplied for your hardware.

Example:
  ./me --left "$HOME" --right /tmp

X11/XWayland supports native external file drag-and-drop. Native Wayland currently
supports internal panel dragging only. --software is a diagnostic fallback with
known bitmap-emoji limitations. System fonts must cover the displayed scripts.
Fedora 44+ and openSUSE Leap 16/Tumbleweed are compatibility targets, but this
preview has not been qualified on their native desktops. See SYSTEM_REQUIREMENTS.md.

Optional per-user desktop installation from the extracted folder:
  install -Dm755 me "$HOME/.local/bin/me"
  install -Dm644 io.github.TomasJelinek.MidnightExplorer.desktop \
    "$HOME/.local/share/applications/io.github.TomasJelinek.MidnightExplorer.desktop"
  install -Dm644 midnight-explorer.png \
    "$HOME/.local/share/icons/hicolor/256x256/apps/io.github.TomasJelinek.MidnightExplorer.png"
Ensure "$HOME/.local/bin" is on PATH when using the desktop launcher.

Settings: $XDG_CONFIG_HOME/midnight-explorer/preferences.json, normally
$HOME/.config/midnight-explorer/preferences.json.
See FEATURE_STATUS.md, KEYBOARD_SHORTCUTS.md and SECURITY.md for capabilities/limits.
Source and releases: https://github.com/TomasJelinek/fm
Dependency notices and licenses are included alongside the executable.
