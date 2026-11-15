#!/usr/bin/env python3
"""Register an existing Midnight Explorer build with the current Linux desktop."""
import argparse
import configparser
from contextlib import contextmanager
import os
from pathlib import Path
import secrets
import stat
import sys


ROOT = Path(__file__).resolve().parents[1]
APP_ID = "io.github.TomasJelinek.MidnightExplorer"


def data_home():
    configured = os.environ.get("XDG_DATA_HOME", "")
    if configured and Path(configured).is_absolute():
        return Path(configured)
    home = os.environ.get("HOME", "")
    if not home or not Path(home).is_absolute():
        raise ValueError("Set an absolute XDG_DATA_HOME or HOME directory.")
    return Path(home) / ".local/share"


def desktop_string(value):
    return (value.replace("\\", "\\\\").replace("\n", "\\n")
            .replace("\r", "\\r").replace("\t", "\\t"))


def executable_command(executable):
    value = str(executable)
    value.encode("utf-8", errors="strict")
    if "=" in value:
        raise ValueError("The Desktop Entry specification forbids '=' in executable paths.")
    if any((ord(character) < 32 and character not in "\n\r\t") or ord(character) == 127
           for character in value):
        raise ValueError("The executable path contains an unsupported control character.")
    # Desktop string unescaping happens before Exec argument unquoting and field expansion.
    quoted = "".join("\\" + character if character in '\\"`$' else character
                     for character in value)
    command = desktop_string('"' + quoted.replace("%", "%%") + '"')
    if "%" in value:
        # Gio checks argv[0] before %% expansion; a fixed launcher keeps this path an argument.
        launcher = Path("/usr/bin/env")
        if not launcher.is_file() or not os.access(launcher, os.X_OK):
            raise ValueError("Executable paths containing '%' require /usr/bin/env.")
        command = f"{launcher} -- {command}"
    return command


@contextmanager
def open_directory(path):
    """Pin each directory without following symbolic links, including during creation."""
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for component in path.parts[1:]:
            try:
                os.mkdir(component, mode=0o755, dir_fd=descriptor)
            except FileExistsError:
                pass
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
                            | os.O_CLOEXEC, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        yield descriptor
    finally:
        os.close(descriptor)


def existing_file(directory, name):
    try:
        descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
                             | os.O_CLOEXEC, dir_fd=directory)
    except FileNotFoundError:
        return None
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 4 * 1024 * 1024:
            raise ValueError(f"Refusing a non-regular or oversized existing file: {name}")
        contents = source.read(4 * 1024 * 1024 + 1)
        if len(contents) > 4 * 1024 * 1024:
            raise ValueError(f"Refusing an oversized existing file: {name}")
        return contents


def check_desktop_entry(contents):
    if contents is None:
        return
    entry = configparser.ConfigParser(interpolation=None)
    try:
        entry.read_string(contents.decode("utf-8"))
        section = entry["Desktop Entry"]
        if (section.get("Type") == "Application" and section.get("Name") == "Midnight Explorer"
                and section.get("StartupWMClass") == APP_ID):
            return
    except (UnicodeError, configparser.Error, KeyError):
        pass
    raise ValueError("Refusing to replace an unrelated existing desktop entry.")


def publish(directory, name, contents):
    temporary = f".{name}.{secrets.token_hex(12)}.tmp"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
                         | os.O_CLOEXEC, 0o644, dir_fd=directory)
    try:
        with os.fdopen(descriptor, "wb") as destination:
            destination.write(contents)
            destination.flush()
            os.fsync(destination.fileno())
        # Both names are relative to a retained directory, never a freshly resolved path.
        os.replace(temporary, name, src_dir_fd=directory, dst_dir_fd=directory)
    finally:
        try:
            os.unlink(temporary, dir_fd=directory)
        except FileNotFoundError:
            pass


def install(executable):
    executable = executable.absolute()
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError(f"Build Midnight Explorer first; no executable file at {executable}")
    command = executable_command(executable)
    template = (ROOT / f"packaging/linux/{APP_ID}.desktop").read_text(encoding="utf-8")
    replacements = {"Exec": command, "TryExec": desktop_string(str(executable))}
    lines = []
    for line in template.splitlines():
        key = line.partition("=")[0]
        lines.append(f"{key}={replacements[key]}" if key in replacements else line)
    desktop_contents = ("\n".join(lines) + "\n").encode("utf-8")
    icon_contents = (ROOT / "assets/branding/midnight-explorer.png").read_bytes()
    base = data_home()
    applications = base / "applications"
    icons = base / "icons/hicolor/256x256/apps"
    desktop_name = f"{APP_ID}.desktop"
    icon_name = f"{APP_ID}.png"
    with open_directory(base / "icons/hicolor") as theme_dir:
        with open_directory(applications) as application_dir, open_directory(icons) as icon_dir:
            check_desktop_entry(existing_file(application_dir, desktop_name))
            existing_file(icon_dir, icon_name)
            publish(icon_dir, icon_name, icon_contents)
            publish(application_dir, desktop_name, desktop_contents)
            # GTK watches the theme root mtime, not nested apps directories. Invalidate
            # existing caches only after both dedicated files were successfully published.
            os.utime(theme_dir, None)
    return applications / desktop_name


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, default=ROOT / "_build/debug/me",
                        help="existing executable to register (default: _build/debug/me)")
    arguments = parser.parse_args()
    try:
        if not sys.platform.startswith("linux"):
            raise ValueError("Desktop registration requires Linux.")
        desktop = install(arguments.executable)
    except (OSError, ValueError) as error:
        parser.exit(1, f"Desktop registration failed: {error}\n")
    print(f"Registered Midnight Explorer: {desktop}")
    print("Restart Midnight Explorer to refresh its running application icon.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
