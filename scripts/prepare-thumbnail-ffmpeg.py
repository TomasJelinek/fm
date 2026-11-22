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
import time
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
    '--disable-protocols', '--disable-avdevice', '--disable-avfilter', '--enable-swresample',
    '--disable-hwaccels', '--disable-pthreads', '--disable-w32threads', '--disable-x86asm',
    '--enable-avcodec', '--enable-avformat', '--enable-avutil', '--enable-swscale',
    '--enable-decoder=vp8,vp9,theora,ffv1,rawvideo,opus',
    '--enable-demuxer=matroska,avi,ogg,mov', '--enable-parser=vp8,vp9',
)
# Windows errors while endpoint protection holds a file that was just written: access
# denied (5) renaming a folder above it, and sharing (32) or lock (33) violations.
# os.replace (MoveFileEx) never reports 1175, File.Replace's code for a replaced file it
# cannot remove (see Invoke-FreshRename in the PowerShell build scripts).
FRESH_FILE_HOLDS = (5, 32, 33) if os.name == 'nt' else ()


# Link rule of the build scripts. Folders above the checkout, above a _build
# outside it or above an input root that the caller names may be links, a subst drive
# or hidden folders: Fedora Silverblue's /home -> /var/home, a linked parent folder or
# AppData. Below such a root, and from _build down, nothing may be a link, a junction
# or another reparse point, and no path may contain '..'.

def is_link(metadata):
    """A symbolic link or any Windows reparse point, such as a junction."""
    return stat.S_ISLNK(metadata.st_mode) or bool(
        getattr(metadata, 'st_file_attributes', 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)


def named_path(value):
    path = Path(value)
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('Expected an absolute path without parent traversal: ' + str(path))
    return path


def unlinked(base, path):
    """`path` below `base` with no link from `base` down; missing folders are accepted."""
    current = base
    for part in path.relative_to(base).parts:
        current = current / part
        try:
            info = current.lstat()
        except FileNotFoundError:
            break
        if is_link(info):
            raise ValueError('Path must not traverse a link or reparse point: ' + str(current))
    return path


def checkout_depth(path):
    """Index in (path, *path.parents) of the shallowest ancestor that is the checkout.

    The checkout is recognised however the path spells it. Taking the shallowest one
    means a link below the checkout that leads back to it is inspected, not trusted.
    """
    checkout = os.stat(REPOSITORY)
    ancestors = (path, *path.parents)
    for index in reversed(range(len(ancestors))):
        try:
            if os.path.samestat(ancestors[index].stat(), checkout):
                return index
        except OSError:
            continue
    return None


def build_directory(value):
    """Generated output strictly below _build, with no link from that _build down.

    The checkout's own _build for a path in the checkout; outside it, the path's first
    _build (a custom target directory).
    """
    path = named_path(value)
    ancestors = (path, *path.parents)
    depth = checkout_depth(path)
    builds = [index for index in range(1, len(ancestors) if depth is None else depth)
              if os.path.normcase(ancestors[index].name) == '_build']
    if not builds or (depth is not None and builds[-1] != depth - 1):
        raise ValueError('FFmpeg generated output must be beneath an explicit _build directory: '
                         + str(path))
    for part in ancestors[builds[-1]::-1]:
        try:
            info = part.lstat()
        except FileNotFoundError:
            break
        if is_link(info) or not stat.S_ISDIR(info.st_mode):
            raise ValueError('Path must not traverse a link, reparse point or file: ' + str(part))
    path.mkdir(parents=True, exist_ok=True)
    return path


def generated_child(root, name):
    """An entry directly below a checked build root; it must not be a link."""
    path = root / name
    if os.path.lexists(path) and is_link(path.lstat()):
        raise ValueError('Path must not traverse a link or reparse point: ' + str(path))
    return path


def download_cache(value):
    """A download cache path inside the checkout, with no link from the checkout down.

    Stricter than the input rule on purpose: verified downloads are opened again by
    path after their checksum and signature checks, so the cache must stay a plain
    folder of the checkout rather than a link to a place that others may write in
    between. The checkout and the folders above it are not inspected.
    """
    path = named_path(value)
    depth = checkout_depth(path)
    if depth is None:
        raise ValueError('The FFmpeg download cache must lie inside the checkout: ' + str(path))
    return unlinked(path.parents[depth - 1] if depth else path, path)


def regular_file(value):
    """A regular file that is not itself a link; the folders above it may be links."""
    path = named_path(value)
    try:
        info = path.lstat()
    except FileNotFoundError:
        info = None
    if info is not None and is_link(info):
        raise ValueError('Path must not traverse a link or reparse point: ' + str(path))
    if info is None or not stat.S_ISREG(info.st_mode):
        raise ValueError('Required regular file is missing: ' + str(path))
    return path


def input_root(value):
    """A directory the caller names. It and the folders above it may be links."""
    path = named_path(value)
    if not path.is_dir():
        raise ValueError('Required directory is missing: ' + str(path))
    return path


def package_file(root, relative):
    """The real path of a regular file below an input root, reached without links.

    Only the root is resolved, so links above it and a root that is itself a link are
    accepted; every name below it is inspected without following links.
    """
    path = Path(root).resolve(strict=True)
    try:
        for part in Path(relative).parts:
            path = path / part
            info = path.lstat()
            if is_link(info):
                raise ValueError('Package input must not traverse a link or reparse point: '
                                 + str(Path(root) / relative))
    except FileNotFoundError:
        info = None
    if info is None or not stat.S_ISREG(info.st_mode):
        raise ValueError('Required regular file is missing: ' + str(Path(root) / relative))
    return path


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
            if getattr(error, 'winerror', None) not in FRESH_FILE_HOLDS:
                raise
            now = time.monotonic()
            if first_error is None:
                first_error, deadline = error, now + timeout
            if now >= deadline:
                break
        time.sleep(min(delay, deadline - now))
        delay = min(2 * delay, 0.2)
    raise first_error


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def check_source_archive(path):
    path = regular_file(path)
    if path.stat().st_size > 32 * 1024 * 1024 or digest(path) != SOURCE_SHA256:
        raise ValueError('FFmpeg source archive checksum does not match the pinned release')
    return path


def fetch_artifact(path, url, maximum, offline=False):
    path = download_cache(path)
    if os.path.lexists(path):
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
        # Retried: endpoint protection may still hold the download just written.
        replace_fresh(temporary, path)
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
    destination = build_directory(destination)
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
            target = unlinked(destination, destination / member.name)
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
                'bin/swscale-10.dll', 'bin/swresample-7.dll']
    return ['lib/libavcodec.so.63', 'lib/libavformat.so.63', 'lib/libavutil.so.61',
            'lib/libswscale.so.10', 'lib/libswresample.so.7']


