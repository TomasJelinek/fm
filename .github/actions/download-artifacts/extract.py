#!/usr/bin/env python3
"""Extract the artifact archives that actions/download-artifact saved undecompressed.

Usage: extract.py ARCHIVES DESTINATION

ARCHIVES holds the downloaded zip files, directly or one directory per artifact; every
regular file in it must be a zip archive. All archives are extracted into DESTINATION,
which must not hold any file yet (empty directories are fine). Every member name is
checked before anything is written: it must be a relative path with '/' separators and
no empty, '.' or '..' component, no backslash, colon or NUL, and no link. A file is never
overwritten, so two archives with a member at the same path fail the step. CRC-32
errors fail it too.
"""
import os
from pathlib import Path
import shutil
import stat
import sys
import zipfile

SYMLINK = 0o120000


def archives(root):
    """Every regular file below `root`, which must all be zip archives."""
    found = []
    for directory, subdirectories, files in os.walk(root):
        for name in subdirectories + files:
            path = Path(directory, name)
            if stat.S_ISLNK(path.lstat().st_mode):
                raise ValueError(f'Link in the downloaded artifacts: {path}')
        for name in files:
            path = Path(directory, name)
            if not zipfile.is_zipfile(path):
                raise ValueError(f'Not a zip archive: {path}')
            found.append(path)
    if not found:
        raise ValueError(f'No artifact archive in {root}')
    return sorted(found)


def member_parts(info):
    """The checked path components of one archive member (a directory ends with '/')."""
    name = info.orig_filename
    if not name or '\0' in name or '\\' in name or ':' in name or name.startswith('/'):
        raise ValueError(f'Unsafe member name {name!r}')
    parts = name.removesuffix('/').split('/') if info.is_dir() else name.split('/')
    if any(part in ('', '.', '..') for part in parts):
        raise ValueError(f'Unsafe member name {name!r}')
    if stat.S_IFMT(info.external_attr >> 16) == SYMLINK:
        raise ValueError(f'Link member {name!r}')
    if info.flag_bits & 0x1:
        raise ValueError(f'Encrypted member {name!r}')
    return parts


def extract(root, destination):
    """Extract every archive below `root` into `destination`; return (archives, files, bytes)."""
    found = archives(root)
    plans = []
    for archive in found:
        with zipfile.ZipFile(archive) as opened:
            plans.append((archive, [(info, member_parts(info)) for info in opened.infolist()]))
    # A restored build cache can leave empty directories behind (rust-cache deletes the
    # files of directories below _build that are not Cargo's), but never a file.
    if destination.is_symlink() or (destination.exists() and not destination.is_dir()) or (
            destination.is_dir() and any(not path.is_dir() or path.is_symlink() for path in destination.rglob('*'))):
        raise ValueError(f'Destination already holds files: {destination}')
    destination.mkdir(parents=True, exist_ok=True)
    files = size = 0
    for archive, members in plans:
        with zipfile.ZipFile(archive) as opened:
            for info, parts in members:
                target = destination.joinpath(*parts)
                if info.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                try:
                    with opened.open(info) as source, open(target, 'xb') as output:
                        shutil.copyfileobj(source, output)
                except FileExistsError:
                    raise ValueError(f'Two archive members at one path: {target}') from None
                files += 1
                size += info.file_size
    return len(found), files, size


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 2:
        print(__doc__.split('\n\n', 2)[1], file=sys.stderr)
        return 2
    root, destination = Path(argv[0]), Path(argv[1])
    try:
        count, files, size = extract(root, destination)
    except (ValueError, OSError, zipfile.BadZipFile) as error:
        print(f'::error::Artifact extraction failed: {error}', flush=True)
        return 1
    print(f'Extracted {files} files ({size} bytes) from {count} archive(s) into {destination}', flush=True)
    return 0


if __name__ == '__main__':
    sys.exit(main())
