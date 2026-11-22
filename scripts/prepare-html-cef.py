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
import time
import urllib.parse
import urllib.request
import uuid


ROOT = Path(__file__).resolve().parents[1]
PROJECT = ROOT / "plugins/midnight-viewer-html"
CACHE_ROOT = ROOT / ".tools/cef"
MAX_MEMBERS = 20000
MAX_EXPANDED = 5 * 1024**3
MAX_FILE = 2 * 1024**3
# Windows errors while endpoint protection holds a file that was just written: access
# denied (5) renaming a folder above it, and sharing (32) or lock (33) violations.
# os.replace (MoveFileEx) never reports 1175, File.Replace's code for a replaced file it
# cannot remove (see Invoke-FreshRename in the PowerShell build scripts).
FRESH_FILE_HOLDS = (5, 32, 33) if os.name == "nt" else ()


def pin_for(platform_name):
    if platform_name == "linux":
        pin = json.loads((PROJECT / "scripts/cef-linux-pinned.json").read_text())
        return {**pin, "bytes": 333351579}
    if platform_name == "win32":
        return {"archive": "cef_binary_156.0.26+g8ea0c73+chromium-156.0.8078.88_windows64_minimal.tar.bz2",
                "archive_sha256": "2b0aa070793a6bb858b737f5edf9cb01a3ef7b9adcad5efe1e08affaf250f809",
                "sha1": "15d29eea209d857c22dc163706080295aea69b5c", "bytes": 175046254}
    raise ValueError("CEF runtime builds support Linux x64 and Windows x64 only")


# Link rule of the build scripts. Folders above the checkout, above a _build
# outside it or above an input root that the caller names may be links, a subst drive
# or hidden folders: Fedora Silverblue's /home -> /var/home, a linked parent folder or
# AppData. Below such a root, and from _build down, nothing may be a link, a junction
# or another reparse point, and no path may contain '..'.

