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
import sys
import tarfile
import tomllib
import zipfile
from pathlib import Path

REQUIRED_VIEWERS = ("midnight.viewer.basic", "midnight.viewer.source", "midnight.viewer.markdown",
                    "midnight.viewer.image", "midnight.viewer.pdf", "midnight.viewer.html")
FILESYSTEM_ABI_MINOR = 3


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


def copy_viewer_packages(stage, viewers, target, required_ids=(), *, validate_only=False):
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
    for required in (filename, executable, *(('LICENSE.txt', 'THIRD_PARTY.md') if web else
                                           ('LICENSE', 'THIRD-PARTY.md'))):
        if package / required not in files:
            raise ValueError(f"Image viewer package missing required file {required}: {package}")
    return manifest


def validate_web_runtime(package, files, manifest, target):
    required = {"runtime-files.json", "LICENSE.txt", "CREDITS.html", "THIRD_PARTY.md",
                "dependency-metadata.json", "icudtl.dat", "resources.pak",
                "chrome_100_percent.pak", "chrome_200_percent.pak", "v8_context_snapshot.bin",
                "vk_swiftshader_icd.json"}
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
    source = Path(source).absolute()
    if source.resolve() != source or not stat.S_ISDIR(plain_metadata(source).st_mode):
        raise ValueError("Worker input must be a real directory")
    if Path(stage).resolve().is_relative_to(source):
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
                or manifest["protocol"] not in (1, 2, 3) or not isinstance(formats, list) or not 1 <= len(formats) <= 16
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
    supported = ({"browse", "stat", "volumes", "activate", "refresh", "resolve-text"} if network else
                 {"browse", "read", "stat", "volumes", "storage", "native-address", "volume-details",
                  "prepared-rename"})
    capabilities = manifest.get("capabilities")
    version = manifest.get("version")
    minimum_host_minor = manifest.get("minimum_host_minor")
    minimum_minor = 2 if network else 0
    encoding = "opaque" if network else "windows-utf16le" if "windows" in target else "unix-bytes"
    required = supported if network else {"native-address"}
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
            or ("prepared-rename" in capabilities and minimum_host_minor < 3)):
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
    source = Path(source).absolute()
    if source.resolve() != source or not stat.S_ISDIR(plain_metadata(source).st_mode):
        raise ValueError("Runtime installation must be a real directory")
    suffix = ".exe" if target.endswith("windows-msvc") else ""
    for name in ("me" + suffix, "midnight-elevated-listing" + suffix):
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
        "Midnight Explorer uses Slint under its royalty-free desktop application license.\n"
        "The release download page displays Slint's attribution badge.\n"
        "These notices do not assign a new license to Midnight Explorer source code.\n",
        encoding="utf-8")


def write_archive(stage, archive, binary_name, archive_format, executables=()):
    binary_path = f"{stage.name}/{binary_name}"
    files = plain_tree(stage, maximum=65536)
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


def copy_authorization_helper(stage, binary, target):
    name = "midnight-elevated-listing.exe" if target.endswith("windows-msvc") else "midnight-elevated-listing"
    source = (Path(binary).parent / name).absolute()
    metadata = plain_metadata(source)
    if source.resolve() != source or not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 512 * 1024 * 1024:
        raise ValueError("Authorization helper must be a regular, bounded, unredirected sibling executable")
    destination = stage / name
    shutil.copyfile(source, destination)
    destination.chmod(0o755)
    return {"path": name, "bytes": metadata.st_size, "sha256": sha256_file(destination), "executable": True}


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "validate-runtime":
        parser = argparse.ArgumentParser(description="Validate every required runtime module")
        parser.add_argument("--source", required=True, type=Path)
        parser.add_argument("--target", required=True)
        args = parser.parse_args(sys.argv[2:])
        print(json.dumps(validate_runtime_installation(args.source, args.target)))
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
    output = args.output.resolve()
    if not output.is_relative_to(root / "_build"):
        raise ValueError("Release output must remain under the repository's _build directory")
    stage = output / name
    stage.mkdir(parents=True, exist_ok=False)
    binary = stage / args.binary.name
    shutil.copyfile(args.binary, binary)
    binary.chmod(0o755)
    authorization = copy_authorization_helper(stage, args.binary, args.target)
    shutil.copyfile(args.readme, stage / "README.txt")
    shutil.copyfile(root / "Cargo.lock", stage / "Cargo.lock")
    shutil.copyfile(root / "assets/branding/midnight-explorer.png", stage / "midnight-explorer.png")
    for relative in ("SECURITY.md", "docs/SYSTEM_REQUIREMENTS.md", "docs/KEYBOARD_SHORTCUTS.md",
                     "docs/FEATURE_STATUS.md", "docs/ASSET_PROVENANCE.md"):
        shutil.copyfile(root / relative, stage / Path(relative).name)
    if args.desktop_entry:
        shutil.copyfile(args.desktop_entry, stage / args.desktop_entry.name)
    plugins = copy_viewer_packages(stage, args.viewers_root or args.binary.parent / "viewers",
                                  args.target, required_ids=REQUIRED_VIEWERS)
    filesystem_root = args.filesystems_root or args.binary.parent / "filesystems"
    thumbnail_root = args.thumbnailers_root or args.binary.parent / "thumbnailers"
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
                  + [authorization["path"]])
    print(archive)


if __name__ == "__main__":
    main()
