#!/usr/bin/env python3
"""Package an already verified executable; never build or publish implicitly."""

import argparse
import contextlib
import hashlib
import json
import os
import re
import shutil
import stat
import tarfile
import tomllib
import zipfile
from pathlib import Path

REQUIRED_VIEWERS = ("midnight.viewer.basic", "midnight.viewer.source", "midnight.viewer.markdown")


def portable_component(name):
    # Archives can be inspected/extracted on another OS. In particular a literal
    # Unix backslash or colon must never become a Windows separator or ADS path.
    reserved = name.split(".", 1)[0].upper()
    if (not name or name in (".", "..") or re.search(r'[\\/:<>"|?*\x00-\x1f]', name)
            or name.endswith((".", " ")) or reserved in ("CON", "PRN", "AUX", "NUL")
            or re.fullmatch(r"(?:COM|LPT)[1-9]", reserved)):
        raise ValueError(f"Nonportable or ambiguous release path component: {name!r}")


def plain_metadata(path):
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or getattr(metadata, "st_file_attributes", 0) & 0x400:
        raise ValueError(f"Symlink/reparse entry is not a release input: {path}")
    if not (stat.S_ISREG(metadata.st_mode) or stat.S_ISDIR(metadata.st_mode)):
        raise ValueError(f"Special file is not a release input: {path}")
    return metadata


def plain_tree(root, maximum=4096):
    root = Path(root).absolute()
    portable_component(root.name)
    if root.resolve() != root or not stat.S_ISDIR(plain_metadata(root).st_mode):
        raise ValueError(f"Release tree must be a real directory without redirected parents: {root}")
    pending, files, count = [root], [], 0
    while pending:
        directory = pending.pop()
        for path in sorted(directory.iterdir()):
            portable_component(path.name)
            count += 1
            if count > maximum:
                raise ValueError(f"Release tree entry limit exceeded: {root}")
            metadata = plain_metadata(path)
            if path.resolve() != path:
                raise ValueError(f"Release entry escapes its package: {path}")
            if stat.S_ISDIR(metadata.st_mode):
                pending.append(path)
            else:
                files.append(path)
    return sorted(files)


