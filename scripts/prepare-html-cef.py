#!/usr/bin/env python3
"""Prepare the exact reviewed CEF 156 SDK without changing sandbox or codec policy."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import sys
import tarfile
import tempfile
import urllib.parse
import urllib.request
import uuid


ROOT = Path(__file__).resolve().parents[1]
PROJECT = ROOT / "plugins/midnight-viewer-html"
CACHE_ROOT = ROOT / ".tools/cef"
MAX_MEMBERS = 20000
MAX_EXPANDED = 5 * 1024**3
MAX_FILE = 2 * 1024**3


def pin_for(platform_name):
    if platform_name == "linux":
        pin = json.loads((PROJECT / "scripts/cef-linux-pinned.json").read_text())
        return {**pin, "bytes": 333351579}
    if platform_name == "win32":
        return {"archive": "cef_binary_156.0.26+g8ea0c73+chromium-156.0.8078.88_windows64_minimal.tar.bz2",
                "archive_sha256": "2b0aa070793a6bb858b737f5edf9cb01a3ef7b9adcad5efe1e08affaf250f809",
                "sha1": "15d29eea209d857c22dc163706080295aea69b5c", "bytes": 175046254}
    raise ValueError("CEF runtime builds support Linux x64 and Windows x64 only")


def ordinary(value):
    path = Path(value).absolute()
    if ".." in path.parts:
        raise ValueError("CEF paths must not contain parent traversal")
    for entry in (path, *path.parents):
        try:
            metadata = entry.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(metadata.st_mode) or getattr(metadata, "st_file_attributes", 0) & 0x400:
            raise ValueError(f"CEF paths must not traverse links/reparse points: {entry}")
    return path


def digest(path, algorithm="sha256"):
    with path.open("rb") as source:
        return hashlib.file_digest(source, algorithm).hexdigest()


def checked_archive(archive, pin):
    archive = ordinary(archive)
    if not archive.is_file() or archive.stat().st_size != pin["bytes"]:
        raise ValueError("CEF archive size does not match the pinned minimal distribution")
    if digest(archive) != pin["archive_sha256"] or digest(archive, "sha1") != pin["sha1"]:
        raise ValueError("CEF archive checksum does not match the pinned minimal distribution")
    return archive


def acquire(cache, pin, offline):
    archive = ordinary(cache / pin["archive"])
    if not archive.exists():
        if offline:
            raise ValueError(f"Offline CEF cache is missing: {archive}")
        cache.mkdir(parents=True, exist_ok=True)
        temporary = cache / (uuid.uuid4().hex + ".download")
        url = "https://cef-builds.spotifycdn.com/" + urllib.parse.quote(pin["archive"], safe="")
        print(f"Downloading pinned CEF SDK: {url}", file=sys.stderr)
        try:
            with urllib.request.urlopen(url, timeout=60) as response, temporary.open("xb") as output:
                if not response.geturl().startswith("https://"):
                    raise ValueError("CEF download redirected away from HTTPS")
                size = 0
                while block := response.read(256 * 1024):
                    size += len(block)
                    if size > pin["bytes"]:
                        raise ValueError("CEF download exceeds its pinned byte count")
                    output.write(block)
            checked_archive(temporary, pin)
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
    return checked_archive(archive, pin)


def tree_hashes(root):
    result = {}
    for index, path in enumerate(root.rglob("*")):
        if index >= MAX_MEMBERS:
            raise ValueError("CEF SDK exceeds its entry budget")
        ordinary(path)
        mode = path.lstat().st_mode
        if stat.S_ISREG(mode):
            result[path.relative_to(root).as_posix()] = digest(path)
        elif not stat.S_ISDIR(mode):
            raise ValueError("CEF SDK must contain only ordinary files and directories")
    return result


def validate_linux(sdk, pin):
    spec = importlib.util.spec_from_file_location("cef_linux_package", PROJECT / "scripts/package-linux.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.validate_sdk(sdk, pin)


def validate_windows(sdk):
    for filename in ("bootstrap.exe", "libcef.dll", "libcef.lib", "LICENSE.txt", "CREDITS.html",
                     "include/cef_version.h", "cmake/cef_variables.cmake", "CMakeLists.txt"):
        if not (sdk / filename).is_file():
            raise ValueError(f"Pinned Windows CEF SDK is incomplete: {filename}")


def extract_flat(archive, output, pin, platform_name):
    prefix = pin["archive"].removesuffix(".tar.bz2")
    seen, written = set(), set()
    expanded = 0
    with tarfile.open(archive, "r:bz2") as bundle:
        for index, member in enumerate(bundle):
            name = member.name.rstrip("/")
            path = PurePosixPath(name)
            pieces = name.split("/")
            expanded += member.size
            if (index >= MAX_MEMBERS or expanded > MAX_EXPANDED or member.size > MAX_FILE
                    or not pieces or pieces[0] != prefix or path.is_absolute()
                    or any(part in ("", ".", "..") for part in pieces)
                    or "\\" in name or ":" in name or any(ord(char) < 32 for char in name)
                    or name in seen or not (member.isfile() or member.isdir())):
                raise ValueError(f"Unsafe or excessive CEF archive member: {name}")
            seen.add(name)
            parts = pieces[1:]
            if not parts:
                continue
            if parts[0] in ("Release", "Resources"):
                parts = parts[1:]
            elif parts[0] not in ("include", "cmake", "libcef_dll") and parts != ["CMakeLists.txt"] \
                    and parts not in (["LICENSE.txt"], ["CREDITS.html"]):
                continue
            if not parts or member.isdir():
                continue
            relative = Path(*parts)
            if platform_name == "linux" and relative.as_posix() not in pin["files"]:
                continue
            identity = relative.as_posix().casefold() if platform_name == "win32" else relative.as_posix()
            if identity in written:
                raise ValueError(f"Duplicate flattened CEF SDK member: {relative}")
            written.add(identity)
            target = ordinary(output / relative)
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.extractfile(member) as source, target.open("xb") as destination:
                shutil.copyfileobj(source, destination, 256 * 1024)
            # No privileged ownership or setuid mode is installed by this build helper.
            target.chmod(0o755 if member.mode & 0o111 else 0o644)
    marker = {"type": "minimal", "name": pin["archive"], "sha1": pin["sha1"]}
    (output / "archive.json").write_text(json.dumps(marker) + "\n", encoding="utf-8")


def prepare(output, offline=False, platform_name=None, cache_root=CACHE_ROOT, environ=None, pin=None):
    platform_name = sys.platform if platform_name is None else platform_name
    pin = pin_for(platform_name) if pin is None else pin
    environ = os.environ if environ is None else environ
    output, cache = ordinary(output), ordinary(cache_root)
    if "_build" not in output.parts[:-1]:
        raise ValueError("Generated CEF SDK output must be under _build/")
    if any(name in environ for name in ("FLATPAK", "NIX_CEF_BINARY")):
        raise ValueError("CEF environment overrides bypass the verified SDK")
    override = environ.get("CEF_PATH")
    if override and platform_name == "linux":
        sdk = ordinary(override)
        if not sdk.is_absolute():
            raise ValueError("CEF_PATH must be absolute")
        validate_linux(sdk, pin)
        return sdk
    if override and ordinary(override) != output:
        raise ValueError("Windows CEF_PATH must name this build's verified cef-sdk directory")
    archive = acquire(cache, pin, offline)
    receipt = ordinary(output.with_name(output.name + ".verified.json"))
    if output.is_dir():
        try:
            hashes = tree_hashes(output)
            if platform_name == "linux":
                validate_linux(output, pin)
                return output
            saved = json.loads(receipt.read_text(encoding="utf-8"))
            validate_windows(output)
            if saved == {"archive_sha256": pin["archive_sha256"], "files": hashes}:
                return output
        except (OSError, ValueError):
            pass
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".cef-stage-", dir=output.parent) as temporary:
        stage = Path(temporary) / "sdk"
        stage.mkdir()
        extract_flat(archive, stage, pin, platform_name)
        if platform_name == "linux":
            validate_linux(stage, pin)
        else:
            validate_windows(stage)
        hashes = tree_hashes(stage)
        previous = Path(temporary) / "previous"
        if output.exists():
            output.replace(previous)
        try:
            stage.replace(output)
        except OSError:
            if previous.exists():
                previous.replace(output)
            raise
        receipt.write_text(json.dumps({"archive_sha256": pin["archive_sha256"], "files": hashes},
                                      sort_keys=True) + "\n", encoding="utf-8")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    try:
        path = prepare(args.output, args.offline)
    except (OSError, ValueError, tarfile.TarError) as error:
        print(f"CEF preparation failed: {error}", file=sys.stderr)
        return 2
    print(json.dumps({"cef_path": str(path)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