def runtime_file(prefix, name):
    """A runtime library below the prefix, reached without links down to its folder.

    The library name itself may be the versioned link that an FFmpeg installation
    creates, but it must resolve to a regular file inside the prefix.
    """
    base = Path(prefix).resolve(strict=True)
    folder = base
    for part in Path(name).parent.parts:
        folder = folder / part
        if is_link(folder.lstat()):
            raise ValueError('Package input must not traverse a link or reparse point: '
                             + str(Path(prefix) / name))
    resolved = (folder / Path(name).name).resolve(strict=True)
    if not resolved.is_relative_to(base):
        raise ValueError('FFmpeg runtime link resolves outside its verified prefix')
    return regular_file(resolved)


def validate_prefix(value, platform_name):
    """Check a prefix; return its files below the resolved prefix (see artifact_hashes)."""
    prefix = input_root(value)
    notices = Path('share', 'midnight-ffmpeg')
    files = {name: package_file(prefix, notices / name) for name in (
        'COPYING.LGPLv2.1', 'LICENSE.md', 'build-options.txt', 'config.h',
        'config_components.h', 'config.mak')}
    paths = list(files.values())
    options = files['build-options.txt'].read_text(encoding='utf-8').splitlines()
    expected = set(BASELINE_OPTIONS)
    if platform_name == 'win32':
        expected.update(('--toolchain=msvc', '--target-os=win64', '--arch=x86_64'))
    if (not options or options[0] != 'FFmpeg ' + VERSION
            or len([option for option in options[1:] if option.startswith('--prefix=')]) != 1
            or {option for option in options[1:] if not option.startswith('--prefix=')} != expected
            or len(options) != len(expected) + 2):
        raise ValueError('FFmpeg build options do not match the approved free decoder baseline')
    config = files['config.h'].read_text(encoding='utf-8')
    for feature in DISABLED_FEATURES:
        values = re.findall(r'^\s*#\s*define\s+CONFIG_' + feature + r'\s+([^\r\n]+)$',
                            config, re.MULTILINE)
        if values != ['0']:
            raise ValueError('FFmpeg must disable CONFIG_' + feature)
    components = files['config_components.h'].read_text(encoding='utf-8')
    # The pinned configure selects the VP3 implementation for the Theora decoder.
    # The pinned Opus decoder selects its internal CELT/SILK/MDCT helpers.
    # Keep internal helpers distinct from externally usable codec components.
    approved = {name + '_DECODER'
                for name in ('VP8', 'VP9', 'THEORA', 'VP3', 'FFV1', 'RAWVIDEO', 'OPUS')}
    approved.update(name + '_DEMUXER' for name in ('MATROSKA', 'AVI', 'OGG', 'MOV'))
    approved.update(('VP8_PARSER', 'VP9_PARSER'))
    enabled = set(re.findall(r'^#define CONFIG_(\w+) 1$', components, re.MULTILINE))
    for feature in sorted(enabled):
        if (feature.endswith(('_DECODER', '_DEMUXER', '_PARSER', '_ENCODER', '_MUXER',
                              '_PROTOCOL', '_FILTER', '_INDEV', '_OUTDEV', '_HWACCEL'))
                and feature not in approved):
            raise ValueError('Unapproved FFmpeg component: ' + feature)
    if not {'MOV_DEMUXER', 'OPUS_DECODER'}.issubset(enabled):
        raise ValueError('FFmpeg is missing required MOV or Opus playback support')
    paths.extend(runtime_file(prefix, name) for name in runtime_names(platform_name))
    for name in ('avcodec', 'avformat', 'avutil', 'swscale', 'swresample'):
        paths.append(package_file(prefix, Path('include', 'lib' + name, name + '.h')))
        if platform_name == 'win32':
            paths.append(package_file(prefix, Path('lib', name + '.lib')))
    return paths


