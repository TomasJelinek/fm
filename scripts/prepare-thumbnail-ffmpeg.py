#!/usr/bin/env python3
"""Prepare the pinned, restricted shared FFmpeg runtime for normal thumbnail builds."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import uuid


REPOSITORY = Path(__file__).resolve().parents[1]
VERSION = '9.0.2'
SOURCE_SHA256 = '8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e'
SIGNER_FINGERPRINT = 'FCF986EA15E6E293A5644F10B4322F04D67658D8'
CACHE_ROOT = REPOSITORY / '.tools' / 'media-native' / ('ffmpeg-' + VERSION)
ARCHIVE_NAME = 'ffmpeg-' + VERSION + '.tar.xz'
SOURCE_URL = 'https://ffmpeg.org/releases/' + ARCHIVE_NAME
KEY_URL = 'https://ffmpeg.org/ffmpeg-devel.asc'
DISABLED_FEATURES = ('GPL', 'NONFREE', 'VERSION3', 'NETWORK')
BASELINE_OPTIONS = (
    '--disable-everything', '--disable-autodetect', '--disable-programs', '--disable-doc',
    '--disable-debug', '--enable-shared', '--disable-static', '--enable-pic',
    '--disable-gpl', '--disable-nonfree', '--disable-version3', '--disable-network',
    '--disable-protocols', '--disable-avdevice', '--disable-avfilter', '--disable-swresample',
    '--disable-hwaccels', '--disable-pthreads', '--disable-w32threads', '--disable-x86asm',
    '--enable-avcodec', '--enable-avformat', '--enable-avutil', '--enable-swscale',
    '--enable-decoder=vp8,vp9,theora,ffv1,rawvideo',
    '--enable-demuxer=matroska,avi,ogg', '--enable-parser=vp8,vp9',
)


def ordinary_path(value):
    path = Path(value)
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('Expected an absolute path without parent traversal: ' + str(path))
    for part in (path, *path.parents):
        try:
            info = part.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            raise ValueError('Path must not traverse a link or reparse point: ' + str(part))
    return path


def build_directory(value):
    path = ordinary_path(value)
    if '_build' not in path.parts[:-1]:
        raise ValueError('FFmpeg generated output must be beneath an explicit _build directory')
    path.mkdir(parents=True, exist_ok=True)
    return path


def regular_file(value):
    path = ordinary_path(value)
    if not path.is_file():
        raise ValueError('Required regular file is missing: ' + str(path))
    return path


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def check_source_archive(path):
    path = regular_file(path)
    if path.stat().st_size > 32 * 1024 * 1024 or digest(path) != SOURCE_SHA256:
        raise ValueError('FFmpeg source archive checksum does not match the pinned release')
    return path


def fetch_artifact(path, url, maximum, offline=False):
    path = ordinary_path(path)
    if path.exists():
        regular_file(path)
        if path.stat().st_size > maximum:
            raise ValueError('FFmpeg cached download exceeds its byte limit: ' + str(path))
        return path
    if offline:
        raise ValueError('Offline FFmpeg cache is missing: ' + str(path)
                         + '; run once with network access or supply a verified prebuilt override')
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(uuid.uuid4().hex + '.download')
    try:
        print('Downloading pinned FFmpeg artifact: ' + url, file=sys.stderr)
        with urllib.request.urlopen(url, timeout=30) as response, temporary.open('xb') as output:
            if not response.geturl().startswith('https://'):
                raise ValueError('FFmpeg download redirected away from HTTPS')
            size = 0
            while block := response.read(256 * 1024):
                size += len(block)
                if size > maximum:
                    raise ValueError('FFmpeg download exceeds its byte limit')
                output.write(block)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)
    return path


def verify_release(archive, signature, key, build_root):
    check_source_archive(archive)
    regular_file(signature)
    regular_file(key)
    gpg = shutil.which('gpg')
    if not gpg:
        raise ValueError('FFmpeg source verification requires gpg; run scripts/setup-linux.sh')
    with tempfile.TemporaryDirectory(prefix='ffmpeg-gpg-', dir=build_root) as temporary:
        os.chmod(temporary, 0o700)
        command = [gpg, '--no-options', '--homedir', temporary, '--batch',
                   '--no-auto-key-retrieve']
        subprocess.run([*command, '--import', str(key)], check=True, stdout=sys.stderr)
        fingerprints = subprocess.run(
            [*command, '--with-colons', '--fingerprint'], check=True,
            stdout=subprocess.PIPE, text=True).stdout
        primary = None
        for line in fingerprints.splitlines():
            fields = line.split(':')
            if fields[0] == 'pub':
                primary = ''
            elif fields[0] == 'fpr' and primary == '':
                primary = fields[9]
                if primary != SIGNER_FINGERPRINT:
                    raise ValueError('Unexpected FFmpeg release signing fingerprint: ' + primary)
        if primary is None:
            raise ValueError('FFmpeg release signing key has no primary fingerprint')
        verification = subprocess.run(
            [*command, '--status-fd=1', '--verify', str(signature), str(archive)],
            check=True, stdout=subprocess.PIPE, text=True).stdout
        valid = [line.split() for line in verification.splitlines()
                 if line.startswith('[GNUPG:] VALIDSIG ')]
        if not any(fields[2] == SIGNER_FINGERPRINT or fields[-1] == SIGNER_FINGERPRINT
                   for fields in valid):
            raise ValueError('FFmpeg source signature does not match the pinned release key')


def extract_source(archive, destination):
    destination = ordinary_path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    expected = 'ffmpeg-' + VERSION
    seen = set()
    with tarfile.open(archive, 'r:xz') as bundle:
        members = []
        expanded = 0
        for member in bundle:
            name = PurePosixPath(member.name)
            pieces = member.name.rstrip('/').split('/')
            expanded += member.size
            if (len(members) >= 50000 or expanded > 512 * 1024 * 1024
                    or not pieces or pieces[0] != expected
                    or any(part in ('', '.', '..') for part in pieces)
                    or '\\' in member.name or ':' in member.name
                    or any(ord(char) < 32 for char in member.name)
                    or name.is_absolute() or member.name in seen
                    or not (member.isfile() or member.isdir())):
                raise ValueError('Unexpected path, link or size in FFmpeg source archive')
            seen.add(member.name)
            members.append(member)
        for member in members:
            target = ordinary_path(destination / member.name)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with bundle.extractfile(member) as source, target.open('xb') as output:
                    shutil.copyfileobj(source, output, 256 * 1024)
                target.chmod(0o755 if member.mode & 0o111 else 0o644)
    source = destination / expected
    if (source / 'RELEASE').read_text(encoding='utf-8').strip() != VERSION:
        raise ValueError('FFmpeg source release does not match its pinned version')
    return source


def runtime_names(platform_name):
    if platform_name == 'win32':
        return ['bin/avcodec-63.dll', 'bin/avformat-63.dll', 'bin/avutil-61.dll',
                'bin/swscale-10.dll']
    return ['lib/libavcodec.so.63', 'lib/libavformat.so.63', 'lib/libavutil.so.61',
            'lib/libswscale.so.10']


def runtime_file(prefix, name):
    path = prefix / name
    ordinary_path(path.parent)
    resolved = path.resolve(strict=True)
    if not resolved.is_relative_to(prefix):
        raise ValueError('FFmpeg runtime link resolves outside its verified prefix')
    return regular_file(resolved)


def validate_prefix(value, platform_name):
    prefix = ordinary_path(value)
    notices = prefix / 'share' / 'midnight-ffmpeg'
    paths = [regular_file(notices / name) for name in (
        'COPYING.LGPLv2.1', 'LICENSE.md', 'build-options.txt', 'config.h',
        'config_components.h', 'config.mak')]
    options = (notices / 'build-options.txt').read_text(encoding='utf-8').splitlines()
    expected = set(BASELINE_OPTIONS)
    if platform_name == 'win32':
        expected.update(('--toolchain=msvc', '--target-os=win64', '--arch=x86_64'))
    if (not options or options[0] != 'FFmpeg ' + VERSION
            or len([option for option in options[1:] if option.startswith('--prefix=')]) != 1
            or {option for option in options[1:] if not option.startswith('--prefix=')} != expected
            or len(options) != len(expected) + 2):
        raise ValueError('FFmpeg build options do not match the approved free decoder baseline')
    config = (notices / 'config.h').read_text(encoding='utf-8')
    for feature in DISABLED_FEATURES:
        values = re.findall(r'^\s*#\s*define\s+CONFIG_' + feature + r'\s+([^\r\n]+)$',
                            config, re.MULTILINE)
        if values != ['0']:
            raise ValueError('FFmpeg must disable CONFIG_' + feature)
    components = (notices / 'config_components.h').read_text(encoding='utf-8')
    # The pinned configure selects the VP3 implementation for the Theora decoder.
    # The worker's independent codec whitelist still accepts only the five public codecs.
    approved = {name + '_DECODER'
                for name in ('VP8', 'VP9', 'THEORA', 'VP3', 'FFV1', 'RAWVIDEO')}
    approved.update(name + '_DEMUXER' for name in ('MATROSKA', 'AVI', 'OGG'))
    approved.update(('VP8_PARSER', 'VP9_PARSER'))
    for feature in re.findall(r'^#define CONFIG_(\w+) 1$', components, re.MULTILINE):
        if (feature.endswith(('_DECODER', '_DEMUXER', '_PARSER', '_ENCODER', '_MUXER',
                              '_PROTOCOL', '_FILTER', '_INDEV', '_OUTDEV', '_HWACCEL'))
                and feature not in approved):
            raise ValueError('Unapproved FFmpeg component: ' + feature)
    paths.extend(runtime_file(prefix, name) for name in runtime_names(platform_name))
    for name in ('avcodec', 'avformat', 'avutil', 'swscale'):
        paths.append(regular_file(prefix / 'include' / ('lib' + name) / (name + '.h')))
        if platform_name == 'win32':
            paths.append(regular_file(prefix / 'lib' / (name + '.lib')))
    return paths


def find_libclang(environ):
    requested = environ.get('LIBCLANG_PATH')
    candidates = [Path(requested)] if requested else []
    if not requested:
        candidates.extend(sorted(Path('/usr/lib').glob('llvm-*/lib'), reverse=True))
        candidates.extend((Path('/usr/lib/x86_64-linux-gnu'), Path('/usr/local/lib')))
        llvm_config = shutil.which('llvm-config')
        if llvm_config:
            result = subprocess.run([llvm_config, '--libdir'], text=True, check=True,
                                    stdout=subprocess.PIPE)
            candidates.append(Path(result.stdout.strip()))
    for candidate in candidates:
        candidate = ordinary_path(candidate)
        if any(candidate.glob('libclang.so*')) or any(candidate.glob('libclang-*.so*')) \
                or (candidate / 'libclang.dll').is_file():
            return candidate
    raise ValueError('libclang is required for video bindings; run scripts/setup-linux.sh '
                     'or set LIBCLANG_PATH to its library directory')


def artifact_hashes(paths, prefix):
    return {str(path.relative_to(prefix)): digest(path) for path in paths}


def prepare(build_root, offline=False, environ=None, platform_name=None):
    environ = os.environ if environ is None else environ
    platform_name = sys.platform if platform_name is None else platform_name
    overrides = [environ.get(name) for name in (
        'MIDNIGHT_FFMPEG_PREFIX', 'MIDNIGHT_FFMPEG_SOURCE_ARCHIVE')]
    if any(overrides):
        if not all(overrides) or not environ.get('LIBCLANG_PATH'):
            raise ValueError('A prebuilt FFmpeg override requires MIDNIGHT_FFMPEG_PREFIX, '
                             'MIDNIGHT_FFMPEG_SOURCE_ARCHIVE and LIBCLANG_PATH together')
        prefix = ordinary_path(overrides[0])
        archive = check_source_archive(Path(overrides[1]))
        validate_prefix(prefix, platform_name)
        return {'prefix': str(prefix), 'source_archive': str(archive),
                'libclang_path': str(find_libclang(environ))}
    if platform_name == 'win32':
        raise ValueError('Windows normal builds require a verified FFmpeg 9.0.2 MSVC prefix: '
                         'set MIDNIGHT_FFMPEG_PREFIX, MIDNIGHT_FFMPEG_SOURCE_ARCHIVE and '
                         'LIBCLANG_PATH; see plugins/midnight-thumbnail-video/README.md')
    if platform_name != 'linux':
        raise ValueError('Automatic FFmpeg preparation currently supports Linux only')
    root = build_directory(build_root)
    cache = ordinary_path(CACHE_ROOT)
    archive = fetch_artifact(cache / ARCHIVE_NAME, SOURCE_URL, 32 * 1024 * 1024, offline)
    check_source_archive(archive)
    signature = fetch_artifact(cache / (ARCHIVE_NAME + '.asc'), SOURCE_URL + '.asc',
                               64 * 1024, offline)
    key = fetch_artifact(cache / 'ffmpeg-devel.asc', KEY_URL, 64 * 1024, offline)
    verify_release(archive, signature, key, root)
    libclang = find_libclang(environ)
    script = regular_file(REPOSITORY / 'scripts' / 'build-media-ffmpeg.sh')
    recipe = {'source_sha256': SOURCE_SHA256, 'script_sha256': digest(script),
              'platform': platform_name, 'machine': os.uname().machine}
    prefix = ordinary_path(root / 'prefix')
    receipt = ordinary_path(root / 'verified-build.json')
    if receipt.is_file():
        saved = json.loads(receipt.read_text(encoding='utf-8'))
        if saved.get('recipe') == recipe:
            try:
                paths = validate_prefix(prefix, platform_name)
                if saved.get('artifacts') == artifact_hashes(paths, prefix):
                    return {'prefix': str(prefix), 'source_archive': str(archive),
                            'libclang_path': str(libclang)}
            except (OSError, ValueError):
                pass
    source_root = ordinary_path(root / 'source')
    if source_root.exists():
        shutil.rmtree(source_root)
    source = extract_source(archive, source_root)
    objects = ordinary_path(root / 'objects')
    if objects.exists():
        shutil.rmtree(objects)
    subprocess.run(['bash', str(script), str(source), str(objects), str(prefix), 'linux'],
                   check=True, stdout=sys.stderr)
    paths = validate_prefix(prefix, platform_name)
    temporary = receipt.with_name(uuid.uuid4().hex + '.json')
    try:
        with temporary.open('x', encoding='utf-8') as output:
            json.dump({'recipe': recipe, 'artifacts': artifact_hashes(paths, prefix)}, output,
                      indent=2)
            output.write('\n')
        temporary.replace(receipt)
    finally:
        temporary.unlink(missing_ok=True)
    return {'prefix': str(prefix), 'source_archive': str(archive),
            'libclang_path': str(libclang)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-root', required=True, type=Path)
    parser.add_argument('--offline', action='store_true')
    args = parser.parse_args()
    try:
        result = prepare(args.build_root, args.offline)
    except (OSError, ValueError, tarfile.TarError, subprocess.CalledProcessError) as error:
        print('FFmpeg preparation failed: ' + str(error), file=sys.stderr)
        return 2
    print(json.dumps(result))
    return 0


if __name__ == '__main__':
    sys.exit(main())
