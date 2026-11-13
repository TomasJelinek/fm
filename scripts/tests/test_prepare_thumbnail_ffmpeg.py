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
            '#define CONFIG_RAWVIDEO_DECODER 1\n#define CONFIG_AVI_DEMUXER 1\n',
            encoding='utf-8')
        for name in ('COPYING.LGPLv2.1', 'LICENSE.md', 'config.mak'):
            (notices / name).write_text('Pinned source notice\n', encoding='utf-8')
        for name in module.runtime_names(platform_name):
            path = prefix / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'Runtime fixture')
        for name in ('avcodec', 'avformat', 'avutil', 'swscale'):
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
        prefix = self.prefix(module)
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

    def test_windows_prebuilt_requires_the_four_msvc_import_libraries(self):
        module = self.module()
        prefix = self.prefix(module, 'win32')
        module.validate_prefix(prefix, 'win32')
        (prefix / 'lib' / 'avformat.lib').unlink()
        with self.assertRaisesRegex(ValueError, 'avformat.lib'):
            module.validate_prefix(prefix, 'win32')

    @unittest.skipUnless(sys.platform == 'linux', 'Linux source builder/cache integration')
    def test_cached_runtime_is_reused_until_binary_or_recipe_changes(self):
        module = self.module()
        fixture = self.prefix(module)
        cache = self.root / 'cache'
        cache.mkdir()
        archive = cache / module.ARCHIVE_NAME
        with tarfile.open(archive, 'w:xz') as bundle:
            release = tarfile.TarInfo('ffmpeg-9.0.2/RELEASE')
            release.size = len(b'9.0.2\n')
            bundle.addfile(release, io.BytesIO(b'9.0.2\n'))
        for name in (module.ARCHIVE_NAME + '.asc', 'ffmpeg-devel.asc'):
            (cache / name).write_bytes(b'Acquisition verification is a separate boundary')
        repository = self.root / 'repository'
        (repository / 'scripts').mkdir(parents=True)
        script = repository / 'scripts' / 'build-media-ffmpeg.sh'
        count = self.root / 'build-count'
        script.write_text(
            '#!/usr/bin/env bash\nset -euo pipefail\nmkdir -p -- "$3"\n'
            + 'cp -R -- ' + shlex.quote(str(fixture) + '/.') + ' "$3/"\n'
            + 'printf "build\\n" >> ' + shlex.quote(str(count)) + '\n', encoding='utf-8')
        clang = self.root / 'clang'
        clang.mkdir()
        (clang / 'libclang.so').write_bytes(b'Compiler fixture')
        with patch.object(module, 'CACHE_ROOT', cache), \
                patch.object(module, 'REPOSITORY', repository), \
                patch.object(module, 'SOURCE_SHA256', hashlib.sha256(archive.read_bytes()).hexdigest()), \
                patch.object(module, 'verify_release'):
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
