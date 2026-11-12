#!/usr/bin/env python3
"""Pinned private Meson/Ninja/NASM archives; no package-manager or global installs."""
import argparse
from dataclasses import dataclass
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import stat
import sys
import tarfile
import urllib.request
import uuid
import zipfile

MAX_DOWNLOAD_BYTES = 16 * 1024 * 1024
MAX_EXTRACT_BYTES = 64 * 1024 * 1024
MAX_ENTRIES = 8192

@dataclass(frozen=True)
class Artifact:
    name: str
    filename: str
    url: str
    sha256: str
    kind: str
    license: str

MESON = Artifact('meson-1.9.1', 'meson-1.9.1-py3-none-any.whl',
    'https://files.pythonhosted.org/packages/9c/07/b48592d325cb86682829f05216e4efb2dc881762b8f1bafb48b57442307a/meson-1.9.1-py3-none-any.whl',
    'f824ab770c041a202f532f69e114c971918ed2daff7ea56583d80642564598d0', 'zip', 'Apache-2.0')
NINJA_WINDOWS = Artifact('ninja-1.13.0-windows', 'ninja-1.13.0-py3-none-win_amd64.whl',
    'https://files.pythonhosted.org/packages/29/45/c0adfbfb0b5895aa18cec400c535b4f7ff3e52536e0403602fc1a23f7de9/ninja-1.13.0-py3-none-win_amd64.whl',
    'fb8ee8719f8af47fed145cced4a85f0755dd55d45b2bddaf7431fa89803c5f3e', 'zip', 'Apache-2.0')
NINJA_LINUX = Artifact('ninja-1.13.0-linux', 'ninja-1.13.0-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl',
    'https://files.pythonhosted.org/packages/ed/de/0e6edf44d6a04dabd0318a519125ed0415ce437ad5a1ec9b9be03d9048cf/ninja-1.13.0-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl',
    'fb46acf6b93b8dd0322adc3a4945452a4e774b75b91293bafcc7b7f8e6517dfa', 'zip', 'Apache-2.0')
NASM_WINDOWS = Artifact('nasm-2.16.03-windows', 'nasm-2.16.03-win64.zip',
    'https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/win64/nasm-2.16.03-win64.zip',
    '3ee4782247bcb874378d02f7eab4e294a84d3d15f3f6ee2de2f47a46aa7226e6', 'zip', 'BSD-2-Clause')
NASM_LINUX = Artifact('nasm-2.16.03-linux', 'nasm-2.16.03.tar.xz',
    'https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/nasm-2.16.03.tar.xz',
    '1412a1c760bbd05db026b6c0d1657affd6631cd0a63cddb6f73cc6d4aa616148', 'tar.xz', 'BSD-2-Clause')


def ordinary_root(value):
    path = Path(value)
    if not path.is_absolute() or path != path.resolve() or '_build' not in path.parts:
        raise ValueError('Tool output requires an absolute ordinary _build directory')
    path.mkdir(parents=True, exist_ok=True)
    return path


def directory(root, name):
    path = root / name
    if path.resolve() != path:
        raise ValueError('Linked generated directory is not allowed')
    path.mkdir(parents=True, exist_ok=True)
    return path


def regular(path):
    return path.resolve() == path and stat.S_ISREG(path.lstat().st_mode)


def checked_bytes(path, artifact):
    if not regular(path) or path.stat().st_size > MAX_DOWNLOAD_BYTES:
        raise ValueError('Invalid cached archive file')
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != artifact.sha256:
        raise ValueError('Tool archive checksum mismatch: ' + artifact.filename)
    return data


def fetch(root, artifact, offline=False, opener=urllib.request.urlopen):
    downloads = directory(root, 'downloads')
    path = downloads / artifact.filename
    if path.exists() or path.is_symlink():
        return checked_bytes(path, artifact)
    if offline:
        raise ValueError('Offline tool archive is missing: ' + artifact.filename)
    temporary = downloads / (uuid.uuid4().hex + '.download')
    try:
        with opener(artifact.url, timeout=30) as response, temporary.open('xb') as output:
            size = 0
            while block := response.read(64 * 1024):
                size += len(block)
                if size > MAX_DOWNLOAD_BYTES:
                    raise ValueError('Tool download exceeds its byte limit')
                output.write(block)
        data = checked_bytes(temporary, artifact)
        temporary.replace(path)
        return data
    finally:
        temporary.unlink(missing_ok=True)


def member_name(raw):
    name = raw.rstrip('/')
    pieces = name.split('/')
    if (not name or name.startswith('/') or chr(92) in name or ':' in name
            or any(piece in ('', '.', '..') for piece in pieces)
            or any(ord(char) < 32 for char in name)):
        raise ValueError('Invalid archive path')
    return PurePosixPath(name).as_posix()