@contextlib.contextmanager
def plain_file(path):
    expected = plain_metadata(path)
    if not stat.S_ISREG(expected.st_mode):
        raise ValueError(f"Expected regular release file: {path}")
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        if (expected.st_dev, expected.st_ino) != (opened.st_dev, opened.st_ino):
            raise ValueError(f"Release input changed while opening: {path}")
        yield stream
        after = os.fstat(stream.fileno())
        if (opened.st_size, opened.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            raise ValueError(f"Release input changed while reading: {path}")


def sha256_file(path):
    with plain_file(path) as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def copy_viewer_packages(stage, viewers, target, required_ids=()):
    """Copy independently built packages, validating all inputs before staging any plugin."""
    viewers = Path(viewers).absolute()
    if viewers.resolve() != viewers or not stat.S_ISDIR(plain_metadata(viewers).st_mode):
        raise ValueError("Viewer package root must be a real directory")
    if Path(stage).resolve().is_relative_to(viewers):
        raise ValueError("Release staging cannot be inside its viewer input tree")
    packages, identities = [], set()
    entries = sorted(viewers.iterdir())
    if not entries or len(entries) > 256:
        raise ValueError("Expected 1..256 independently built viewer packages")
    for package in entries:
        files = plain_tree(package)
        manifest_path = package / "plugin.toml"
        with plain_file(manifest_path) as stream:
            source = stream.read(65537)
        if len(source) > 65536:
            raise ValueError(f"Oversized plugin manifest: {package}")
        manifest = tomllib.loads(source.decode("utf-8"))
        identity, version = manifest.get("id"), manifest.get("version")
        if not isinstance(identity, str) or not re.fullmatch(r"[a-zA-Z0-9._-]{1,128}", identity):
            raise ValueError(f"Invalid plugin ID: {package}")
        if identity in identities:
            raise ValueError(f"Duplicate viewer plugin ID: {identity}")
        identities.add(identity)
        if not isinstance(version, str) or not 1 <= len(version) <= 64:
            raise ValueError(f"Invalid plugin version: {package}")
        if (type(manifest.get("schema_version")) is not int or manifest["schema_version"] != 1
                or manifest.get("kind") != "viewer" or manifest.get("target") != target
                or type(manifest.get("abi_major")) is not int or manifest["abi_major"] != 1
                or type(manifest.get("minimum_host_minor")) is not int
                or manifest["minimum_host_minor"] != 0):
            raise ValueError(f"Incompatible viewer manifest: {package}")
        library = manifest.get("entry_library")
        if (not isinstance(library, str) or not 1 <= len(library) <= 255
                or library in (".", "..") or any(char in library for char in "/\\:\0")
                or library.endswith((".", " "))):
            raise ValueError(f"Viewer entry library must be one filename: {package}")
        models = manifest.get("models", [])
        services = manifest.get("required_host_services", [])
        if (not isinstance(models, list) or not 1 <= len(models) <= 3
                or any(model not in ("text-page.v1", "binary-page.v1", "document-blocks.v1") for model in models)
                or not isinstance(services, list) or len(services) > 16
                or any(service not in ("content.read.v1", "work.schedule.v1") for service in services)):
            raise ValueError(f"Unsupported viewer models/services: {package}")
        for required in ("plugin.toml", library, "LICENSE", "THIRD-PARTY.md"):
            if package / required not in files:
                raise ValueError(f"Viewer package missing required file {required}: {package}")
        packages.append((package, manifest, files))
    missing = set(required_ids) - identities
    if missing:
        raise ValueError("Missing required viewer packages: " + ", ".join(sorted(missing)))
    inventory = []
    for package, manifest, files in packages:
        copied = []
        for source in files:
            relative = Path("viewers") / package.name / source.relative_to(package)
            destination = stage / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            with plain_file(source) as original, destination.open("xb") as output:
                shutil.copyfileobj(original, output)
            destination.chmod(0o644)
            copied.append({"path": relative.as_posix(), "bytes": destination.stat().st_size,
                           "sha256": sha256_file(destination)})
        fingerprint = hashlib.sha256(json.dumps(copied, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        inventory.append({"id": manifest["id"], "version": manifest["version"],
                          "target": target, "abi_major": manifest["abi_major"],
                          "entry_library": manifest["entry_library"], "models": manifest["models"],
                          "package_sha256": fingerprint, "files": copied})
    return inventory


def dependency_packages(metadata, dependency_list):
    """Use the application's tree, not metadata's workspace-wide feature union."""
    selected = set()
    for line in dependency_list.splitlines():
        if not line.strip():
            continue
        match = re.fullmatch(r"(\S+) v(\S+)(?: .*)?", line.strip())
        if not match:
            raise ValueError(f"Malformed dependency line (use --prefix none): {line!r}")
        selected.add(match.groups())
    if not any(name == "midnight-explorer" for name, _ in selected):
        raise ValueError("Dependency list must contain midnight-explorer")
    packages = {}
    for package in metadata["packages"]:
        key = (package["name"], package["version"])
        if key in selected:
            if key in packages:
                raise ValueError(f"Ambiguous dependency source: {key}")
            packages[key] = package
    if selected - packages.keys():
        raise ValueError(f"Dependencies missing from metadata: {selected - packages.keys()}")
    return [package for _, package in sorted(packages.items()) if package["source"]]


def license_files(package):
    source = Path(package["manifest_path"]).parent
    files = set()
    if package.get("license_file"):
        files.add(source / package["license_file"])
    for entry in source.iterdir():
        name = entry.name.lower()
        if name.startswith(("license", "licence", "copying", "copyright", "notice")):
            if entry.is_dir():
                files.update(path for path in entry.rglob("*") if path.is_file())
            elif entry.is_file():
                files.add(entry)
    return source, sorted(files)


def add_notices(stage, metadata, dependency_list, rust_docs, overrides):
    inventory = []
    missing = []
    for package in dependency_packages(metadata, dependency_list):
        name = f'{package["name"]}-{package["version"]}'
        source, files = license_files(package)
        if not files and overrides:
            source = overrides / name
            files = sorted(path for path in source.rglob("*") if path.is_file())
        if not files:
            missing.append(name)
        for path in files:
            destination = stage / "licenses" / name / path.relative_to(source)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
        inventory.append({key: package.get(key) for key in
                          ("name", "version", "license", "repository", "source")})
    if missing:
        raise ValueError("Missing dependency license files: " + ", ".join(missing))
    rust_target = stage / "licenses" / "rust"
    rust_target.mkdir(parents=True)
    shutil.copyfile(rust_docs / "COPYRIGHT-library.html", rust_target / "COPYRIGHT-library.html")
    (stage / "THIRD-PARTY.json").write_text(
        json.dumps(inventory, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (stage / "THIRD-PARTY.txt").write_text(
        "Dependency names, versions and declared licenses are in THIRD-PARTY.json.\n"
        "Original license and copyright notices are retained under licenses/.\n"
        "The inventory conservatively includes build dependencies, not just linked code.\n"
        "Rust standard-library notices are in licenses/rust/.\n"
        "Midnight Explorer uses Slint under its royalty-free desktop application license.\n"
        "The release download page displays Slint's attribution badge.\n"
        "These notices do not assign a new license to Midnight Explorer source code.\n",
        encoding="utf-8")


def write_archive(stage, archive, binary_name, archive_format):
    binary_path = f"{stage.name}/{binary_name}"
    files = plain_tree(stage, maximum=65536)

    def safe_tar_member(member):
        if not (member.isdir() or member.isfile()):
            raise ValueError(f"Unexpected archive entry type: {member.name}")
        member.mode = 0o755 if member.isdir() or member.name == binary_path else 0o644
        member.uid = member.gid = 0
        member.uname = member.gname = ""
        member.mtime = int(member.mtime)
        member.pax_headers = {}
        return member

    if archive_format == "zip":
        with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
            for path in files:
                info = zipfile.ZipInfo.from_file(path, path.relative_to(stage.parent))
                info.create_system = 3
                mode = 0o755 if info.filename == binary_path else 0o644
                info.external_attr = (stat.S_IFREG | mode) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                with plain_file(path) as source, bundle.open(info, "w") as target:
                    shutil.copyfileobj(source, target)
    else:
        with tarfile.open(archive, "x:gz", compresslevel=9) as bundle:
            bundle.add(stage, arcname=stage.name, filter=safe_tar_member)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--dependency-list", type=Path, required=True)
    parser.add_argument("--rust-docs", type=Path, required=True)
    parser.add_argument("--license-overrides", type=Path)
    parser.add_argument("--target", required=True)
    parser.add_argument("--format", choices=("zip", "tar.gz"), required=True)
    parser.add_argument("--readme", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--desktop-entry", type=Path)
    parser.add_argument("--viewers-root", type=Path,
                        help="Independent plugin packages; defaults to viewers beside the executable")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[a-z0-9][a-z0-9._-]*", args.target):
        raise ValueError("Invalid target name")
    if not re.fullmatch(r"[0-9a-f]{40}", args.revision):
        raise ValueError("Expected a full Git revision")
    root = Path(__file__).resolve().parent.parent
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    rust = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    name = f"midnight-explorer-{version}-{args.target}"
    output = args.output.resolve()
    if not output.is_relative_to(root / "_build"):
        raise ValueError("Release output must remain under the repository's _build directory")
    stage = output / name
    stage.mkdir(parents=True, exist_ok=False)
    binary = stage / args.binary.name
    shutil.copyfile(args.binary, binary)
    binary.chmod(0o755)
    shutil.copyfile(args.readme, stage / "README.txt")
    shutil.copyfile(root / "Cargo.lock", stage / "Cargo.lock")
    shutil.copyfile(root / "assets/branding/midnight-explorer.png", stage / "midnight-explorer.png")
    for relative in ("SECURITY.md", "docs/SYSTEM_REQUIREMENTS.md", "docs/KEYBOARD_SHORTCUTS.md",
                     "docs/FEATURE_STATUS.md"):
        shutil.copyfile(root / relative, stage / Path(relative).name)
    if args.desktop_entry:
        shutil.copyfile(args.desktop_entry, stage / args.desktop_entry.name)
    plugins = copy_viewer_packages(stage, args.viewers_root or args.binary.parent / "viewers",
                                  args.target, required_ids=REQUIRED_VIEWERS)
    add_notices(stage, json.loads(args.metadata.read_text(encoding="utf-8-sig")),
                args.dependency_list.read_text(encoding="utf-8-sig"),
                args.rust_docs, args.license_overrides)
    with binary.open("rb") as stream:
        binary_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    (stage / "BUILD-INFO.json").write_text(json.dumps({
        "version": version, "target": args.target, "revision": args.revision,
        "profile": "release", "rust": rust, "binary": binary.name,
        "binary_sha256": binary_hash,
        "viewer_plugins": plugins,
    }, indent=2) + "\n", encoding="utf-8")
    archive = output / f"{name}.{args.format}"
    if archive.exists():
        raise FileExistsError(archive)
    write_archive(stage, archive, binary.name, args.format)
    print(archive)


if __name__ == "__main__":
    main()
