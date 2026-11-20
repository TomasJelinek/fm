"""Pinned thumbnail runtime acquisition and prebuilt-prefix validation."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

# Sibling fixture module, also when run as scripts.tests.<module> from the checkout.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from link_fixtures import directory_link, hidden_directory, hold  # noqa: E402


SCRIPT = Path(__file__).resolve().parents[1] / 'prepare-thumbnail-ffmpeg.py'
REPOSITORY = SCRIPT.parent.parent


class PrepareThumbnailFfmpeg(unittest.TestCase):
    def setUp(self):
        root = REPOSITORY / '_build' / 'tests'
        root.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=root, prefix='ffmpeg-prepare-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def module(self):
        self.assertTrue(SCRIPT.is_file(), 'Pinned FFmpeg bootstrap is not implemented')
        spec = importlib.util.spec_from_file_location('prepare_thumbnail_ffmpeg', SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def prefix(self, module, platform_name='linux'):
        prefix = self.root / 'prefix'
        notices = prefix / 'share' / 'midnight-ffmpeg'
        notices.mkdir(parents=True)
        platform_options = ('\n--toolchain=msvc\n--target-os=win64\n--arch=x86_64'
                            if platform_name == 'win32' else '')
        (notices / 'build-options.txt').write_text(
            'FFmpeg 9.0.2\n--prefix=' + str(prefix) + '\n'
            + '\n'.join(module.BASELINE_OPTIONS) + platform_options + '\n', encoding='utf-8')
        (notices / 'config.h').write_text(
            ''.join(f'#define CONFIG_{feature} 0\n' for feature in module.DISABLED_FEATURES),
            encoding='utf-8')
        (notices / 'config_components.h').write_text(
            '#define CONFIG_RAWVIDEO_DECODER 1\n#define CONFIG_AVI_DEMUXER 1\n'
            '#define CONFIG_OPUS_DECODER 1\n#define CONFIG_MOV_DEMUXER 1\n',
            encoding='utf-8')
        for name in ('COPYING.LGPLv2.1', 'LICENSE.md', 'config.mak'):
            (notices / name).write_text('Pinned source notice\n', encoding='utf-8')
        for name in module.runtime_names(platform_name):
            path = prefix / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'Runtime fixture')
        for name in ('avcodec', 'avformat', 'avutil', 'swscale', 'swresample'):
            path = prefix / 'include' / ('lib' + name) / (name + '.h')
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('/* Header fixture */', encoding='utf-8')
            if platform_name == 'win32':
                library = prefix / 'lib' / (name + '.lib')
                library.parent.mkdir(parents=True, exist_ok=True)
                library.write_bytes(b'Import library fixture')
        return prefix

    def test_cli_failure_keeps_stdout_empty(self):
        self.assertTrue(SCRIPT.is_file(), 'Pinned FFmpeg bootstrap is not implemented')
        environment = dict(os.environ)
        for name in ('MIDNIGHT_FFMPEG_PREFIX', 'MIDNIGHT_FFMPEG_SOURCE_ARCHIVE',
                     'LIBCLANG_PATH'):
            environment.pop(name, None)
        environment['MIDNIGHT_FFMPEG_PREFIX'] = str(self.root / 'prefix')
        result = subprocess.run(
            [sys.executable, '-B', str(SCRIPT), '--build-root', str(self.root / 'native'),
             '--offline'], env=environment, text=True, capture_output=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, '')
        self.assertIn('MIDNIGHT_FFMPEG_SOURCE_ARCHIVE', result.stderr)

    def test_offline_cache_failure_is_actionable(self):
        module = self.module()
        with self.assertRaisesRegex(ValueError, 'Offline FFmpeg cache is missing'):
            module.fetch_artifact(self.root / 'missing.tar.xz', module.SOURCE_URL,
                                  32 * 1024 * 1024, offline=True)

    def test_partial_prebuilt_override_is_rejected_before_acquisition(self):
        module = self.module()
        with self.assertRaisesRegex(ValueError, 'requires.*MIDNIGHT_FFMPEG_SOURCE_ARCHIVE'):
            module.prepare(self.root / 'native', environ={
                'MIDNIGHT_FFMPEG_PREFIX': str(self.root / 'prefix')})

    def test_windows_requires_explicit_verified_prebuilt_runtime(self):
        module = self.module()
        with self.assertRaisesRegex(ValueError, 'Windows.*MIDNIGHT_FFMPEG_PREFIX'):
            module.prepare(self.root / 'native', environ={}, platform_name='win32')

    def test_gpl_or_extra_codec_in_configuration_is_rejected(self):
        module = self.module()
        prefix = self.prefix(module)
        module.validate_prefix(prefix, 'linux')
        config = prefix / 'share/midnight-ffmpeg/config.h'
        config.write_text(config.read_text().replace('CONFIG_GPL 0', 'CONFIG_GPL 1'))
        with self.assertRaisesRegex(ValueError, 'CONFIG_GPL'):
            module.validate_prefix(prefix, 'linux')
        config.write_text(config.read_text().replace('CONFIG_GPL 1', 'CONFIG_GPL 0'))
        (prefix / 'share/midnight-ffmpeg/config_components.h').write_text(
            '#define CONFIG_H264_DECODER 1\n')
        with self.assertRaisesRegex(ValueError, 'H264_DECODER'):
            module.validate_prefix(prefix, 'linux')

    def test_mov_and_opus_are_required_by_the_playback_profile(self):
        module = self.module()
        prefix = self.prefix(module, 'win32')
        module.validate_prefix(prefix, 'win32')
        config = prefix / 'share/midnight-ffmpeg/config_components.h'
        for feature in ('MOV_DEMUXER', 'OPUS_DECODER'):
            recorded = config.read_text(encoding='utf-8')
            config.write_text(recorded.replace('CONFIG_' + feature + ' 1',
                                               'CONFIG_' + feature + ' 0'), encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'missing required'):
                module.validate_prefix(prefix, 'win32')
            config.write_text(recorded, encoding='utf-8')

    def test_unapproved_build_option_is_rejected(self):
        module = self.module()
        prefix = self.prefix(module)
        options = prefix / 'share/midnight-ffmpeg/build-options.txt'
        options.write_text(options.read_text() + '--enable-libx264\n')
        with self.assertRaisesRegex(ValueError, 'build options'):
            module.validate_prefix(prefix, 'linux')

    def test_conflicting_configuration_definitions_are_rejected(self):
        module = self.module()
        prefix = self.prefix(module)
        config = prefix / 'share/midnight-ffmpeg/config.h'
        config.write_text(config.read_text() + '#define CONFIG_GPL 1\n')
        with self.assertRaisesRegex(ValueError, 'CONFIG_GPL'):
            module.validate_prefix(prefix, 'linux')

    @unittest.skipIf(sys.platform == 'win32', 'Symlink creation requires Windows privileges')
    def test_versioned_runtime_symlink_is_accepted_only_inside_prefix(self):
        module = self.module()
        prefix = self.prefix(module)
        library = prefix / module.runtime_names('linux')[0]
        library.unlink()
        real = library.with_suffix(library.suffix + '.1')
        real.write_bytes(b'Library fixture')
        library.symlink_to(real.name)
        module.validate_prefix(prefix, 'linux')
        library.unlink()
        outside = self.root / 'outside-library'
        outside.write_bytes(b'Outside fixture')
        library.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, 'outside.*prefix'):
            module.validate_prefix(prefix, 'linux')

    @unittest.skipIf(sys.platform == 'win32', 'Symlink creation requires Windows privileges')
    def test_generated_root_and_source_archive_must_not_traverse_symlinks(self):
        module = self.module()
        (self.root / 'linked').symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'link|reparse'):
            module.build_directory(self.root / 'linked' / 'native')
        with self.assertRaisesRegex(ValueError, '_build'):
            module.build_directory(Path('/tmp/thumbnail-native'))
        archive = self.root / 'archive'
        archive.write_bytes(b'Pinned source fixture')
        (self.root / 'archive-link').symlink_to(archive)
        with self.assertRaisesRegex(ValueError, 'link|reparse'):
            module.check_source_archive(self.root / 'archive-link')

    def test_source_extraction_refuses_traversal_and_links(self):
        module = self.module()
        archive = self.root / 'source.tar.xz'
        for name, kind in (('ffmpeg-9.0.2/../escape', tarfile.REGTYPE),
                           ('ffmpeg-9.0.2/link', tarfile.SYMTYPE)):
            with self.subTest(name=name, kind=kind):
                with tarfile.open(archive, 'w:xz') as bundle:
                    member = tarfile.TarInfo(name)
                    member.type = kind
                    member.linkname = '../../outside'
                    bundle.addfile(member)
                with self.assertRaisesRegex(ValueError, 'source archive'):
                    module.extract_source(archive, self.root / 'source')
        self.assertFalse((self.root / 'escape').exists())

    def test_prebuilt_result_is_exact_json_contract_after_hash_and_policy_checks(self):
        module = self.module()
        prefix = self.prefix(module, sys.platform)
        archive = self.root / 'ffmpeg-9.0.2.tar.xz'
        archive.write_bytes(b'Verified archive fixture')
        clang = self.root / 'clang'
        clang.mkdir()
        (clang / 'libclang.so').write_bytes(b'Compiler fixture')
        environment = {'MIDNIGHT_FFMPEG_PREFIX': str(prefix),
                       'MIDNIGHT_FFMPEG_SOURCE_ARCHIVE': str(archive),
                       'LIBCLANG_PATH': str(clang)}
        with self.assertRaisesRegex(ValueError, 'checksum'):
            module.prepare(self.root / 'native', environ=environment)
        with patch.object(module, 'SOURCE_SHA256', hashlib.sha256(archive.read_bytes()).hexdigest()):
            result = module.prepare(self.root / 'native', environ=environment)
        self.assertEqual(json.loads(json.dumps(result)), {
            'prefix': str(prefix), 'source_archive': str(archive), 'libclang_path': str(clang)})

    def test_automatic_libclang_prefers_the_newest_llvm_and_skips_linked_candidates(self):
        module = self.module()
        usr = self.root / 'usr'
        for version in ('9', '18', '17'):
            library = usr / 'lib' / f'llvm-{version}' / 'lib'
            library.mkdir(parents=True)
            (library / 'libclang.so.1').write_bytes(b'Compiler fixture')
        environment = {'PATH': str(self.root / 'no-tools')}
        self.assertEqual(module.find_libclang(environment, usr), usr / 'lib' / 'llvm-18' / 'lib')
        # A distribution link below the search root (here the newest LLVM directory, which
        # holds another version) is skipped, not fatal.
        newest = directory_link(usr / 'lib' / 'llvm-19', usr / 'lib' / 'llvm-17')
        self.assertEqual(module.find_libclang(environment, usr), usr / 'lib' / 'llvm-18' / 'lib')
        # An explicit choice names its own root, so it may be reached through a link; it is
        # never replaced by an automatic candidate.
        self.assertEqual(module.find_libclang(dict(environment, LIBCLANG_PATH=str(newest / 'lib')), usr),
                         newest / 'lib')
        with self.assertRaisesRegex(ValueError, 'libclang is required'):
            module.find_libclang(dict(environment, LIBCLANG_PATH=str(usr / 'lib')), usr)

    def test_windows_prebuilt_requires_the_four_msvc_import_libraries(self):
        module = self.module()
        prefix = self.prefix(module, 'win32')
        module.validate_prefix(prefix, 'win32')
        (prefix / 'lib' / 'avformat.lib').unlink()
        with self.assertRaisesRegex(ValueError, 'avformat.lib'):
            module.validate_prefix(prefix, 'win32')

    def source_build(self, module, repository):
        """A repository whose download cache and FFmpeg build script are fixtures.

        Returns the patches that point the module at it, the build counter and script.
        """
        fixture = self.prefix(module)
        cache = repository / '.tools' / 'media-native' / ('ffmpeg-' + module.VERSION)
        cache.mkdir(parents=True)
        archive = cache / module.ARCHIVE_NAME
        with tarfile.open(archive, 'w:xz') as bundle:
            release = tarfile.TarInfo('ffmpeg-9.0.2/RELEASE')
            release.size = len(b'9.0.2\n')
            bundle.addfile(release, io.BytesIO(b'9.0.2\n'))
        for name in (module.ARCHIVE_NAME + '.asc', 'ffmpeg-devel.asc'):
            (cache / name).write_bytes(b'Acquisition verification is a separate boundary')
        (repository / 'scripts').mkdir(parents=True)
        script = repository / 'scripts' / 'build-media-ffmpeg.sh'
        count = self.root / 'build-count'
        script.write_text(
            '#!/usr/bin/env bash\nset -euo pipefail\nmkdir -p -- "$3"\n'
            + 'cp -R -- ' + shlex.quote(str(fixture) + '/.') + ' "$3/"\n'
            + 'printf "build\\n" >> ' + shlex.quote(str(count)) + '\n', encoding='utf-8')
        patches = (patch.object(module, 'CACHE_ROOT', cache), patch.object(module, 'REPOSITORY', repository),
                   patch.object(module, 'SOURCE_SHA256', hashlib.sha256(archive.read_bytes()).hexdigest()),
                   patch.object(module, 'verify_release'))
        return patches, count, script

    @unittest.skipUnless(sys.platform == 'linux', 'Linux source builder/cache integration')
    def test_cached_runtime_is_reused_until_binary_or_recipe_changes(self):
        module = self.module()
        repository = self.root / 'repository'
        patches, count, script = self.source_build(module, repository)
        clang = self.root / 'clang'
        clang.mkdir()
        (clang / 'libclang.so').write_bytes(b'Compiler fixture')
        with patches[0], patches[1], patches[2], patches[3]:
            arguments = {'offline': True, 'environ': {'LIBCLANG_PATH': str(clang)}}
            result = module.prepare(self.root / 'native', **arguments)
            self.assertEqual(count.read_text().splitlines(), ['build'])
            self.assertEqual(module.prepare(self.root / 'native', **arguments), result)
            self.assertEqual(count.read_text().splitlines(), ['build'])
            runtime = Path(result['prefix']) / module.runtime_names('linux')[0]
            runtime.write_bytes(b'Changed binary forces a rebuild')
            module.prepare(self.root / 'native', **arguments)
            self.assertEqual(len(count.read_text().splitlines()), 2)
            script.write_text(script.read_text() + '# Recipe changed\n')
            module.prepare(self.root / 'native', **arguments)
            self.assertEqual(len(count.read_text().splitlines()), 3)

    @unittest.skipUnless(sys.platform == 'linux', 'Linux source builder/cache integration')
    def test_source_build_from_a_linked_checkout_waits_out_a_held_receipt(self):
        # The checkout lies below a link (Fedora Silverblue's /home -> /var/home); its build
        # root is named through it. The receipt just written is renamed into place with the
        # retry; here a scripted Windows hold stands in for endpoint protection.
        module = self.module()
        real = self.root / 'var' / 'home' / 'checkout'
        patches, count, _ = self.source_build(module, real)
        checkout = directory_link(self.root / 'home', self.root / 'var' / 'home') / 'checkout'
        clang = self.root / 'clang'
        clang.mkdir()
        (clang / 'libclang.so').write_bytes(b'Compiler fixture')
        replace, held = Path.replace, []

        def held_receipt(path, target):
            if Path(target).name == 'verified-build.json' and not held:
                held.append(path)
                error = PermissionError(13, 'scripted hold')
                error.winerror = 32
                raise error
            return replace(path, target)
        with patches[0], patches[1], patches[2], patches[3], \
                patch.object(module, 'FRESH_FILE_HOLDS', (32,)), patch.object(Path, 'replace', held_receipt):
            result = module.prepare(checkout / '_build' / 'thumbnailers' / 'ffmpeg', offline=True,
                                    environ={'LIBCLANG_PATH': str(clang)})
        self.assertEqual(len(held), 1)
        self.assertEqual(Path(result['prefix']), checkout / '_build' / 'thumbnailers' / 'ffmpeg' / 'prefix')
        self.assertTrue((real / '_build/thumbnailers/ffmpeg/verified-build.json').is_file())
        self.assertEqual(count.read_text().splitlines(), ['build'])

    def linked_checkout(self, module):
        """A checkout below hidden AppData, reached through a link to AppData.

        The script's checkout is pointed at it, as if the script were run from there.
        """
        real = hidden_directory(self.root / 'AppData') / 'checkout'
        (real / '_build').mkdir(parents=True)
        (real / '.tools' / 'media-native').mkdir(parents=True)
        linked = directory_link(self.root / 'linked', real.parent) / 'checkout'
        patcher = patch.object(module, 'REPOSITORY', real)
        patcher.start()
        self.addCleanup(patcher.stop)
        return real, linked

    def test_build_root_follows_the_checkout_however_it_is_spelled(self):
        module = self.module()
        real, checkout = self.linked_checkout(module)
        root = checkout / '_build' / 'thumbnailers' / 'ffmpeg'
        self.assertEqual(module.build_directory(root), root)
        self.assertTrue((real / '_build' / 'thumbnailers' / 'ffmpeg').is_dir())
        outside = self.root / 'outside'
        outside.mkdir()
        directory_link(real / '_build' / 'redirect', outside)
        directory_link(real / '_build' / 'loop', real)
        # Outside the checkout the walk starts at the path's first _build, which for this
        # fixture is the repository's own _build above it; a link below that is refused.
        custom = directory_link(self.root / 'custom target', outside)
        for path in (checkout / '_build' / 'redirect' / 'ffmpeg', checkout / '_build' / 'loop' / '_build' / 'ffmpeg',
                     custom / '_build' / 'ffmpeg'):
            with self.subTest(path=path):
                with self.assertRaisesRegex(ValueError, 'link'):
                    module.build_directory(path)
        for path in (checkout / 'ffmpeg', checkout / 'sources' / '_build' / 'ffmpeg'):
            with self.subTest(path=path):
                with self.assertRaisesRegex(ValueError, 'beneath an explicit _build'):
                    module.build_directory(path)
        self.assertEqual(list(outside.iterdir()), [])
        self.assertFalse((checkout / 'ffmpeg').exists())

    def test_prefix_and_its_parents_may_be_links_but_nothing_inside_it(self):
        module = self.module()
        platform_name = 'win32' if os.name == 'nt' else 'linux'
        prefix = self.prefix(module, platform_name)
        current = directory_link(self.root / 'current', prefix)
        parent = directory_link(self.root / 'linked parent', self.root)
        for spelling in (current, parent / 'prefix'):
            with self.subTest(prefix=spelling):
                paths = module.validate_prefix(spelling, platform_name)
                self.assertTrue(all(path.is_relative_to(prefix.resolve()) for path in paths))
                self.assertEqual(set(module.artifact_hashes(paths, spelling)),
                                 set(module.artifact_hashes(paths, prefix)))
        notices = prefix / 'share' / 'midnight-ffmpeg'
        moved = self.root / 'notices elsewhere'
        notices.rename(moved)
        directory_link(notices, moved)
        with self.assertRaisesRegex(ValueError, 'link'):
            module.validate_prefix(current, platform_name)

    def test_source_archive_and_libclang_may_lie_below_links(self):
        module = self.module()
        archive = self.root / 'ffmpeg-9.0.2.tar.xz'
        archive.write_bytes(b'Pinned source fixture')
        downloads = directory_link(self.root / 'downloads', self.root)
        with patch.object(module, 'SOURCE_SHA256', hashlib.sha256(archive.read_bytes()).hexdigest()):
            self.assertEqual(module.check_source_archive(downloads / archive.name), downloads / archive.name)
        library = self.root / 'llvm-18' / 'lib'
        library.mkdir(parents=True)
        (library / 'libclang.dll').write_bytes(b'Compiler fixture')
        llvm = directory_link(self.root / 'llvm', self.root / 'llvm-18')
        self.assertEqual(module.find_libclang({'LIBCLANG_PATH': str(llvm / 'lib')}), llvm / 'lib')

    def test_download_cache_stays_a_plain_folder_inside_the_checkout(self):
        # Kept stricter than the input rule: verified downloads are opened again by path.
        module = self.module()
        real, checkout = self.linked_checkout(module)
        cached = checkout / '.tools' / 'media-native' / 'ffmpeg-devel.asc'
        cached.write_bytes(b'Cached key')
        self.assertEqual(module.fetch_artifact(cached, module.KEY_URL, 64 * 1024, offline=True), cached)
        shared = self.root / 'shared tools'
        (shared / 'media-native').mkdir(parents=True)
        (shared / 'media-native' / 'ffmpeg-devel.asc').write_bytes(b'Shared key')
        shutil.rmtree(real / '.tools')
        directory_link(real / '.tools', shared)
        with self.assertRaisesRegex(ValueError, 'link'):
            module.fetch_artifact(cached, module.KEY_URL, 64 * 1024, offline=True)
        with self.assertRaisesRegex(ValueError, 'inside the checkout'):
            module.fetch_artifact(shared / 'media-native' / 'ffmpeg-devel.asc', module.KEY_URL, 64 * 1024,
                                  offline=True)

    @unittest.skipUnless(os.name == 'nt', 'Windows refuses to rename a held file')
    def test_brief_hold_of_a_new_download_is_waited_out(self):
        module = self.module()
        cache = self.root / 'cache'
        cache.mkdir()
        timers = []

        class Response:
            """The download; once it is complete, endpoint protection holds the new file."""
            def __init__(self):
                self.blocks = [b'Downloaded key']

            def __enter__(self):
                return self

            def __exit__(self, *failure):
                return False

            def geturl(self):
                return module.KEY_URL

            def read(self, size):
                if self.blocks:
                    return self.blocks.pop()
                (download,) = cache.glob('*.download')
                timers.append(hold(download, 0.3, share_delete=False))
                return b''
        with patch.object(module.urllib.request, 'urlopen', return_value=Response()):
            path = module.fetch_artifact(cache / 'ffmpeg-devel.asc', module.KEY_URL, 64 * 1024)
        for timer in timers:
            timer.join()
        self.assertEqual(len(timers), 1)
        self.assertEqual(path.read_bytes(), b'Downloaded key')
        self.assertEqual(list(cache.glob('*.download')), [])

    @unittest.skipUnless(shutil.which('gpg'), 'GPG release-verification prerequisite')
    def test_wrong_archive_fails_before_any_gpg_import(self):
        module = self.module()
        archive = self.root / 'untrusted.tar.xz'
        archive.write_bytes(b'Untrusted source')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            module.verify_release(archive, self.root / 'absent.asc',
                                  self.root / 'absent.key', self.root)


if __name__ == '__main__':
    unittest.main()
