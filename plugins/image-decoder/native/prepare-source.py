#!/usr/bin/env python3
"""Verify and extract the pinned dav1d source beneath a caller-owned _build tree."""
import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path, PurePosixPath
import sys
import tarfile
import urllib.request
import uuid

VERSION = '1.5.4'
SHA256 = '686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5'
URL = f'https://download.videolan.org/pub/videolan/dav1d/{VERSION}/dav1d-{VERSION}.tar.xz'
MAX_ARCHIVE_BYTES = 4 * 1024 * 1024


def load_bootstrap():
    """The link rule and fresh-rename retry that the native helpers share."""
    name = 'dav1d_native_bootstrap'
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name('bootstrap-tools.py'))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


BOOTSTRAP = load_bootstrap()


# The release archive remains byte-for-byte pinned.
# The stdatomic guard is from dav1d a34e0685b9111b9eb1d7835ea7bea5b425dced73.
# The archive has no Git metadata, so its version comes from Meson.
BUILD_FIXES = (
    ('include/compat/msvc/stdatomic.h',
     'ea432d173d85546b732867f4ae40a34b71087fbc0d354970c3beadd9a3d5993c',
     '563f7ce9107fedbe0d34f3c4ec2922398eecef2e978d8180b0a29f960d390551',
     b'#if defined(__clang__) && __has_include_next(<stdatomic.h>)\n',
     b'#ifndef __has_include_next\n'
     b'#define __has_include_next(x) 0\n'
     b'#endif\n'
     b'\n'
     b'#if defined(__clang__) && __has_include_next(<stdatomic.h>)\n'),
    ('include/meson.build',
     '6231c6abae99d68a230f8767da49dfd0e56df73b65fa5ebc6529c125bddb1487',
     'cac4ee9017b82280269835b493b0e0e962f4eca72e1507e4e5a879185220af61',
     b"dav1d_git_dir = join_paths(dav1d_src_root, '.git')\n"
     b'rev_target = vcs_tag(command: [\n'
     b"        'git', '--git-dir', dav1d_git_dir, 'describe', '--long', '--always'\n"
     b'    ],\n'
     b"    input: 'vcs_version.h.in',\n"
     b"    output: 'vcs_version.h'\n"
     b')\n',
     b'rev_target = configure_file(\n'
     b"    input: 'vcs_version.h.in',\n"
     b"    output: 'vcs_version.h',\n"
     b"    configuration: {'VCS_TAG': meson.project_version()}\n"
     b')\n'),
    ('meson.build',
     '9f0eea8f574c1aa456445be1527d610b3edf7b4a65836947264bc292e52cfd1f',
     'b2baf852fb23a7a1422679c0df1288614bac1cdf1b80a349579abfaa9acf624e',
     b'else\n'
     b"    warning('Compiler does not support -fvisibility=hidden, all symbols will be public!')\n",
     b"elif get_option('default_library') != 'static'\n"
     b"    warning('Compiler does not support -fvisibility=hidden, all symbols will be public!')\n"),
)


def apply_build_fixes(source):
    for relative, original, patched, old, new in BUILD_FIXES:
        path = source / relative
        # The tree was checked for links; the file itself must be a plain one.
        if not BOOTSTRAP.regular(path):
            raise ValueError(f'Unexpected path for the dav1d build fix: {relative}')
        contents = path.read_bytes()
        actual = hashlib.sha256(contents).hexdigest()
        if actual == patched:
            continue
        if actual != original or contents.count(old) != 1:
            raise ValueError(f'Unexpected dav1d release contents: {relative}')
        updated = contents.replace(old, new, 1)
        if hashlib.sha256(updated).hexdigest() != patched:
            raise ValueError(f'dav1d build fix checksum mismatch: {relative}')
        temporary = path.with_name(f'{path.name}.{uuid.uuid4().hex}.tmp')
        try:
            with temporary.open('xb') as stream:
                stream.write(updated)
            # Retried: endpoint protection may still hold the files just extracted.
            BOOTSTRAP.replace_fresh(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def build_directory(value):
    return BOOTSTRAP.build_directory(Path(value).absolute(), 'Native build output')


def check_unlinked(root):
    """Refuse a link directly below the build root or anywhere in the extracted source.

    Meson reuses the source, build and install folders by path, so none of them may lead
    out of the checked _build; the source is reused across builds, so all of it is
    inspected, while Meson regenerates its build and install folders below their roots.
    """
    def inspect(entries):
        for item in entries:
            if BOOTSTRAP.is_link(item.lstat()):
                raise ValueError(f'Native build output must not contain a link: {item}')
    inspect(root.iterdir())
    for base, dirs, files in os.walk(root / f'dav1d-{VERSION}', followlinks=False):
        inspect(Path(base) / name for name in dirs + files)


def archive_bytes(archive):
    """The cached archive, read once: it is verified and extracted from the same bytes."""
    if not BOOTSTRAP.regular(archive) or archive.stat().st_size > MAX_ARCHIVE_BYTES:
        raise ValueError('Cached dav1d source checksum mismatch')
    data = archive.read_bytes()
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise ValueError('Cached dav1d source checksum mismatch')
    return data


def prepare(value, offline=False):
    root = build_directory(value)
    check_unlinked(root)
    archive = root / f'dav1d-{VERSION}.tar.xz'
    if not os.path.lexists(archive):
        if offline:
            raise ValueError(f'Offline source archive is missing: {archive}')
        temporary = root / (uuid.uuid4().hex + '.download')
        try:
            with urllib.request.urlopen(URL, timeout=30) as response, temporary.open('xb') as out:
                size = 0
                while block := response.read(256 * 1024):
                    size += len(block)
                    if size > MAX_ARCHIVE_BYTES:
                        raise ValueError('dav1d source download is unexpectedly large')
                    out.write(block)
            if hashlib.sha256(temporary.read_bytes()).hexdigest() != SHA256:
                raise ValueError('dav1d source checksum mismatch')
            # Retried: endpoint protection may still hold the download just written.
            BOOTSTRAP.replace_fresh(temporary, archive)
        finally:
            temporary.unlink(missing_ok=True)
    data = archive_bytes(archive)
    source = root / f'dav1d-{VERSION}'
    receipt = root / 'source.json'
    if os.path.lexists(source):
        if not source.is_dir() or not receipt.is_file():
            raise ValueError('Unrecognized existing native source directory')
        if json.loads(receipt.read_text(encoding='utf-8')) != {'url': URL, 'sha256': SHA256}:
            raise ValueError('Existing native source belongs to another release')
        apply_build_fixes(source)
        return source
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:xz') as bundle:
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
    apply_build_fixes(source)
    receipt.write_text(json.dumps({'url': URL, 'sha256': SHA256}, indent=2) + '\n',
                       encoding='utf-8')
    return source


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('build_root')
    parser.add_argument('--offline', action='store_true')
    args = parser.parse_args()
    print(prepare(args.build_root, args.offline))
