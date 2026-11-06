#!/usr/bin/env python3
"""Package an already verified executable; never build or publish implicitly."""

import argparse
import hashlib
import json
import re
import shutil
import tarfile
import tomllib
import zipfile
from pathlib import Path


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
    add_notices(stage, json.loads(args.metadata.read_text(encoding="utf-8-sig")),
                args.dependency_list.read_text(encoding="utf-8-sig"),
                args.rust_docs, args.license_overrides)
    with binary.open("rb") as stream:
        binary_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    (stage / "BUILD-INFO.json").write_text(json.dumps({
        "version": version, "target": args.target, "revision": args.revision,
        "profile": "release", "rust": rust, "binary": binary.name,
        "binary_sha256": binary_hash,
    }, indent=2) + "\n", encoding="utf-8")
    archive = output / f"{name}.{args.format}"
    if archive.exists():
        raise FileExistsError(archive)
    if args.format == "zip":
        with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
            for path in sorted(stage.rglob("*")):
                if path.is_file():
                    bundle.write(path, path.relative_to(output))
    else:
        with tarfile.open(archive, "x:gz", compresslevel=9) as bundle:
            bundle.add(stage, arcname=name)
    print(archive)


if __name__ == "__main__":
    main()
