#!/usr/bin/env python3
"""Validate the pinned Linux SDK and preserve complete package provenance/notices."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys

PROJECT = Path(__file__).resolve().parents[1]
PIN = PROJECT / 'scripts/cef-linux-pinned.json'


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def validate_sdk(sdk, pin):
    metadata = json.loads((sdk / 'archive.json').read_text())
    if metadata != {'type': 'minimal', 'name': pin['archive'], 'sha1': pin['sha1']}:
        raise ValueError('CEF archive.json does not identify the pinned Linux minimal archive')
    for relative, expected in pin['files'].items():
        path = sdk / relative
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'CEF SDK missing regular file: {relative}')
        if digest(path) != expected:
            raise ValueError(f'CEF SDK SHA-256 mismatch: {relative}')
    allowed = set(pin['files'])
    for relative in pin['files']:
        allowed.update(parent.as_posix() for parent in Path(relative).parents
                       if parent.as_posix() != '.')
    for path in sdk.rglob('*'):
        relative = path.relative_to(sdk).as_posix()
        if path.is_symlink() or relative not in allowed:
            raise ValueError(f'unexpected SDK entry: {relative}')


def validate_native(manifest):
    for entry in manifest['files']:
        name = Path(entry['path'])
        if name.is_absolute() or '..' in name.parts or not name.parts:
            raise ValueError('unsafe native runtime destination')
        if digest(Path(entry['source'])) != entry['sha256']:
            raise ValueError(f'native runtime SHA-256 mismatch: {name}')


def package(sdk, destination, launcher, metadata, pin, native):
    # A fresh package prevents stale files from surviving a previous runtime version.
    destination.mkdir()
    shutil.copy2(launcher, destination / 'midnight-viewer-html')
    for relative in pin['files']:
        if '/' not in relative and relative != 'CMakeLists.txt':
            shutil.copy2(sdk / relative, destination / relative)
    shutil.copytree(sdk / 'locales', destination / 'locales')
    for name in ['README.md', 'THIRD_PARTY.md']:
        shutil.copy2(PROJECT / name, destination / name)
    if native:
        for entry in native['files']:
            output = destination / entry['path']
            if output.exists():
                raise ValueError(f'native runtime would replace package file: {entry["path"]}')
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(entry['source'], output)
        public = dict(native)
        public['files'] = [{k: v for k, v in entry.items() if k != 'source'}
                           for entry in native['files']]
        (destination / 'native-dependencies.json').write_text(
            json.dumps(public, indent=2, sort_keys=True) + '\n')
    graph = json.loads(metadata.read_text())
    resolved = {node['id'] for node in graph['resolve']['nodes']}
    omissions = {
        'cef': 'cef-rs-9aa78c450489eddb828eb982eaee6c104a89d348',
        'cef-dll-sys': 'cef-rs-9aa78c450489eddb828eb982eaee6c104a89d348',
        'download-cef': 'cef-rs-207bdd2917c038283635022ffcf7767496bcc7e6',
        'objc2': 'objc2-d7d2fa23ceaa5e6096c923b081040e5d81b3b9df',
        'objc2-encode': 'objc2-8d214f5477365ffcbcbb7de058c86ed9a518efb7',
    }
    for dependency in graph['packages']:
        if dependency['id'] not in resolved or dependency['name'].startswith('midnight-'):
            continue
        source = Path(dependency['manifest_path']).parent
        if dependency['name'] in omissions:
            source = PROJECT / 'notices' / omissions[dependency['name']]
        notices = [p for p in sorted(source.rglob('*')) if p.is_file() and re.match(
            r'^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)([._-]|$)', p.name, re.I)]
        if not dependency['license'] or not notices:
            raise ValueError(f'Missing pinned license: {dependency["name"]}')
        for notice in notices:
            output = destination / 'notices' / (
                dependency['name'] + '-' + dependency['version']) / notice.relative_to(source)
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(notice, output)
    shutil.copy2(metadata, destination / 'dependency-metadata.json')
    (destination / 'web-viewer.json').write_text(json.dumps({
        'schema': 1, 'protocol': 1, 'id': 'midnight.viewer.html', 'name': 'Chromium HTML',
        'version': '0.1.0', 'target': 'x86_64-unknown-linux-gnu',
        'executable': 'midnight-viewer-html', 'extensions': ['html', 'htm', 'xhtml']}) + '\n')
    files = [{'path': p.relative_to(destination).as_posix(), 'bytes': p.stat().st_size,
              'sha256': digest(p)} for p in sorted(destination.rglob('*')) if p.is_file()]
    (destination / 'runtime-files.json').write_text(json.dumps({
        'cef': pin['cef'], 'archive_sha256': pin['archive_sha256'], 'files': files},
        indent=2, sort_keys=True) + '\n')


if __name__ == '__main__':
    try:
        if 'FLATPAK' in os.environ or 'NIX_CEF_BINARY' in os.environ:
            raise ValueError('CEF environment overrides bypass the verified SDK')
        pin = json.loads(PIN.read_text())
        sdk = Path(sys.argv[2]).resolve(strict=True)
        validate_sdk(sdk, pin)
        native_path = os.environ.get('MIDNIGHT_HTML_NATIVE_MANIFEST')
        native = json.loads(Path(native_path).read_text()) if native_path else None
        if native:
            validate_native(native)
        if sys.argv[1] == 'package':
            package(sdk, Path(sys.argv[3]), Path(sys.argv[4]), Path(sys.argv[5]), pin, native)
        elif sys.argv[1] == 'validate':
            # Cargo executes cef-dll-sys in its own crate directory, not this helper's cwd.
            sys.stdout.buffer.write(str(sdk).encode('utf-8') + b'\0')
        else:
            raise ValueError('expected validate or package')
    except (OSError, ValueError, KeyError, IndexError) as error:
        sys.exit(f'Linux CEF package rejected: {error}')