def llvm_version(path):
    """Numeric LLVM major version of an `llvm-N/lib` directory (llvm-18 before llvm-9)."""
    suffix = path.parent.name.removeprefix('llvm-')
    return int(suffix) if suffix.isdigit() else -1


def has_libclang(directory):
    return (any(directory.glob('libclang.so*')) or any(directory.glob('libclang-*.so*'))
            or (directory / 'libclang.dll').is_file())


def find_libclang(environ, root=Path('/usr')):
    requested = environ.get('LIBCLANG_PATH')
    if requested:
        # An explicit choice names its own root, so it and the folders above it may be
        # links (a linked LLVM folder); it is never replaced by another candidate.
        candidate = named_path(Path(requested))
        if has_libclang(candidate):
            return candidate
    else:
        candidates = sorted((root / 'lib').glob('llvm-*/lib'), key=llvm_version, reverse=True)
        candidates.extend(root / name for name in (
            'lib/x86_64-linux-gnu', 'lib/aarch64-linux-gnu', 'lib64', 'local/lib'))
        searched = [(root, candidate) for candidate in candidates]
        llvm_config = shutil.which('llvm-config', path=environ.get('PATH'))
        if llvm_config:
            result = subprocess.run([llvm_config, '--libdir'], text=True, check=True,
                                    stdout=subprocess.PIPE)
            # llvm-config names its own library folder, which may be reached through links.
            libdir = Path(result.stdout.strip())
            searched.append((libdir, libdir))
        for base, candidate in searched:
            try:
                # Below the search root a distribution link (a linked lib64, or an
                # llvm-N folder linked to another version) is skipped, not fatal: its
                # name would not tell which LLVM it holds.
                candidate = unlinked(base, named_path(candidate))
            except ValueError:
                continue
            if has_libclang(candidate):
                return candidate
    raise ValueError('libclang is required for video bindings; run scripts/setup-linux.sh '
                     'or set LIBCLANG_PATH to its library directory')


def artifact_hashes(paths, prefix):
    base = Path(prefix).resolve(strict=True)
    return {str(path.relative_to(base)): digest(path) for path in paths}


def prepare(build_root, offline=False, environ=None, platform_name=None):
    environ = os.environ if environ is None else environ
    platform_name = sys.platform if platform_name is None else platform_name
    overrides = [environ.get(name) for name in (
        'MIDNIGHT_FFMPEG_PREFIX', 'MIDNIGHT_FFMPEG_SOURCE_ARCHIVE')]
    if any(overrides):
        if not all(overrides) or not environ.get('LIBCLANG_PATH'):
            raise ValueError('A prebuilt FFmpeg override requires MIDNIGHT_FFMPEG_PREFIX, '
                             'MIDNIGHT_FFMPEG_SOURCE_ARCHIVE and LIBCLANG_PATH together')
        prefix = named_path(overrides[0])
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
    cache = download_cache(CACHE_ROOT)
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
    prefix = generated_child(root, 'prefix')
    receipt = generated_child(root, 'verified-build.json')
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
    source_root = generated_child(root, 'source')
    if source_root.exists():
        shutil.rmtree(source_root)
    source = extract_source(archive, source_root)
    objects = generated_child(root, 'objects')
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
        # Retried: endpoint protection may still hold the receipt just written.
        replace_fresh(temporary, receipt)
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