def is_link(metadata):
    """A symbolic link or any Windows reparse point, such as a junction."""
    return stat.S_ISLNK(metadata.st_mode) or bool(
        getattr(metadata, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)


def named(value):
    path = Path(value).absolute()
    if ".." in path.parts:
        raise ValueError("CEF paths must not contain parent traversal")
    return path


def unlinked(base, path):
    """`path` below `base` with no link from `base` down; missing folders are accepted."""
    current = base
    for part in path.relative_to(base).parts:
        current = current / part
        try:
            metadata = current.lstat()
        except FileNotFoundError:
            break
        if is_link(metadata):
            raise ValueError(f"CEF paths must not traverse links/reparse points: {current}")
    return path


def checkout_depth(path):
    """Index in (path, *path.parents) of the shallowest ancestor that is the checkout.

    The checkout is recognised however the path spells it. Taking the shallowest one
    means a link below the checkout that leads back to it is inspected, not trusted.
    """
    checkout = os.stat(ROOT)
    ancestors = (path, *path.parents)
    for index in reversed(range(len(ancestors))):
        try:
            if os.path.samestat(ancestors[index].stat(), checkout):
                return index
        except OSError:
            continue
    return None


def generated_output(value):
    """Generated output strictly below _build, with no link from that _build down.

    The checkout's own _build for a path in the checkout; outside it, the path's first
    _build (a custom target directory).
    """
    path = named(value)
    ancestors = (path, *path.parents)
    depth = checkout_depth(path)
    builds = [index for index in range(1, len(ancestors) if depth is None else depth)
              if os.path.normcase(ancestors[index].name) == "_build"]
    if not builds or (depth is not None and builds[-1] != depth - 1):
        raise ValueError(f"Generated CEF SDK output must be under _build/: {path}")
    for entry in ancestors[builds[-1]::-1]:
        try:
            metadata = entry.lstat()
        except FileNotFoundError:
            break
        if is_link(metadata) or not stat.S_ISDIR(metadata.st_mode):
            raise ValueError(f"CEF paths must not traverse links/reparse points or files: {entry}")
    return path


def download_cache(value):
    """The download cache inside the checkout, with no link from the checkout down.

    Stricter than the input rule on purpose: the verified archive is opened again by
    path after its checksums are compared, so the cache must stay a plain folder of
    the checkout rather than a link to a place that others may write in between. The
    checkout and the folders above it are not inspected.
    """
    path = named(value)
    depth = checkout_depth(path)
    if depth is None:
        raise ValueError(f"The CEF download cache must lie inside the checkout: {path}")
    return unlinked(path.parents[depth - 1] if depth else path, path)


def replace_fresh(source, target, timeout=2.0):
    """Rename `source` over `target`, waiting out a brief hold of freshly written files.

    Only the Windows errors in FRESH_FILE_HOLDS are retried, with a short backoff for
    about `timeout` seconds; then the first error is raised. Other errors, and every
    error on other platforms, are raised at once.
    """
    first_error, deadline, delay = None, 0.0, 0.02
    while True:
        try:
            return Path(source).replace(target)
        except OSError as error:
            if getattr(error, "winerror", None) not in FRESH_FILE_HOLDS:
                raise
            now = time.monotonic()
            if first_error is None:
                first_error, deadline = error, now + timeout
            if now >= deadline:
                break
        time.sleep(min(delay, deadline - now))
        delay = min(2 * delay, 0.2)
    raise first_error


def digest(path, algorithm="sha256"):
    with path.open("rb") as source:
        return hashlib.file_digest(source, algorithm).hexdigest()


def checked_archive(archive, pin):
    archive = named(archive)
    try:
        metadata = archive.lstat()
    except FileNotFoundError:
        metadata = None
    if metadata is not None and is_link(metadata):
        raise ValueError(f"CEF paths must not traverse links/reparse points: {archive}")
    if metadata is None or not stat.S_ISREG(metadata.st_mode) or metadata.st_size != pin["bytes"]:
        raise ValueError("CEF archive size does not match the pinned minimal distribution")
    if digest(archive) != pin["archive_sha256"] or digest(archive, "sha1") != pin["sha1"]:
        raise ValueError("CEF archive checksum does not match the pinned minimal distribution")
    return archive


def acquire(cache, pin, offline):
    archive = cache / pin["archive"]
    if not os.path.lexists(archive):
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
            # Retried: endpoint protection may still hold the download just written.
            replace_fresh(temporary, archive)
        finally:
            temporary.unlink(missing_ok=True)
    return checked_archive(archive, pin)


def tree_hashes(root):
    result = {}
    for index, path in enumerate(root.rglob("*")):
        if index >= MAX_MEMBERS:
            raise ValueError("CEF SDK exceeds its entry budget")
        metadata = path.lstat()
        if is_link(metadata):
            raise ValueError(f"CEF paths must not traverse links/reparse points: {path}")
        mode = metadata.st_mode
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
            target = unlinked(output, output / relative)
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.extractfile(member) as source, target.open("xb") as destination:
                shutil.copyfileobj(source, destination, 256 * 1024)
            # No privileged ownership or setuid mode is installed by this build helper.
            target.chmod(0o755 if member.mode & 0o111 else 0o644)
    marker = {"type": "minimal", "name": pin["archive"], "sha1": pin["sha1"]}
    (output / "archive.json").write_bytes(
        (json.dumps(marker) + "\n").encode("utf-8"))


def prepare(output, offline=False, platform_name=None, cache_root=CACHE_ROOT, environ=None, pin=None):
    platform_name = sys.platform if platform_name is None else platform_name
    pin = pin_for(platform_name) if pin is None else pin
    environ = os.environ if environ is None else environ
    output, cache = generated_output(output), download_cache(cache_root)
    if any(name in environ for name in ("FLATPAK", "NIX_CEF_BINARY")):
        raise ValueError("CEF environment overrides bypass the verified SDK")
    override = environ.get("CEF_PATH")
    if override and platform_name == "linux":
        if not Path(override).is_absolute():
            raise ValueError("CEF_PATH must be absolute")
        # The caller names the SDK root: it and the folders above it may be links, while
        # validate_linux refuses any link inside it and checks every file's digest.
        sdk = named(override)
        validate_linux(sdk, pin)
        return sdk
    if override and named(override) != output:
        raise ValueError("Windows CEF_PATH must name this build's verified cef-sdk directory")
    archive = acquire(cache, pin, offline)
    receipt = output.with_name(output.name + ".verified.json")
    if os.path.lexists(receipt) and is_link(receipt.lstat()):
        raise ValueError(f"CEF paths must not traverse links/reparse points: {receipt}")
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
            # Retried: endpoint protection may still hold files of a recent build.
            replace_fresh(output, previous)
        try:
            # Retried: endpoint protection may still hold the files just extracted.
            replace_fresh(stage, output)
        except OSError:
            if previous.exists():
                # Retried like the publication: the previous SDK returns to its place.
                replace_fresh(previous, output)
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
