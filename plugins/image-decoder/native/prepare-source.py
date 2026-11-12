#!/usr/bin/env python3
"""Verify and extract the pinned dav1d source beneath a caller-owned _build tree."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile
import urllib.request
import uuid

VERSION = '1.5.4'
SHA256 = '686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5'
URL = f'https://download.videolan.org/pub/videolan/dav1d/{VERSION}/dav1d-{VERSION}.tar.xz'


def build_directory(value):
    path = Path(value).absolute()
    if path != path.resolve() or '_build' not in path.parts:
        raise ValueError('Native build output must be under an ordinary _build directory')
    path.mkdir(parents=True, exist_ok=True)
    return path


def prepare(value, offline=False):
    root = build_directory(value)
    archive = root / f'dav1d-{VERSION}.tar.xz'
    if not archive.exists():
        if offline:
            raise ValueError(f'Offline source archive is missing: {archive}')
        temporary = root / (uuid.uuid4().hex + '.download')
        try:
            with urllib.request.urlopen(URL, timeout=30) as response, temporary.open('xb') as out:
                size = 0
                while block := response.read(256 * 1024):
                    size += len(block)
                    if size > 4 * 1024 * 1024:
                        raise ValueError('dav1d source download is unexpectedly large')
                    out.write(block)
            if hashlib.sha256(temporary.read_bytes()).hexdigest() != SHA256:
                raise ValueError('dav1d source checksum mismatch')
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
    if archive.is_symlink() or hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        raise ValueError('Cached dav1d source checksum mismatch')
    source = root / f'dav1d-{VERSION}'
    receipt = root / 'source.json'
    if source.exists():
        if source.is_symlink() or not receipt.is_file():
            raise ValueError('Unrecognized existing native source directory')
        if json.loads(receipt.read_text(encoding='utf-8')) != {'url': URL, 'sha256': SHA256}:
            raise ValueError('Existing native source belongs to another release')
        return source
    with tarfile.open(archive) as bundle:
        members = bundle.getmembers()
        if sum(item.size for item in members) > 32 * 1024 * 1024:
            raise ValueError('dav1d source expansion exceeds expected bounds')
        for item in members:
            name = PurePosixPath(item.name)
            if (not name.parts or name.parts[0] != source.name or name.is_absolute()
                    or '..' in name.parts or '\\' in item.name
                    or not (item.isfile() or item.isdir())):
                raise ValueError('Unexpected path or link in dav1d source archive')
        bundle.extractall(root, members=members, filter='data')
    receipt.write_text(json.dumps({'url': URL, 'sha256': SHA256}, indent=2) + '\n',
                       encoding='utf-8')
    return source


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('build_root')
    parser.add_argument('--offline', action='store_true')
    args = parser.parse_args()
    print(prepare(args.build_root, args.offline))