def archive_entries(artifact, data):
    if hashlib.sha256(data).hexdigest() != artifact.sha256:
        raise ValueError('Tool archive checksum mismatch')
    entries, seen = [], set()
    if artifact.kind == 'zip':
        bundle = zipfile.ZipFile(io.BytesIO(data))
        members = bundle.infolist()
    elif artifact.kind == 'tar.xz':
        bundle = tarfile.open(fileobj=io.BytesIO(data), mode='r|xz')
        members = bundle
    else:
        raise ValueError('Unsupported archive type')
    with bundle:
        expanded = 0
        for count, item in enumerate(members, 1):
            expanded += item.file_size if artifact.kind == 'zip' else item.size
            if count > MAX_ENTRIES or expanded > MAX_EXTRACT_BYTES:
                raise ValueError('Tool archive expansion exceeds its limit')
            if artifact.kind == 'zip':
                name = member_name(item.orig_filename)
                mode = item.external_attr >> 16
                is_dir, mtime = item.is_dir(), None
                if stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR):
                    raise ValueError('Archive link or special type is not allowed')
                payload = b'' if is_dir else bundle.read(item)
            else:
                name = member_name(item.name)
                mode, is_dir, mtime = item.mode, item.isdir(), item.mtime
                if not (item.isfile() or is_dir):
                    raise ValueError('Archive link or special type is not allowed')
                payload = b'' if is_dir else bundle.extractfile(item).read()
            if name.casefold() in seen:
                raise ValueError('Archive path collision')
            seen.add(name.casefold())
            entries.append((name, is_dir, payload, mode, mtime))
    return entries


def check_tree(path, expected=None):
    seen, count = set(), 0
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            count += 1
            if count > MAX_ENTRIES:
                raise ValueError('Cached tool tree exceeds its entry limit')
            item = Path(base) / name
            if item.resolve() != item:
                raise ValueError('Linked cached tool path is not allowed')
        for name in files:
            item = Path(base) / name
            relative = item.relative_to(path).as_posix()
            if not regular(item):
                raise ValueError('Invalid cached tool file')
            if expected is not None:
                if (relative not in expected or item.stat().st_size != len(expected[relative])
                        or item.read_bytes() != expected[relative]):
                    raise ValueError('Cached tool contents differ from the verified archive')
                seen.add(relative)
    if expected is not None and seen != expected.keys():
        raise ValueError('Cached tool files are missing')


def restore_timestamps(root, entries):
    # Generated configure/build inputs rely on the verified release timestamps.
    # Restore only after every cached byte/path has passed verification.
    for name, is_dir, _, _, mtime in entries:
        if not is_dir and mtime is not None:
            options = {"follow_symlinks": False} if os.utime in os.supports_follow_symlinks else {}
            os.utime(root / name, (mtime, mtime), **options)

def unpack(root, artifact, data):
    entries = archive_entries(artifact, data)
    tools = directory(root, 'tools')
    destination = tools / artifact.name
    if destination.resolve() != destination:
        raise ValueError('Linked cached tool directory is not allowed')
    expected = {name: payload for name, is_dir, payload, _, _ in entries if not is_dir}
    if destination.exists():
        if not destination.is_dir():
            raise ValueError('Invalid cached tool directory')
        check_tree(destination, expected)
        restore_timestamps(destination, entries)
        return destination
    staging = tools / ('.' + artifact.name + '-' + uuid.uuid4().hex)
    staging.mkdir(mode=0o700)
    try:
        for name, is_dir, payload, mode, _ in entries:
            target = staging / name
            if is_dir:
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with target.open('xb') as output:
                    output.write(payload)
                if os.name != 'nt':
                    target.chmod((mode & 0o755) or 0o644)
        restore_timestamps(staging, entries)
        staging.rename(destination)
    finally:
        # Only our freshly created, verified in-root staging directory may be removed.
        if staging.exists() and staging.resolve() == staging and staging.parent == tools:
            shutil.rmtree(staging)
    return destination


def prepare(value, target, offline=False):
    if target not in ('windows', 'linux'):
        raise ValueError('Unsupported bootstrap platform')
    root = ordinary_root(value)
    artifacts = [MESON, NINJA_WINDOWS, NASM_WINDOWS] if target == 'windows' else [MESON, NINJA_LINUX, NASM_LINUX]
    lock = root / '.bootstrap.lock'
    with lock.open('x') as claim:
        claim.write(str(os.getpid()))
    try:
        for artifact in artifacts:
            unpack(root, artifact, fetch(root, artifact, offline))
        for name in ['bin', 'nasm-build', 'tmp']:
            check_tree(directory(root, name))
        receipt = {'platform': target, 'artifacts': [vars(item) for item in artifacts]}
        temporary = root / (uuid.uuid4().hex + '.json')
        try:
            with temporary.open('x', encoding='utf-8') as output:
                output.write(json.dumps(receipt, indent=2) + '\n')
            temporary.replace(root / 'tool-provenance.json')
        finally:
            temporary.unlink(missing_ok=True)
    finally:
        lock.unlink(missing_ok=True)
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('build_root')
    parser.add_argument('--platform', choices=['windows', 'linux'], required=True)
    parser.add_argument('--offline', action='store_true')
    args = parser.parse_args()
    if sys.version_info < (3, 12) or platform.machine().lower() not in ('amd64', 'x86_64'):
        raise ValueError('Bootstrap requires Python 3.12+ and an x86-64 host')
    print(prepare(args.build_root, args.platform, args.offline))

if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, tarfile.TarError, zipfile.BadZipFile) as error:
        print('Native tool bootstrap: ' + str(error), file=sys.stderr)
        sys.exit(1)
