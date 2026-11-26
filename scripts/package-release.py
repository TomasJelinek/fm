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
import struct
import sys
import tarfile
import tomllib
import zipfile
from pathlib import Path

REQUIRED_VIEWERS = ("midnight.viewer.basic", "midnight.viewer.source", "midnight.viewer.markdown",
                    "midnight.viewer.image", "midnight.viewer.pdf", "midnight.viewer.html")
FILESYSTEM_ABI_MINOR = 5
# The application executable: the `midnight-explorer` Cargo binary (releases up to 0.5.0
# named it `me`). Packages and runtime installations carry it under exactly this name.
APPLICATION = "midnight-explorer"


def portable_component(name):
    # Archives can be inspected/extracted on another OS. In particular a literal
    # Unix backslash or colon must never become a Windows separator or ADS path.
    reserved = name.split(".", 1)[0].upper()
    if (not name or name in (".", "..") or re.search(r'[\\/:<>"|?*\x00-\x1f]', name)
            or name.endswith((".", " ")) or reserved in ("CON", "PRN", "AUX", "NUL")
            or re.fullmatch(r"(?:COM|LPT)[1-9]", reserved)):
        raise ValueError(f"Nonportable or ambiguous release path component: {name!r}")


def is_link(metadata):
    """A symbolic link or any Windows reparse point, such as a junction."""
    return stat.S_ISLNK(metadata.st_mode) or bool(
        getattr(metadata, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)


def plain_metadata(path):
    metadata = path.lstat()
    if is_link(metadata):
        raise ValueError(f"Symlink/reparse entry is not a release input: {path}")
    if not (stat.S_ISREG(metadata.st_mode) or stat.S_ISDIR(metadata.st_mode)):
        raise ValueError(f"Special file is not a release input: {path}")
    return metadata


# Link rule of every collector and output. Folders above a caller-named
# package root, the checkout or its _build may be links: Fedora Silverblue's
# /home -> /var/home, a linked parent folder, a subst drive or an 8.3 spelling.
# Nothing below that root or below _build may be a link, a reparse point or '..'.

def real_directory(path, description):
    """A caller-named root that is itself a real directory; folders above it may be links."""
    path = Path(path).absolute()
    if ".." in path.parts or not stat.S_ISDIR(plain_metadata(path).st_mode):
        raise ValueError(f"{description} must be a real directory named without '..': {path}")
    return path


def package_member(root, source):
    """The path of an input below its package root; '..' or a path outside the root escapes."""
    root, source = Path(root).absolute(), Path(source).absolute()
    if source != root and source.is_relative_to(root):
        member = source.relative_to(root)
        if ".." not in member.parts:
            return member
    raise ValueError(f"Package input escapes its package: {source}")


def package_path(source, *, root):
    """The real path of a package input below its root, reached without links.

    The caller names the root, such as a crate's manifest directory. Only the root is
    resolved, so links above it and a root that is itself a link (a registry crate
    folder) are accepted. Every component below it must be a portable name and is
    inspected without following links, so neither a link, '..' nor a drive or stream
    name can substitute content from outside the package.
    """
    root = Path(root).absolute()
    member = package_member(root, source)
    # Check the names before any access: on Windows, joining a drive-qualified name such
    # as 'D:x' re-anchors the path on that drive, and 'LICENSE:x' names a data stream.
    for part in member.parts:
        portable_component(part)
    # Descend from the resolved root; an error names the link as the caller spelled it.
    path, given = root.resolve(strict=True), root
    for part in member.parts:
        path, given = path / part, given / part
        if is_link(path.lstat()):
            raise ValueError(f"Package input must not traverse links: {given}")
    return path


def build_output(output, repository):
    """An output directory strictly below the checkout's own _build, reached without links.

    The checkout is recognised as the same directory however the output spells it, so
    it may be reached through links. _build and every existing folder below it down to
    the output must be a real directory: no link redirects what the caller replaces.
    """
    output = Path(output).absolute()
    if ".." in output.parts:
        raise ValueError(f"Generated output must not contain '..': {output}")
    checkout = os.stat(repository)

    def is_checkout(path):
        try:
            return os.path.samestat(path.stat(), checkout)
        except OSError:
            return False

    # The shallowest ancestor that is the checkout ends the walk, so a link below the
    # checkout that leads back to it is still inspected, never mistaken for the checkout.
    ancestors = (output, *output.parents)
    depth = next((index for index in reversed(range(len(ancestors))) if is_checkout(ancestors[index])), 0)
    below = ancestors[:depth]
    if len(below) < 2 or os.path.normcase(below[-1].name) != "_build":
        raise ValueError(f"Generated output must be below the repository's _build: {output}")
    for path in below:
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            continue
        if is_link(metadata) or not stat.S_ISDIR(metadata.st_mode):
            raise ValueError(f"Generated output must not traverse links or files: {path}")
    return output


def plain_tree(root, maximum=4096):
    """Every regular file below a real root directory, walked without following links."""
    root = Path(root).absolute()
    portable_component(root.name)
    root = real_directory(root, "Release tree")
    pending, files, count = [root], [], 0
    while pending:
        directory = pending.pop()
        for path in sorted(directory.iterdir()):
            portable_component(path.name)
            count += 1
            if count > maximum:
                raise ValueError(f"Release tree entry limit exceeded: {root}")
            # Links, reparse points and special files are refused, never followed.
            if stat.S_ISDIR(plain_metadata(path).st_mode):
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


def copy_viewer_packages(stage, viewers, target, required_ids=(), *, validate_only=False):
    """Copy independently built packages, validating all inputs before staging any plugin."""
    viewers = real_directory(viewers, "Viewer package root")
    if Path(stage).resolve().is_relative_to(viewers.resolve()):
        raise ValueError("Release staging cannot be inside its viewer input tree")
    packages, identities = [], set()
    entries = sorted(viewers.iterdir())
    if not entries or len(entries) > 256:
        raise ValueError("Expected 1..256 independently built viewer packages")
    for package in entries:
        files = plain_tree(package)
        manifests = [name for name in ("plugin.toml", "image-viewer.json", "web-viewer.json")
                     if package / name in files]
        if len(manifests) != 1:
            raise ValueError("A viewer package must have exactly one entry manifest")
        if package / "web-viewer.json" in files:
            manifest = image_viewer_manifest(package, files, target, web=True)
            validate_web_runtime(package, files, manifest, target)
            worker_identity(manifest, identities)
            packages.append((package, manifest, files, manifest["executable"]))
            continue
        if package / "image-viewer.json" in files:
            manifest = image_viewer_manifest(package, files, target)
            worker_identity(manifest, identities)
            packages.append((package, manifest, files, manifest["executable"]))
            continue
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
        packages.append((package, manifest, files, None))
    missing = set(required_ids) - identities
    if missing:
        raise ValueError("Missing required viewer packages: " + ", ".join(sorted(missing)))
    if validate_only:
        return [manifest for _, manifest, _, _ in packages]
    inventory = []
    for package, manifest, files, worker in packages:
        web = package / "web-viewer.json" in files
        copied = []
        for source in files:
            relative = Path("viewers") / package.name / source.relative_to(package)
            destination = stage / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            with plain_file(source) as original, destination.open("xb") as output:
                shutil.copyfileobj(original, output)
            executable = (worker is not None and source == package / worker) or (
                web and source.parent == package
                and (source.name == "chrome-sandbox" or source.suffix.lower() == ".exe"))
            destination.chmod(0o755 if executable else 0o644)
            copied.append({"path": relative.as_posix(), "bytes": destination.stat().st_size,
                           "sha256": sha256_file(destination), "executable": executable})
        fingerprint = hashlib.sha256(json.dumps(copied, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        item = {"id": manifest["id"], "version": manifest["version"], "target": target,
                "package_sha256": fingerprint, "files": copied}
        if worker:
            item.update(kind="web-viewer-worker" if web else "image-viewer-worker",
                        protocol=manifest["protocol"],
                        entry_executable=worker, extensions=manifest["extensions"])
        else:
            item.update(abi_major=manifest["abi_major"], entry_library=manifest["entry_library"],
                        models=manifest["models"])
        inventory.append(item)
    return inventory


def image_viewer_manifest(package, files, target, *, web=False):
    if package / "plugin.toml" in files:
        raise ValueError("A viewer package cannot contain both native and process entry manifests")
    filename = "web-viewer.json" if web else "image-viewer.json"
    manifest = read_worker_manifest(package / filename, json.loads)
    fields = {"schema", "protocol", "id", "name", "version", "target", "executable", "extensions"}
    extensions = manifest.get("extensions")
    if (set(manifest) != fields or type(manifest.get("schema")) is not int
            or manifest["schema"] != 1 or type(manifest.get("protocol")) is not int
            or manifest["protocol"] not in ((1,) if web else (1, 2)) or manifest.get("target") != target
            or not isinstance(manifest.get("name"), str) or not 1 <= len(manifest["name"]) <= 128
            or not isinstance(extensions, list) or not 1 <= len(extensions) <= 64
            or any(not isinstance(value, str) or not re.fullmatch(r"[a-z0-9]{1,32}", value)
                   for value in extensions) or len(set(extensions)) != len(extensions)):
        raise ValueError("Incompatible image viewer manifest")
    executable = worker_executable(manifest.get("executable"), target)
    # The HTML provider's own licence is LICENSE; its LICENSE.txt is CEF's.
    for required in (filename, executable, *(('LICENSE', 'LICENSE.txt', 'THIRD_PARTY.md') if web else
                                           ('LICENSE', 'THIRD-PARTY.md'))):
        if package / required not in files:
            raise ValueError(f"Image viewer package missing required file {required}: {package}")
    return manifest


def validate_web_runtime(package, files, manifest, target):
    required = {"runtime-files.json", "LICENSE.txt", "CREDITS.html", "THIRD_PARTY.md",
                "dependency-metadata.json", "icudtl.dat", "resources.pak",
                "chrome_100_percent.pak", "chrome_200_percent.pak", "v8_context_snapshot.bin",
                "vk_swiftshader_icd.json"}
    if manifest["id"] == "midnight.viewer.html":
        required.add("locales/en-US.pak")
    if target.endswith("windows-msvc"):
        required.update(("libcef.dll", "chrome_elf.dll", "dxcompiler.dll", "vk_swiftshader.dll",
                         "vulkan-1.dll", str(Path(manifest["executable"]).with_suffix(".dll"))))
    else:
        required.update(("libcef.so", "libvk_swiftshader.so", "libvulkan.so.1", "chrome-sandbox"))
    if (any(package / name not in files for name in required)
            or not any(file.is_relative_to(package / "locales") and file.suffix == ".pak"
                       for file in files)
            or not any(file.parent != package and file.is_relative_to(package / "notices")
                       for file in files)):
        raise ValueError("Incomplete HTML viewer runtime or CEF attribution")

    def unique_fields(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("Duplicate CEF runtime inventory field")
            result[key] = value
        return result

    with plain_file(package / "runtime-files.json") as stream:
        encoded = stream.read(4 * 1024 * 1024 + 1)
    if len(encoded) > 4 * 1024 * 1024:
        raise ValueError("CEF runtime inventory exceeds 4 MiB")
    inventory = json.loads(encoded.decode("utf-8-sig"), object_pairs_hook=unique_fields)
    if (not isinstance(inventory, dict) or set(inventory) != {"cef", "archive_sha256", "files"}
            or not isinstance(inventory.get("cef"), str) or not 1 <= len(inventory["cef"]) <= 128
            or not isinstance(inventory.get("archive_sha256"), str)
            or not re.fullmatch(r"[0-9a-f]{64}", inventory["archive_sha256"])
            or not isinstance(inventory.get("files"), list)
            or not 1 <= len(inventory["files"]) <= 4096):
        raise ValueError("Invalid CEF runtime inventory")
    expected = {file.relative_to(package).as_posix(): file for file in files
                if file != package / "runtime-files.json"}
    indexed = set()
    for entry in inventory["files"]:
        if (not isinstance(entry, dict) or set(entry) != {"path", "bytes", "sha256"}
                or not isinstance(entry.get("path"), str) or type(entry.get("bytes")) is not int
                or entry["bytes"] < 0 or not isinstance(entry.get("sha256"), str)
                or not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"])):
            raise ValueError("Invalid CEF runtime file entry")
        name = entry["path"]
        for component in name.split("/"):
            portable_component(component)
        if name in indexed or name not in expected:
            raise ValueError("Duplicate or unexpected CEF runtime file")
        indexed.add(name)
        path = expected[name]
        if path.stat().st_size != entry["bytes"] or sha256_file(path) != entry["sha256"]:
            raise ValueError("CEF runtime file size or SHA-256 mismatch: " + name)
    if indexed != set(expected):
        raise ValueError("CEF runtime inventory does not cover every package file")


def worker_input(stage, source):
    source = real_directory(source, "Worker input")
    if Path(stage).resolve().is_relative_to(source.resolve()):
        raise ValueError("Release staging cannot be inside its worker input")
    return source


def worker_identity(manifest, identities, limit=128):
    identity, version = manifest.get("id"), manifest.get("version")
    if (not isinstance(identity, str) or not re.fullmatch(r"[a-zA-Z0-9._-]{1," + str(limit) + "}", identity)
            or identity in (".", "..") or identity in identities
            or not isinstance(version, str) or not 1 <= len(version) <= 128):
        raise ValueError("Invalid or duplicate worker identity/version")
    identities.add(identity)


def worker_executable(value, target):
    if not isinstance(value, str) or not 1 <= len(value) <= 128:
        raise ValueError("Worker executable must be a filename")
    portable_component(value)
    if target.endswith("windows-msvc") != value.lower().endswith(".exe"):
        raise ValueError("Worker executable suffix does not match the target")
    return value


def read_worker_manifest(path, decoder):
    with plain_file(path) as stream:
        source = stream.read(16 * 1024 + 1)
    if len(source) > 16 * 1024:
        raise ValueError("Worker manifest exceeds 16 KiB")
    result = decoder(source.decode("utf-8"))
    if not isinstance(result, dict):
        raise ValueError("Worker manifest must be a mapping")
    return result


def copy_worker_files(stage, source_root, prefix, files, executables):
    inventory = []
    for source in files:
        relative = Path(prefix) / source.relative_to(source_root)
        destination = stage / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        with plain_file(source) as original, destination.open("xb") as output:
            shutil.copyfileobj(original, output)
        executable = source in executables
        destination.chmod(0o755 if executable else 0o644)
        inventory.append({"path": relative.as_posix(), "bytes": destination.stat().st_size,
                          "sha256": sha256_file(destination), "executable": executable})
    digest = hashlib.sha256(json.dumps(inventory, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return inventory, digest


# The filesystem worker protocols the host accepts: filesystem-api's `Protocol`
# (`TryFrom<u16>`); test_worker_packages checks that both agree.
FILESYSTEM_WORKER_PROTOCOLS = (1, 2, 3, 4)


def copy_filesystem_packages(stage, source, target, require_native=False, require_network=False,
                             *, require_archive=False, validate_only=False):
    source = worker_input(stage, source)
    packages, identities = [], set()
    entries = sorted(source.iterdir())
    if not 1 <= len(entries) <= 64:
        raise ValueError("Expected 1..64 independent filesystem packages")
    fields = {"schema_version", "id", "kind", "version", "entry_executable", "target", "protocol", "formats"}
    for package in entries:
        files = plain_tree(package)
        manifest = read_worker_manifest(package / "plugin.toml", tomllib.loads)
        worker_identity(manifest, identities)
        if manifest.get("kind") in ("filesystem-native", "filesystem-network"):
            library = library_filesystem_manifest(manifest, package, files, target)
            packages.append((package, manifest, files, library))
            continue
        formats = manifest.get("formats")
        if (set(manifest) not in (fields, fields | {"stable_entry_ids"})
                or type(manifest.get("stable_entry_ids", False)) is not bool
                or type(manifest.get("schema_version")) is not int
                or manifest["schema_version"] != 1 or manifest.get("kind") != "filesystem-worker"
                or manifest.get("target") != target or type(manifest.get("protocol")) is not int
                or manifest["protocol"] not in FILESYSTEM_WORKER_PROTOCOLS or not isinstance(formats, list) or not 1 <= len(formats) <= 16
                or any(value not in ("zip", "tar", "gzip", "tar-gzip", "7z", "xz", "tar-xz",
                                     "bzip2", "tar-bzip2", "zstd", "tar-zstd") for value in formats)):
            raise ValueError("Incompatible filesystem worker manifest")
        executable = worker_executable(manifest.get("entry_executable"), target)
        if (package / executable not in files or package / "THIRD-PARTY.md" not in files
                or not any(file.is_relative_to(package / "licenses") for file in files)):
            raise ValueError("Filesystem package is missing its executable or attribution")
        packages.append((package, manifest, files, executable))
    native_count = sum(manifest["kind"] == "filesystem-native" for _, manifest, _, _ in packages)
    if native_count > 1 or (require_native and native_count != 1):
        raise ValueError("Release requires exactly one native filesystem provider for its platform")
    network_count = sum(manifest["kind"] == "filesystem-network" for _, manifest, _, _ in packages)
    if network_count > 1 or (require_network and network_count != 1):
        raise ValueError("Release requires exactly one network filesystem provider for its platform")
    if require_archive and not any(manifest["kind"] == "filesystem-worker"
                                   and manifest["id"] == "midnight.fs.archive"
                                   for _, manifest, _, _ in packages):
        raise ValueError("Release requires the midnight.fs.archive filesystem provider")
    if validate_only:
        return [manifest for _, manifest, _, _ in packages]
    inventory = []
    for package, manifest, files, executable in packages:
        standalone = {package / executable} if manifest["kind"] == "filesystem-worker" else set()
        copied, digest = copy_worker_files(stage, package, Path("filesystems") / package.name,
                                           files, standalone)
        inventory.append({**manifest, "package_sha256": digest, "files": copied})
    return inventory


def library_filesystem_manifest(manifest, package, files, target):
    fields = {"schema_version", "id", "kind", "version", "entry_library", "target", "protocol",
              "abi_major", "minimum_host_minor", "capabilities", "address_encoding"}
    network = manifest["kind"] == "filesystem-network"
    discovery = {"browse", "stat", "volumes", "activate", "refresh", "resolve-text"}
    supported = (discovery if network else
                 {"browse", "read", "stat", "volumes", "storage", "native-address", "volume-details",
                  "prepared-rename"})
    if network and "-linux-" in target and manifest.get("id") == "network.linux":
        supported = supported | {"sign-in"}
    capabilities = manifest.get("capabilities")
    version = manifest.get("version")
    minimum_host_minor = manifest.get("minimum_host_minor")
    minimum_minor = 2 if network else 0
    encoding = "opaque" if network else "windows-utf16le" if "windows" in target else "unix-bytes"
    required = discovery if network else {"native-address"}
    if (set(manifest) != fields or type(manifest.get("schema_version")) is not int
            or manifest["schema_version"] != 1 or manifest.get("target") != target
            or not isinstance(version, str) or not 1 <= len(version.encode("utf-8")) <= 64
            or type(manifest.get("protocol")) is not int or manifest["protocol"] != 1
            or type(manifest.get("abi_major")) is not int or manifest["abi_major"] != 1
            or type(minimum_host_minor) is not int
            or not minimum_minor <= minimum_host_minor <= FILESYSTEM_ABI_MINOR
            or manifest.get("address_encoding") != encoding
            or not isinstance(capabilities, list) or not 1 <= len(capabilities) <= len(supported)
            or any(not isinstance(value, str) or value not in supported for value in capabilities)
            or len(set(capabilities)) != len(capabilities) or not required.issubset(capabilities)
            or ("prepared-rename" in capabilities and minimum_host_minor < 3)
            or ("sign-in" in capabilities and minimum_host_minor < 5)):
        raise ValueError("Incompatible library filesystem manifest")
    library = manifest.get("entry_library")
    if not isinstance(library, str):
        raise ValueError("Filesystem entry must be a library filename")
    portable_component(library)
    suffix = ".dll" if "windows" in target else ".so"
    if (not library.endswith(suffix) or package / library not in files
            or package / "THIRD-PARTY.md" not in files):
        raise ValueError("Filesystem package lacks its library or embedded notices")
    return library


def copy_thumbnail_packages(stage, source, target, *, require_media=False, validate_only=False):
    source = worker_input(stage, source)
    files = plain_tree(source)
    manifests = [file for file in files if file.parent == source and file.suffix == ".json"]
    if not 1 <= len(manifests) <= 128 or len(list(source.iterdir())) > 128:
        raise ValueError("Thumbnail package must fit the host's discovery limit")
    providers, identities, executables = [], set(), set()
    fields = {"protocol", "id", "version", "executable", "extensions", "video"}
    for path in manifests:
        manifest = read_worker_manifest(path, json.loads)
        worker_identity(manifest, identities, limit=64)
        extensions = manifest.get("extensions")
        if (set(manifest) != fields or type(manifest.get("protocol")) is not int
                or manifest["protocol"] != 1 or type(manifest.get("video")) is not bool
                or not isinstance(extensions, list) or not 1 <= len(extensions) <= 32
                or any(not isinstance(value, str) or not re.fullmatch(r"[a-zA-Z0-9_-]{1,16}", value)
                       for value in extensions)
                or not re.fullmatch(r"[a-zA-Z0-9._-]{1,128}", manifest["version"])):
            raise ValueError("Incompatible thumbnail worker manifest")
        if (manifest["version"] in (".", "..")
                or not isinstance(manifest.get("executable"), str)
                or not re.fullmatch(r"[a-zA-Z0-9._-]{1,128}", manifest["executable"])):
            raise ValueError("Invalid thumbnail worker version or executable")
        executable = source / worker_executable(manifest.get("executable"), target)
        notices = source / "notices" / executable.stem
        if executable not in files or not any(file.is_relative_to(notices) for file in files):
            raise ValueError("Thumbnail provider is missing its executable or attribution")
        executables.add(executable)
        providers.append(manifest)
    if require_media and {provider["video"] for provider in providers} != {False, True}:
        raise ValueError("Standard packages require image and video thumbnailers")
    # The bundled FFmpeg runtime must carry its matching source and licence;
    # independent thumbnail workers are free to use a different implementation.
    ffmpeg = ("avcodec-63.dll", "avformat-63.dll", "avutil-61.dll", "swscale-10.dll") if target.endswith("windows-msvc") else (
        "libavcodec.so.63", "libavformat.so.63", "libavutil.so.61", "libswscale.so.10")
    playback_name = ("midnight-video-playback.exe" if target.endswith("windows-msvc")
                     else "midnight-video-playback")
    playback = source / worker_executable(playback_name, target)
    # The platform's decoders (Media Foundation on Windows; the distribution's FFmpeg, then
    # GStreamer, on Linux) run in two helpers without the bundled FFmpeg. Windows packages
    # always carry them; on Linux they are optional, but come as a pair.
    suffix = ".exe" if target.endswith("windows-msvc") else ""
    platform = [source / worker_executable(name + suffix, target)
                for name in ("midnight-thumbnail-video-system", "midnight-video-playback-system")]
    has_playback_profile = any(
        provider["id"] == "midnight-video"
        and provider["version"] == "ffmpeg-9.0.2-playback-r4"
        for provider in providers)
    if has_playback_profile or playback in files or any(path in files for path in platform):
        resampler = ("swresample-7.dll" if target.endswith("windows-msvc")
                     else "libswresample.so.7")
        if not has_playback_profile or playback not in files or source / resampler not in files:
            raise ValueError("Video playback requires the matching worker and shared FFmpeg profile")
        installed = [path in files for path in platform]
        if any(installed) != all(installed) or (suffix and not all(installed)):
            raise ValueError("Video playback requires both platform decoder helpers")
        ffmpeg += (resampler,)
        executables.add(playback)
        executables.update(path for path in platform if path in files)
    if (any(provider["id"] == "midnight-video" for provider in providers)
            or any(source / name in files for name in ffmpeg)):
        required = [source / name for name in ffmpeg]
        required.extend(source / "notices/ffmpeg-9.0.2" / name for name in (
            "ffmpeg-9.0.2.tar.xz", "COPYING.LGPLv2.1", "build-options.txt", "config.h", "config.mak"))
        if any(path not in files for path in required):
            raise ValueError("Incomplete FFmpeg runtime, corresponding source or build notices")
    if validate_only:
        return {"providers": providers, "target": target}
    copied, digest = copy_worker_files(stage, source, "thumbnailers", files, executables)
    return {"providers": providers, "target": target, "package_sha256": digest, "files": copied}


def validate_runtime_installation(source, target):
    """Check the normal provider graph without creating staging or deployment files."""
    source = real_directory(source, "Runtime installation")
    suffix = ".exe" if target.endswith("windows-msvc") else ""
    for name in (APPLICATION + suffix, "midnight-elevated-listing" + suffix):
        if not stat.S_ISREG(plain_metadata(source / name).st_mode):
            raise ValueError("Normal builds require an ordinary executable: " + name)
    viewers = copy_viewer_packages(source, source / "viewers", target,
                                   required_ids=REQUIRED_VIEWERS, validate_only=True)
    filesystems = copy_filesystem_packages(source, source / "filesystems", target,
                                           require_native=True, require_network=True,
                                           require_archive=True, validate_only=True)
    thumbnails = copy_thumbnail_packages(source, source / "thumbnailers", target,
                                         require_media=True, validate_only=True)
    return {"viewers": [viewer["id"] for viewer in viewers],
            "filesystems": [filesystem["id"] for filesystem in filesystems],
            "thumbnailers": [provider["id"] for provider in thumbnails["providers"]]}


def vendored(package):
    marker = (package.get("metadata") or {}).get("midnight")
    return isinstance(marker, dict) and marker.get("vendored-third-party") is True


def dependency_packages(metadata, dependency_list, *, required_root="midnight-explorer",
                        include_local=False):
    """Use the application's tree, not metadata's workspace-wide feature union."""
    selected = set()
    for line in dependency_list.splitlines():
        if not line.strip():
            continue
        match = re.fullmatch(r"(\S+) v(\S+)(?: .*)?", line.strip())
        if not match:
            raise ValueError(f"Malformed dependency line (use --prefix none): {line!r}")
        selected.add(match.groups())
    if required_root and not any(name == required_root for name, _ in selected):
        raise ValueError(f"Dependency list must contain {required_root}")
    packages = {}
    for package in metadata["packages"]:
        key = (package["name"], package["version"])
        if key in selected:
            if key in packages:
                raise ValueError(f"Ambiguous dependency source: {key}")
            packages[key] = package
    if selected - packages.keys():
        raise ValueError(f"Dependencies missing from metadata: {selected - packages.keys()}")
    return [package for _, package in sorted(packages.items())
            if include_local or package["source"]
            or vendored(package)]


def license_files(package):
    """A crate's declared license file and its license-named files, found without links.

    Entries of the crate directory named like a license or notice are collected, and a
    directory so named contributes every file below it. A link, reparse point or special
    file among them is refused, not followed; package_path checks the declared file.
    """
    source = Path(package["manifest_path"]).parent
    files = set()
    if package.get("license_file"):
        files.add(source / package["license_file"])
    for entry in source.iterdir():
        name = entry.name.lower()
        if name.startswith(("license", "licence", "copying", "copyright", "notice")):
            if stat.S_ISDIR(plain_metadata(entry).st_mode):
                files.update(plain_tree(entry, maximum=65536))
            else:
                files.add(entry)
    return source, sorted(files)


def copy_package_file(source, destination, *, root):
    """Copy a package input below its root to the same relative path below destination."""
    path = package_path(source, root=root)
    target = Path(destination) / package_member(root, source)
    target.parent.mkdir(parents=True, exist_ok=True)
    with plain_file(path) as original, target.open("xb") as output:
        shutil.copyfileobj(original, output)


def add_notices(stage, metadata, dependency_list, rust_docs, overrides):
    inventory = []
    missing = []
    for package in dependency_packages(metadata, dependency_list):
        name = f'{package["name"]}-{package["version"]}'
        portable_component(name)
        source, files = license_files(package)
        if not files and overrides:
            source = Path(overrides) / name
            files = plain_tree(source, maximum=65536) if os.path.lexists(source) else []
        if not files:
            missing.append(name)
        for path in files:
            copy_package_file(path, stage / "licenses" / name, root=source)
        inventory.append({key: package.get(key) for key in
                          ("name", "version", "license", "repository", "source", "authors")})
    if missing:
        raise ValueError("Missing dependency license files: " + ", ".join(missing))
    rust_target = stage / "licenses" / "rust"
    rust_target.mkdir(parents=True)
    copy_package_file(Path(rust_docs) / "COPYRIGHT-library.html", rust_target, root=rust_docs)
    project_docs = Path(__file__).resolve().parents[1] / "docs"
    for author in ("folke", "joshdick", "rose-pine", "catppuccin"):
        relative = Path("licenses/themes") / author / "LICENSE"
        destination = stage / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(project_docs / relative, destination)
    shutil.copyfile(project_docs / "THEME_PROVENANCE.md", stage / "THEME_PROVENANCE.md")
    (stage / "THIRD-PARTY.json").write_text(
        json.dumps(inventory, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (stage / "THIRD-PARTY.txt").write_text(
        "Dependency names, versions and declared licenses are in THIRD-PARTY.json.\n"
        "Original license and copyright notices are retained under licenses/.\n"
        "The inventory conservatively includes build dependencies, not just linked code.\n"
        "Rust standard-library notices are in licenses/rust/.\n"
        "Adapted palette sources are in THEME_PROVENANCE.md; licenses are in licenses/themes/.\n"
        "Midnight Explorer uses Slint under Slint's GPL-3.0-only option.\n"
        "The About dialog and the release download page display Slint's attribution.\n"
        "Midnight Explorer and its plugins are licensed under GPL-3.0-or-later; see the LICENSE file.\n"
        "These notices cover third-party components only.\n",
        encoding="utf-8")


# The program's license is the repository's LICENSE (GNU GPL version 3). Windows
# packages name it LICENSE.txt, so it opens on a double-click.
PROJECT_DOCUMENTS = ("SECURITY.md", "docs/SYSTEM_REQUIREMENTS.md", "docs/KEYBOARD_SHORTCUTS.md",
                     "docs/FEATURE_STATUS.md", "docs/ASSET_PROVENANCE.md")
SOURCE_REPOSITORY = "https://github.com/TomasJelinek/fm"


def project_license_name(target):
    return "LICENSE.txt" if target.endswith("windows-msvc") else "LICENSE"


def copy_project_documents(stage, root, target):
    """The license, the project README and the user documents every archive carries."""
    shutil.copyfile(root / "LICENSE", stage / project_license_name(target))
    shutil.copyfile(root / "README.md", stage / "README.md")
    for relative in PROJECT_DOCUMENTS:
        shutil.copyfile(root / relative, stage / Path(relative).name)


def check_release_readme(readme, version):
    """The run instructions name this version and where its corresponding source is."""
    if not readme.startswith(f"Midnight Explorer {version} "):
        raise ValueError(f"The release README must begin with 'Midnight Explorer {version} '")
    if f"{SOURCE_REPOSITORY}/tree/v{version}" not in readme:
        raise ValueError(f"The release README must name its source, {SOURCE_REPOSITORY}/tree/v{version}")


def release_debug_data(stream):
    """Inspect PE/ELF headers without loading image contents or changing signed files."""
    def read_at(offset, size):
        stream.seek(offset)
        result = stream.read(size)
        if len(result) != size:
            raise ValueError("Truncated executable headers")
        return result

    magic = stream.read(4)
    if magic.startswith(b"MZ"):
        pe, = struct.unpack("<I", read_at(60, 4))
        if read_at(pe, 4) != b"PE\0\0":
            raise ValueError("Invalid PE signature")
        sections, _, symbols, count, opt_size = struct.unpack("<HIIIH", read_at(pe + 6, 16))
        if symbols and count:
            return "COFF symbol table"
        optional = read_at(pe + 24, opt_size)
        directories = {b"\x0b\x01": 96, b"\x0b\x02": 112}.get(optional[:2])
        if directories is None:
            raise ValueError("Invalid PE optional header")
        debug_rva, debug_size = (0, 0)
        if len(optional) >= directories + 56 and struct.unpack_from("<I", optional, directories - 4)[0] > 6:
            debug_rva, debug_size = struct.unpack_from("<II", optional, directories + 48)
        for index in range(sections):
            section = read_at(pe + 24 + opt_size + 40 * index, 40)
            name = section[:8].rstrip(b"\0")
            if name.startswith((b".debug", b".zdebug", b".stab")):
                return name.decode("ascii", errors="replace")
            address, raw_size, raw = struct.unpack_from("<III", section, 12)
            if debug_size and address <= debug_rva < address + raw_size:
                if debug_size > raw_size - (debug_rva - address) or debug_size % 28:
                    raise ValueError("Invalid PE debug directory")
                for entry in range(0, debug_size, 28):
                    kind, size, _, data = struct.unpack("<IIII", read_at(raw + debug_rva - address + entry + 12, 16))
                    if kind in (1, 17):  # COFF debug info or embedded portable PDB
                        return "embedded PE debugging data"
                    # RSDS/NB10 records identify an external PDB. Older CodeView formats
                    # can carry the actual symbol information inside the executable.
                    if kind == 2 and size and (size < 4 or read_at(data, 4) not in (b"RSDS", b"NB10")):
                        return "embedded CodeView symbols"
    elif magic == b"\x7fELF":
        elf_class, encoding = read_at(4, 2)
        if elf_class not in (1, 2) or encoding not in (1, 2):
            raise ValueError("Invalid ELF class or encoding")
        endian = "<" if encoding == 1 else ">"
        wide = elf_class == 2
        word = "Q" if wide else "I"
        table, = struct.unpack(endian + word, read_at(40 if wide else 32, 8 if wide else 4))
        size, count, strings = struct.unpack(endian + "HHH", read_at(58 if wide else 46, 6))
        if not table:
            return None  # An ELF image may omit its section headers entirely.
        layout = struct.Struct(endian + ("IIQQQQIIQQ" if wide else "IIIIIIIIII"))
        if size < layout.size:
            raise ValueError("Invalid ELF section header size")

        def section(index):
            return layout.unpack(read_at(table + size * index, layout.size))

        count = count or section(0)[5]
        strings = section(0)[6] if strings == 0xffff else strings
        if strings >= count:
            raise ValueError("Invalid ELF string table index")
        names_offset, names_size = section(strings)[4:6]
        for index in range(count):
            name, kind = section(index)[:2]
            if kind == 2:  # SHT_SYMTAB; dynamic linking still needs SHT_DYNSYM.
                return "ELF symbol table"
            if name < names_size:
                label = read_at(names_offset + name, min(32, names_size - name)).split(b"\0", 1)[0]
                if label.startswith((b".debug", b".zdebug", b".stab", b".gdb_index", b".gnu_debugdata")):
                    return label.decode("ascii", errors="replace")
    return None


def check_release_payload(stage, files):
    for path in files:
        relative = path.relative_to(stage)
        if (relative.suffix.lower() in {".pdb", ".ipdb", ".dbg", ".debug", ".sym", ".dwo", ".dwp", ".ilk", ".map"}
                or any(part.lower().endswith(".dsym") for part in relative.parts)):
            raise ValueError(f"Debug sidecar in release: {relative}")
        if path.name == "web-viewer.json" and path.parent.parent == stage / "viewers":
            manifest = read_worker_manifest(path, json.loads)
            if manifest.get("id") == "midnight.viewer.html":
                locales = path.parent / "locales"
                if {file for file in files if file.is_relative_to(locales)} != {locales / "en-US.pak"}:
                    raise ValueError("The bundled HTML viewer must ship exactly locales/en-US.pak")
        with plain_file(path) as stream:
            try:
                debug = release_debug_data(stream)
            except ValueError as error:
                raise ValueError(f"{relative}: {error}") from error
        if debug:
            raise ValueError(f"{relative}: {debug} in release")


def write_archive(stage, archive, binary_name, archive_format, executables=()):
    binary_path = f"{stage.name}/{binary_name}"
    files = plain_tree(stage, maximum=65536)
    check_release_payload(stage, files)
    executable_paths = {binary_path}
    for relative in executables:
        relative = Path(relative)
        if relative.is_absolute() or not relative.parts:
            raise ValueError("Worker executable must be stage-relative")
        for part in relative.parts:
            portable_component(part)
        if stage / relative not in files:
            raise ValueError("Declared worker executable is absent from staging")
        executable_paths.add(f"{stage.name}/{relative.as_posix()}")

    def safe_tar_member(member):
        if not (member.isdir() or member.isfile()):
            raise ValueError(f"Unexpected archive entry type: {member.name}")
        member.mode = 0o755 if member.isdir() or member.name in executable_paths else 0o644
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
                mode = 0o755 if info.filename in executable_paths else 0o644
                info.external_attr = (stat.S_IFREG | mode) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                with plain_file(path) as source, bundle.open(info, "w") as target:
                    shutil.copyfileobj(source, target)
    else:
        with tarfile.open(archive, "x:gz", compresslevel=9) as bundle:
            bundle.add(stage, arcname=stage.name, filter=safe_tar_member)


def application_executable(binary, target):
    """The release binary must be the application under its released name."""
    name = APPLICATION + (".exe" if target.endswith("windows-msvc") else "")
    if Path(binary).name != name:
        raise ValueError(f"The application executable must be named {name}: {Path(binary).name}")
    return name


def authorization_helper(binary, target):
    name = "midnight-elevated-listing.exe" if target.endswith("windows-msvc") else "midnight-elevated-listing"
    source = (Path(binary).parent / name).absolute()
    metadata = plain_metadata(source)
    if ".." in source.parts or not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 512 * 1024 * 1024:
        raise ValueError("Authorization helper must be a regular, bounded, unredirected sibling executable")
    return name, source, metadata


# The record that crates/midnight-explorer/src/main.rs keeps in the executable: the
# SHA-256 that build.rs took from MIDNIGHT_ELEVATED_LISTING, empty when it was unset.
HELPER_DIGEST_RECORD = re.compile(rb"MIDNIGHT_AUTHORIZATION_HELPER_SHA256=([0-9a-f]{64})?;")


def recorded_helper_digest(executable):
    """The authorization helper digest recorded in the executable's bytes, or None."""
    recorded = {match.group(1) for match in HELPER_DIGEST_RECORD.finditer(executable)}
    if len(recorded) != 1:
        raise ValueError("The application executable must carry exactly one authorization helper "
                         f"digest record, found {len(recorded)}")
    digest = recorded.pop()
    return digest.decode("ascii") if digest else None


def check_authorization_helper_digest(binary, helper):
    """A release ships exactly the helper whose digest its executable records."""
    with plain_file(binary) as stream:
        recorded = recorded_helper_digest(stream.read())
    if recorded is None:
        raise ValueError("The application executable records no authorization helper digest; "
                         "build it with MIDNIGHT_ELEVATED_LISTING naming the helper (RELEASE_BUILD.md)")
    actual = sha256_file(helper)
    if recorded != actual:
        raise ValueError(f"The application executable records authorization helper {recorded}, "
                         f"but the packaged helper is {actual}")
    return actual


HTML_SANDBOX_INSTALLER = "install-html-viewer-apparmor.sh"


def copy_html_sandbox_installer(stage, root, target):
    """Linux packages carry the one-time AppArmor step for the HTML viewer's Chromium sandbox."""
    if "-linux-" not in target:
        return None
    destination = stage / HTML_SANDBOX_INSTALLER
    shutil.copyfile(root / "scripts" / HTML_SANDBOX_INSTALLER, destination)
    destination.chmod(0o755)
    return HTML_SANDBOX_INSTALLER


def copy_authorization_helper(stage, binary, target):
    name, source, metadata = authorization_helper(binary, target)
    destination = stage / name
    shutil.copyfile(source, destination)
    destination.chmod(0o755)
    return {"path": name, "bytes": metadata.st_size, "sha256": sha256_file(destination), "executable": True}


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "validate-filesystems":
        # CI's release job: the filesystem packages this run built must pass the same
        # checks as release packaging and deployment (manifest kind, target, protocol).
        parser = argparse.ArgumentParser(description="Validate native, network and archive packages")
        parser.add_argument("--source", required=True, type=Path)
        parser.add_argument("--target", required=True)
        args = parser.parse_args(sys.argv[2:])
        stage = args.source.resolve().parent / "filesystem-validation-stage"
        packages = copy_filesystem_packages(stage, args.source, args.target, require_native=True,
                                            require_network=True, require_archive=True,
                                            validate_only=True)
        print(json.dumps({"filesystems": len(packages)}))
        return
    if len(sys.argv) > 1 and sys.argv[1] == "validate-runtime":
        parser = argparse.ArgumentParser(description="Validate every required runtime module")
        parser.add_argument("--source", required=True, type=Path)
        parser.add_argument("--target", required=True)
        args = parser.parse_args(sys.argv[2:])
        print(json.dumps(validate_runtime_installation(args.source, args.target)))
        return
    if len(sys.argv) > 1 and sys.argv[1] == "check-authorization-helper":
        # CI's release job: the built executable records the digest of the helper beside it.
        parser = argparse.ArgumentParser(description="Check the recorded authorization helper digest")
        parser.add_argument("--binary", required=True, type=Path)
        parser.add_argument("--target", required=True)
        args = parser.parse_args(sys.argv[2:])
        helper = authorization_helper(args.binary, args.target)[1]
        print(json.dumps({"authorization_helper_sha256": check_authorization_helper_digest(args.binary, helper)}))
        return
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
    parser.add_argument("--filesystems-root", type=Path,
                        help="Required native, network and archive packages; defaults beside executable")
    parser.add_argument("--thumbnailers-root", type=Path,
                        help="Required image/video workers; defaults to thumbnailers beside the executable")
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
    output = build_output(args.output, root)
    stage = output / name
    check_release_readme(args.readme.read_text(encoding="utf-8"), version)
    viewers_root = args.viewers_root or args.binary.parent / "viewers"
    filesystem_root = args.filesystems_root or args.binary.parent / "filesystems"
    thumbnail_root = args.thumbnailers_root or args.binary.parent / "thumbnailers"
    # Validate every module before creating the stage: a refused input leaves no
    # partial staging directory that would block the next attempt.
    application_executable(args.binary, args.target)
    check_authorization_helper_digest(args.binary, authorization_helper(args.binary, args.target)[1])
    copy_viewer_packages(stage, viewers_root, args.target, required_ids=REQUIRED_VIEWERS, validate_only=True)
    copy_filesystem_packages(stage, filesystem_root, args.target, require_native=True, require_network=True,
                             require_archive=True, validate_only=True)
    copy_thumbnail_packages(stage, thumbnail_root, args.target, require_media=True, validate_only=True)
    stage.mkdir(parents=True, exist_ok=False)
    binary = stage / args.binary.name
    shutil.copyfile(args.binary, binary)
    binary.chmod(0o755)
    authorization = copy_authorization_helper(stage, args.binary, args.target)
    # Again for the staged copies, which are what the archive ships.
    check_authorization_helper_digest(binary, stage / authorization["path"])
    sandbox_installer = copy_html_sandbox_installer(stage, root, args.target)
    shutil.copyfile(args.readme, stage / "README.txt")
    shutil.copyfile(root / "Cargo.lock", stage / "Cargo.lock")
    shutil.copyfile(root / "assets/branding/midnight-explorer.png", stage / "midnight-explorer.png")
    copy_project_documents(stage, root, args.target)
    if args.desktop_entry:
        shutil.copyfile(args.desktop_entry, stage / args.desktop_entry.name)
    plugins = copy_viewer_packages(stage, viewers_root, args.target, required_ids=REQUIRED_VIEWERS)
    filesystems = copy_filesystem_packages(stage, filesystem_root, args.target,
                                          require_native=True, require_network=True, require_archive=True)
    thumbnails = copy_thumbnail_packages(stage, thumbnail_root, args.target, require_media=True)
    worker_files = [file for package in filesystems for file in package["files"]] + thumbnails["files"]
    add_notices(stage, json.loads(args.metadata.read_text(encoding="utf-8-sig")),
                args.dependency_list.read_text(encoding="utf-8-sig"),
                args.rust_docs, args.license_overrides)
    with binary.open("rb") as stream:
        binary_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    (stage / "BUILD-INFO.json").write_text(json.dumps({
        "version": version, "target": args.target, "revision": args.revision,
        "profile": "release", "rust": rust, "binary": binary.name,
        "binary_sha256": binary_hash,
        "authorization_helper": authorization,
        "viewer_plugins": plugins,
        "filesystem_plugins": filesystems,
        "thumbnail_workers": thumbnails,
    }, indent=2) + "\n", encoding="utf-8")
    archive = output / f"{name}.{args.format}"
    if archive.exists():
        raise FileExistsError(archive)
    viewer_executables = [file["path"] for plugin in plugins for file in plugin["files"]
                          if file.get("executable")]
    write_archive(stage, archive, binary.name, args.format,
                  [file["path"] for file in worker_files if file["executable"]] + viewer_executables
                  + [authorization["path"]] + ([sandbox_installer] if sandbox_installer else []))
    print(archive)


if __name__ == "__main__":
    main()
