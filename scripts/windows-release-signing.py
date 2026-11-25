#!/usr/bin/env python3
"""Prepare and finalize a Windows release around native trusted signing.

Never executes archive contents. Verification receipts are written by the trusted
PowerShell verifier in the same job, outside the extracted package.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import tomllib
import zipfile

_spec = importlib.util.spec_from_file_location(
    "release_packaging", Path(__file__).with_name("package-release.py"))
packaging = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(packaging)

TARGET = "x86_64-pc-windows-msvc"
SIGNING_FILES = (
    "me.exe", "midnight-elevated-listing.exe",
    "viewers/midnight-viewer-basic/midnight_viewer_basic.dll",
    "viewers/midnight-viewer-source/midnight_viewer_source.dll",
    "viewers/midnight-viewer-markdown/midnight_viewer_markdown.dll",
    "viewers/midnight-viewer-image/midnight-viewer-image.exe",
    "viewers/midnight-viewer-pdf/midnight-viewer-pdf.exe",
    # CEF's unsigned upstream bootstrap and the project client must share a signer.
    "viewers/midnight-viewer-html/midnight-viewer-html.exe",
    "viewers/midnight-viewer-html/midnight-viewer-html.dll",
    "filesystems/native/midnight_filesystem_native_windows.dll",
    "filesystems/network/midnight_filesystem_network.dll",
    "filesystems/archive/midnight-fs-archive.exe",
    "thumbnailers/midnight-thumbnail-image.exe",
    "thumbnailers/midnight-thumbnail-video.exe",
    "thumbnailers/midnight-video-playback.exe",
)
MAX_BYTES = 8 * 1024**3
MAX_ENTRIES = 65536


def unique_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate JSON field: " + key)
        result[key] = value
    return result


def read_json(path):
    with packaging.plain_file(path) as stream:
        encoded = stream.read(32 * 1024**2 + 1)
    if len(encoded) > 32 * 1024**2:
        raise ValueError("Oversized signing metadata")
    return json.loads(encoded.decode("utf-8-sig"), object_pairs_hook=unique_fields)


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8", newline="\n")


def fingerprint(records):
    return hashlib.sha256(json.dumps(records, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def relative_path(value):
    if not isinstance(value, str) or not value:
        raise ValueError("Expected a relative package path")
    for component in value.split("/"):
        packaging.portable_component(component)
    return Path(*PurePosixPath(value).parts)


def snapshot(root):
    return {path.relative_to(root).as_posix():
            {"bytes": path.stat().st_size, "sha256": packaging.sha256_file(path)}
            for path in packaging.plain_tree(root, maximum=MAX_ENTRIES)}


def groups(info):
    return [*info["viewer_plugins"], *info["filesystem_plugins"], info["thumbnail_workers"]]


def check_inventory(root, info):
    actual = snapshot(root)
    if info["binary"] != "me.exe" or info["binary_sha256"] != actual["me.exe"]["sha256"]:
        raise ValueError("Application hash or filename mismatch")
    helper = info["authorization_helper"]
    if helper["path"] != "midnight-elevated-listing.exe":
        raise ValueError("Unexpected authorization helper")
    indexed = {"me.exe"}
    for record in [helper, *(record for group in groups(info) for record in group["files"])]:
        name = record["path"]
        relative_path(name)
        if (name in indexed or name not in actual or type(record["bytes"]) is not int
                or record["bytes"] != actual[name]["bytes"]
                or record["sha256"] != actual[name]["sha256"]):
            raise ValueError("Duplicate, missing or mismatched inventory file: " + name)
        indexed.add(name)
    for group in groups(info):
        if group["package_sha256"] != fingerprint(group["files"]):
            raise ValueError("Package fingerprint mismatch")
    module_files = {name for name in actual if name.startswith(("viewers/", "filesystems/", "thumbnailers/"))}
    if module_files != indexed - {"me.exe", helper["path"]}:
        raise ValueError("Module inventories do not cover the complete payload")
    for name in SIGNING_FILES:
        if name not in indexed:
            raise ValueError("Missing required signing target: " + name)
    # A manifest may select a different entry DLL while retaining the expected
    # signed DLL as an unused file. Bind the actual load/launch targets to policy.
    for kind in ("viewers", "filesystems"):
        for directory in (root / kind).iterdir():
            for manifest in directory.iterdir():
                if manifest.name == "plugin.toml":
                    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
                    entry = data.get("entry_library", data.get("entry_executable"))
                elif manifest.name in ("image-viewer.json", "web-viewer.json"):
                    entry = read_json(manifest)["executable"]
                else:
                    continue
                name = (directory.relative_to(root) / relative_path(entry)).as_posix()
                if name not in SIGNING_FILES:
                    raise ValueError("Manifest entry is outside the signing policy: " + name)
    for manifest in (root / "thumbnailers").glob("*.json"):
        entry = read_json(manifest)["executable"]
        if (Path("thumbnailers") / relative_path(entry)).as_posix() not in SIGNING_FILES:
            raise ValueError("Thumbnail entry is outside the signing policy")
    for name in actual:
        path = PurePosixPath(name)
        if (path.suffix.lower() in (".exe", ".dll") and name not in SIGNING_FILES
                and (path.suffix.lower() == ".exe" or path.name.lower().startswith("midnight"))):
            raise ValueError("Unrecognized executable requires a signing-policy update: " + name)
    return actual


def extract_archive(archive, destination, name):
    with zipfile.ZipFile(archive) as bundle:
        entries = bundle.infolist()
        if not entries or len(entries) > MAX_ENTRIES or sum(e.file_size for e in entries) > MAX_BYTES:
            raise ValueError("Release ZIP count or size limit exceeded")
        seen, files, directories = set(), set(), set()
        for entry in entries:
            if entry.orig_filename != entry.filename:
                raise ValueError("ZIP member name was normalized by the ZIP reader")
            value = entry.filename[:-1] if entry.is_dir() else entry.filename
            path = relative_path(value)
            mode = entry.external_attr >> 16
            kind = stat.S_IFMT(mode)
            if (path.parts[0] != name or entry.flag_bits & 1 or entry.external_attr & 0x400
                    or kind not in (0, stat.S_IFREG, stat.S_IFDIR)
                    or (kind == stat.S_IFDIR) != entry.is_dir() and kind != 0):
                raise ValueError("Unsafe or unexpected release ZIP member: " + value)
            key = value.casefold()
            if key in seen or any(parent.as_posix().casefold() in files for parent in PurePosixPath(value).parents):
                raise ValueError("Duplicate or conflicting release ZIP path: " + value)
            seen.add(key)
            if not entry.is_dir():
                if key in directories:
                    raise ValueError("ZIP file conflicts with a directory")
                files.add(key)
            directories.update(parent.as_posix().casefold() for parent in PurePosixPath(value).parents)
            if entry.is_dir():
                directories.add(key)
        destination.mkdir(parents=True, exist_ok=False)
        for entry in entries:
            target = destination / relative_path(entry.filename.rstrip("/"))
            if entry.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(entry) as source, target.open("xb") as output:
                shutil.copyfileobj(source, output)


def prepare(archive, work_dir, revision, version):
    if not re.fullmatch(r"[0-9a-f]{40}", revision) or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Expected a full revision and numeric release version")
    name = f"midnight-explorer-{version}-{TARGET}"
    archive = Path(archive).absolute()
    if archive.name != name + ".zip":
        raise ValueError("Unexpected release ZIP name")
    work = packaging.build_output(work_dir, Path(__file__).resolve().parents[1])
    if work.exists():
        raise ValueError("Signing workspace must not exist; use a fresh directory")
    work.mkdir(parents=True)
    extract_archive(archive, work / "package", name)
    root = work / "package" / name
    info = read_json(root / "BUILD-INFO.json")
    if (info.get("version"), info.get("revision"), info.get("target"), info.get("profile")) != (
            version, revision, TARGET, "release"):
        raise ValueError("Release version, revision, target or profile mismatch")
    packaging.validate_runtime_installation(root, TARGET)
    before = check_inventory(root, info)
    plan = {"schema": 1, "root": str(root), "version": version, "revision": revision,
            "archive": archive.name, "input_sha256": packaging.sha256_file(archive),
            "files": list(SIGNING_FILES), "before": before}
    write_json(work / "signing-plan.json", plan)
    (work / "signing-files.txt").write_text("".join(
        f"package/{name}/{value}\n" for value in SIGNING_FILES), encoding="utf-8", newline="\n")
    return plan


def refresh_records(root, info):
    info["binary_sha256"] = packaging.sha256_file(root / info["binary"])
    for record in [info["authorization_helper"], *(record for group in groups(info) for record in group["files"])]:
        path = root / relative_path(record["path"])
        record.update(bytes=path.stat().st_size, sha256=packaging.sha256_file(path))
    for group in groups(info):
        group["package_sha256"] = fingerprint(group["files"])


def finalize(work_dir):
    work = packaging.build_output(work_dir, Path(__file__).resolve().parents[1])
    work = packaging.real_directory(work, "Signing workspace")
    plan_path = work / "signing-plan.json"
    plan = read_json(plan_path)
    root = work / "package" / f'midnight-explorer-{plan["version"]}-{TARGET}'
    if str(root) != plan["root"] or plan["files"] != list(SIGNING_FILES):
        raise ValueError("Signing plan root or target policy mismatch")
    receipt = read_json(work / "verification.json")
    if receipt.get("schema") != 1 or receipt.get("plan_sha256") != packaging.sha256_file(plan_path):
        raise ValueError("Native verification receipt is stale")
    verified = {record["path"]: record["sha256"] for record in receipt["files"]}
    if len(receipt["files"]) != len(SIGNING_FILES) or set(verified) != set(SIGNING_FILES):
        raise ValueError("Native verification did not cover the signing plan")
    current = snapshot(root)
    if current.keys() != plan["before"].keys():
        raise ValueError("Signing added or removed payload files")
    for name, record in current.items():
        if name in verified:
            if record["sha256"] != verified[name]:
                raise ValueError("Signed file changed after native verification: " + name)
        elif record != plan["before"][name]:
            raise ValueError("Signing modified an unrelated file: " + name)
    # This inventory also covers the bootstrap and its companion DLL. Update it
    # before its own bytes/hash in the outer release manifest.
    inner_path = root / "viewers/midnight-viewer-html/runtime-files.json"
    inner = read_json(inner_path)
    for record in inner["files"]:
        path = inner_path.parent / relative_path(record["path"])
        record.update(bytes=path.stat().st_size, sha256=packaging.sha256_file(path))
    write_json(inner_path, inner)
    info_path = root / "BUILD-INFO.json"
    info = read_json(info_path)
    refresh_records(root, info)
    write_json(info_path, info)
    packaging.validate_runtime_installation(root, TARGET)
    check_inventory(root, info)
    output = work / "output"
    output.mkdir(exist_ok=False)
    archive = output / plan["archive"]
    packaging.write_archive(root, archive, "me.exe", "zip", list(SIGNING_FILES))
    digest = packaging.sha256_file(archive)
    archive.with_suffix(".zip.sha256").write_text(
        f"{digest}  {archive.name}\n", encoding="utf-8", newline="\n")
    write_json(archive.with_suffix(".zip.signing.json"), {
        "schema": 1, "revision": plan["revision"], "version": plan["version"],
        "input_sha256": plan["input_sha256"], "archive_sha256": digest,
        "verification": receipt})
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("prepare")
    command.add_argument("--archive", type=Path, required=True)
    command.add_argument("--work-dir", type=Path, required=True)
    command.add_argument("--revision", required=True)
    command.add_argument("--version", required=True)
    command = commands.add_parser("finalize")
    command.add_argument("--work-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.archive, args.work_dir, args.revision, args.version)
    else:
        print(finalize(args.work_dir))


if __name__ == "__main__":
    main()
