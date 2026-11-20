"""Preserve exact pinned dependency license/notice files in a staged plugin package."""
import argparse
import json
import os
import pathlib
import re
import shutil
import stat
import subprocess

# The inventory marks a directory as this script's output (see replace_notices).
INVENTORY = "inventory.json"
MAX_ENTRIES = 65536
NOTICE_PREFIXES = ("license", "licence", "copying", "notice", "copyright")


def linked(metadata):
    """A symbolic link or a Windows reparse point such as a junction."""
    return stat.S_ISLNK(metadata.st_mode) or bool(getattr(metadata, "st_file_attributes", 0) & 0x400)


def is_link(path):
    """Whether an existing path is a link; a missing path is not."""
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return False
    return linked(metadata)


def read_package_file(root, path):
    """Read a regular file below a dependency's root without following links.

    Only the root is resolved, so links above it (a linked CARGO_HOME, /home -> /var/home)
    and a root that is itself a link are accepted. Every name below it must be a plain
    portable name and no component may be a link, so neither a link, '..' nor a drive or
    stream name can substitute content from outside the package.
    """
    root, path = pathlib.Path(root).absolute(), pathlib.Path(path).absolute()
    if path == root or not path.is_relative_to(root):
        raise SystemExit(f"Dependency notice escapes its package: {path}")
    parts = path.relative_to(root).parts
    # Names are checked before any access: on Windows 'D:x' re-anchors a joined path on
    # drive D: and 'LICENSE:x' names a data stream.
    for part in parts:
        if (part in ("", ".", "..") or re.search(r'[\\/:<>"|?*\x00-\x1f]', part)
                or part.endswith((".", " "))
                or re.fullmatch(r"(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?", part, re.IGNORECASE)):
            raise SystemExit(f"Dependency notice escapes its package or is not portable: {path}")
    current = root.resolve(strict=True)
    for part in parts:
        current = current / part
        metadata = current.lstat()
        if linked(metadata):
            raise SystemExit(f"Dependency notice must not traverse links: {path}")
    if not stat.S_ISREG(metadata.st_mode):
        raise SystemExit(f"Dependency notice must be a regular file: {path}")
    with open(current, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if (metadata.st_dev, metadata.st_ino) != (opened.st_dev, opened.st_ino):
            raise SystemExit(f"Dependency notice changed while it was opened: {path}")
        return stream.read()


def checked_output(value):
    """The absolute output path below _build, inspected before anything resolves it.

    A link from the _build component down would redirect the replacement to a
    directory that this script did not create.
    """
    raw = pathlib.Path(value)
    if ".." in raw.parts:
        raise SystemExit("Distribution notices output must not contain '..'")
    output = raw.absolute()
    if "_build" not in output.parts[:-1]:
        raise SystemExit("Distribution notices must be staged under _build")
    first = output.parts.index("_build")
    for depth in range(first + 1, len(output.parts) + 1):
        candidate = pathlib.Path(*output.parts[:depth])
        if is_link(candidate):
            raise SystemExit(f"Distribution notices output must not traverse links: {candidate}")
    return output


def notice_plan(metadata):
    """Read every notice before the previous output is changed."""
    plan = {}
    inventory = []
    for package in metadata["packages"]:
        if package["source"] is None or "midnight-explorer.invalid" in package["source"]:
            continue
        license_name = package.get("license")
        if not license_name:
            raise SystemExit(f"Missing license metadata: {package['name']}")
        source = pathlib.Path(package["manifest_path"]).parent
        # Discovery never follows links: a link named like a notice is refused, not read.
        files = []
        for path in sorted(source.iterdir()):
            if path.name.lower().startswith(NOTICE_PREFIXES):
                metadata = path.lstat()
                if linked(metadata):
                    raise SystemExit(f"Dependency notice must not traverse links: {path}")
                if stat.S_ISREG(metadata.st_mode):
                    files.append(path)
        if package.get("license_file"):
            files.append(source / package["license_file"])
        if not files:
            raise SystemExit(f"Pinned license files must be reviewed: {package['name']}")
        plan[f"{package['name']}-{package['version']}"] = {
            path.name: read_package_file(source, path) for path in files}
        inventory.append({"name": package["name"], "version": package["version"], "license": license_name})
    return plan, inventory


def generated_entries(entry):
    """Yield entry and everything below it without following links."""
    yield entry
    if entry.is_dir() and not is_link(entry):
        for directory, directories, files in os.walk(entry, followlinks=False):
            for name in directories + files:
                yield pathlib.Path(directory, name)


def replace_notices(output, plan, inventory):
    """Replace only an earlier output of this script, identified by its inventory.

    The inventory is written before any other change and is never removed, so an
    interrupted regeneration is completed by the next build instead of blocking it.
    Notices of a dependency that is no longer shipped never survive a rebuild.
    """
    marker = output / INVENTORY
    stale = []
    if is_link(output):
        raise SystemExit(f"Distribution notices output must not be a link: {output}")
    if output.exists():
        if not output.is_dir():
            raise SystemExit("Distribution notices output must be a directory")
        entries = sorted(output.iterdir())
        if entries and (is_link(marker) or not marker.is_file()):
            raise SystemExit(f"Refusing to replace a directory that is not generated notices: {output}")
        stale = [entry for entry in entries if entry.name != INVENTORY]
        count = 0
        for entry in stale:
            for path in generated_entries(entry):
                count += 1
                if count > MAX_ENTRIES:
                    raise SystemExit(f"Generated notices exceed {MAX_ENTRIES} entries: {output}")
                # This script never creates links; never delete through one.
                if is_link(path):
                    raise SystemExit(f"Refusing to replace generated notices that contain a link: {path}")
    else:
        output.mkdir(parents=True)
    marker.write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
    for entry in stale:
        if entry.is_dir():
            shutil.rmtree(entry)
        else:
            entry.unlink()
    for name, files in plan.items():
        target = output / name
        target.mkdir()
        for filename, content in files.items():
            (target / filename).write_bytes(content)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = checked_output(args.output)
    # Cargo writes UTF-8 JSON; Python before 3.15 would decode it in the ANSI code page.
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--format-version", "1", "--locked", "--offline",
        "--manifest-path", str(root / "Cargo.toml"), "--config", args.config,
    ], text=True, encoding="utf-8"))
    plan, inventory = notice_plan(metadata)
    replace_notices(output, plan, inventory)


if __name__ == "__main__":
    main()
