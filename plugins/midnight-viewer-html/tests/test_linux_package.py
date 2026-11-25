"""Independent Linux package safety tests; no host implementation is imported."""
import ctypes
import json
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

PROJECT = Path(__file__).resolve().parents[1]
ROOT = PROJECT.parents[1]
SPEC = importlib.util.spec_from_file_location('linux_package', PROJECT / 'scripts/package-linux.py')
sys.dont_write_bytecode = True
PACKAGER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGER)


class LinuxBuildValidation(unittest.TestCase):
    def test_relative_sdk_is_canonicalized_before_cargo(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=work) as tmp:
            tmp = Path(tmp)
            project = tmp / 'project'
            (project / 'scripts').mkdir(parents=True)
            for name in ['build-linux.sh', 'package-linux.py']:
                shutil.copy2(PROJECT / 'scripts' / name, project / 'scripts' / name)
            sdk = tmp / 'sdk 文書\n'
            sdk.mkdir()
            metadata = {'type': 'minimal', 'name': 'fixture.tar.bz2', 'sha1': 'published'}
            (sdk / 'archive.json').write_text(json.dumps(metadata))
            (project / 'scripts/cef-linux-pinned.json').write_text(json.dumps({
                'archive': metadata['name'], 'sha1': metadata['sha1'],
                'files': {'archive.json': PACKAGER.digest(sdk / 'archive.json')}}))
            tools = tmp / 'bin'
            tools.mkdir()
            marker = tmp / 'cef-path-seen-by-cargo'
            cargo = tools / 'cargo'
            cargo.write_text('#!/bin/sh\nprintf "%s" "$CEF_PATH" >"' + str(marker) + '"\nexit 99\n')
            cargo.chmod(0o755)
            config = tmp / 'config.toml'
            config.write_text('')
            env = dict(os.environ, CEF_PATH=sdk.name, PATH=f'{tools}:{os.environ["PATH"]}',
                       MIDNIGHT_HTML_TARGET_DIR=str(tmp / 'target'))
            subprocess.run(['bash', str(project / 'scripts/build-linux.sh'), str(config)],
                           cwd=tmp, env=env, capture_output=True, text=True)
            self.assertEqual(marker.read_text(), str(sdk.resolve()),
                             'cef-dll-sys resolves relative input in its own crate directory')

    def test_package_preserves_native_and_rust_notices_with_hash_inventory(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=work) as tmp:
            tmp = Path(tmp)
            sdk = tmp / 'sdk'
            (sdk / 'locales').mkdir(parents=True)
            (sdk / 'libcef.so').write_bytes(b'native runtime fixture')
            (sdk / 'LICENSE.txt').write_text('CEF license fixture')
            (sdk / 'CREDITS.html').write_text('Bundled component notices fixture')
            (sdk / 'locales/en-US.pak').write_bytes(b'locale fixture')
            (sdk / 'locales/de.pak').write_bytes(b'unused locale')
            dependency = tmp / 'dependency'
            (dependency / 'nested').mkdir(parents=True)
            (dependency / 'LICENSE-MIT').write_text('Dependency license fixture')
            (dependency / 'nested/NOTICE').write_text('Bundled dependency notice fixture')
            metadata = tmp / 'metadata.json'
            metadata.write_text(json.dumps({'resolve': {'nodes': [{'id': 'dependency'}]},
                'packages': [{'id': 'dependency', 'name': 'fixture', 'version': '1.0.0',
                    'manifest_path': str(dependency / 'Cargo.toml'), 'license': 'MIT'}]}))
            launcher = tmp / 'launcher'
            launcher.write_bytes(b'launcher fixture')
            pin = {'files': {p.relative_to(sdk).as_posix(): PACKAGER.digest(p)
                    for p in sdk.rglob('*') if p.is_file()},
                   'cef': 'pinned fixture', 'archive_sha256': 'a' * 64}
            destination = tmp / 'package'
            PACKAGER.package(sdk, destination, launcher, metadata, pin, None)
            self.assertEqual((destination / 'notices/fixture-1.0.0/nested/NOTICE').read_text(),
                             'Bundled dependency notice fixture')
            self.assertEqual((destination / 'CREDITS.html').read_text(),
                             'Bundled component notices fixture')
            inventory = json.loads((destination / 'runtime-files.json').read_text())
            indexed = {entry['path']: entry for entry in inventory['files']}
            self.assertEqual(indexed['libcef.so']['sha256'], PACKAGER.digest(sdk / 'libcef.so'))
            self.assertIn('dependency-metadata.json', indexed)
            self.assertIn('locales/en-US.pak', indexed)
            self.assertEqual([p.name for p in (destination / 'locales').iterdir()], ['en-US.pak'])
            # The provider's own GPL licence travels beside CEF's LICENSE.txt.
            self.assertEqual((destination / 'LICENSE').read_bytes(), (PROJECT / 'LICENSE').read_bytes())
            self.assertEqual(indexed['LICENSE']['sha256'], PACKAGER.digest(PROJECT / 'LICENSE'))
            self.assertEqual((destination / 'LICENSE.txt').read_text(), 'CEF license fixture')
            with self.assertRaises(FileExistsError):
                PACKAGER.package(sdk, destination, launcher, metadata, pin, None)

    @unittest.skipUnless(sys.platform.startswith('linux') and shutil.which('cc') and shutil.which('strip'),
                         'Linux native strip and load check')
    def test_release_strips_staging_copy_and_hashes_loadable_result(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=work) as temp:
            root = Path(temp)
            sdk = root / 'sdk'
            (sdk / 'locales').mkdir(parents=True)
            (sdk / 'locales/en-US.pak').write_bytes(b'English locale')
            library = sdk / 'libcef.so'
            subprocess.run(['cc', '-shared', '-fPIC', '-g', '-x', 'c', '-', '-o', library],
                           input=b'int probe(void) { return 17; }', check=True)
            original = PACKAGER.digest(library)
            pin = {'cef': 'fixture', 'archive_sha256': 'a' * 64, 'files': {'libcef.so': original}}
            metadata = root / 'metadata.json'
            metadata.write_text('{"resolve":{"nodes":[]},"packages":[]}')
            stage = root / 'package'
            PACKAGER.package(sdk, stage, Path(sys.executable), metadata, pin, None, 'release')
            self.assertEqual(PACKAGER.digest(library), original)
            stripped = stage / 'libcef.so'
            self.assertLess(stripped.stat().st_size, library.stat().st_size)
            self.assertEqual(ctypes.CDLL(str(stripped)).probe(), 17)
            inventory = json.loads((stage / 'runtime-files.json').read_text())
            entry = next(file for file in inventory['files'] if file['path'] == 'libcef.so')
            self.assertEqual(entry['sha256'], PACKAGER.digest(stripped))

    @staticmethod
    def link(link, target):
        """A directory symlink, or a junction on Windows, where symlinks need a privilege."""
        target.mkdir(parents=True, exist_ok=True)
        link.parent.mkdir(parents=True, exist_ok=True)
        if os.name == 'nt':
            import _winapi
            _winapi.CreateJunction(str(target), str(link))
        else:
            link.symlink_to(target, target_is_directory=True)
        return link

    def test_dependency_notices_are_found_and_read_without_following_links(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=work) as tmp:
            tmp = Path(tmp)
            # A linked cargo home (/home -> /var/home, ~/.cargo -> /data/cargo) and a crate
            # folder that is itself a link are accepted: they lie at or above the root.
            home = self.link(tmp / 'home/.cargo', tmp / 'disk/cargo')
            crate = home / 'registry/src/index-0000/fixture-1.0.0'
            (crate / 'nested').mkdir(parents=True)
            (crate / 'LICENSE-MIT').write_bytes(b'Dependency license fixture')
            (crate / 'nested/NOTICE').write_bytes(b'Bundled dependency notice fixture')
            (crate / 'lib.rs').write_bytes(b'not a notice')
            expected = {Path('LICENSE-MIT'): b'Dependency license fixture',
                        Path('nested/NOTICE'): b'Bundled dependency notice fixture'}
            for root in (crate, self.link(tmp / 'registry/fixture-1.0.0', crate)):
                with self.subTest(root=root):
                    self.assertEqual(PACKAGER.notice_files(root), expected)
            # Below the root nothing is followed: a linked folder anywhere is refused.
            outside = tmp / 'outside'
            outside.mkdir()
            (outside / 'LICENSE').write_bytes(b'Unrelated private content')
            self.link(crate / 'nested/vendor', outside)
            with self.assertRaisesRegex(ValueError, 'must not traverse links: .*vendor'):
                PACKAGER.notice_files(crate)

    @unittest.skipIf(os.name == 'nt', 'File symlinks need a privilege on Windows')
    def test_linked_license_file_is_refused(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=work) as tmp:
            tmp = Path(tmp)
            (tmp / 'outside').write_bytes(b'Unrelated private content')
            (tmp / 'dependency').mkdir()
            (tmp / 'dependency/LICENSE').symlink_to(tmp / 'outside')
            with self.assertRaisesRegex(ValueError, 'must not traverse links: .*LICENSE'):
                PACKAGER.notice_files(tmp / 'dependency')

    def test_versioned_fallback_directory_is_rejected(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=work) as tmp:
            sdk = Path(tmp)
            metadata = {'type': 'minimal', 'name': 'pinned.tar.bz2', 'sha1': 'published'}
            (sdk / 'archive.json').write_text(json.dumps(metadata))
            pin = {'archive': metadata['name'], 'sha1': metadata['sha1'],
                   'files': {'archive.json': PACKAGER.digest(sdk / 'archive.json')}}
            PACKAGER.validate_sdk(sdk, pin)
            (sdk / '156.0.26').mkdir()
            with self.assertRaisesRegex(ValueError, 'unexpected SDK entry'):
                PACKAGER.validate_sdk(sdk, pin)

    def test_native_runtime_requires_pinned_hashes(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=work) as tmp:
            source = Path(tmp) / 'lib.so'
            source.write_bytes(b'unapproved')
            manifest = {'files': [{'source': str(source), 'path': 'lib.so', 'sha256': '0' * 64}]}
            validate = getattr(PACKAGER, 'validate_native', None)
            self.assertIsNotNone(validate, 'Native sibling runtime input needs digest validation')
            with self.assertRaisesRegex(ValueError, 'native runtime SHA-256 mismatch'):
                validate(manifest)

    def test_incomplete_or_corrupt_sdk_is_rejected(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=work) as tmp:
            sdk = Path(tmp)
            metadata = {'type': 'minimal', 'name': 'pinned.tar.bz2', 'sha1': 'published'}
            (sdk / 'archive.json').write_text(json.dumps(metadata))
            pin = {'archive': metadata['name'], 'sha1': metadata['sha1'],
                   'files': {'libcef.so': '0' * 64}}
            with self.assertRaisesRegex(ValueError, 'missing regular file'):
                PACKAGER.validate_sdk(sdk, pin)
            (sdk / 'libcef.so').write_bytes(b'changed runtime')
            with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                PACKAGER.validate_sdk(sdk, pin)

    def test_invalid_sdk_never_reaches_cargo(self):
        work = ROOT / '_build/qa/html-linux/tests'
        work.mkdir(parents=True, exist_ok=True)
        for metadata in [None, {}, {'type': 'minimal', 'name': 'wrong', 'sha1': 'wrong'}]:
            with self.subTest(metadata=metadata), tempfile.TemporaryDirectory(dir=work) as tmp:
                tmp = Path(tmp)
                sdk = tmp / 'sdk'
                sdk.mkdir()
                if metadata is not None:
                    (sdk / 'archive.json').write_text(json.dumps(metadata))
                tools = tmp / 'bin'
                tools.mkdir()
                marker = tmp / 'cargo-called'
                cargo = tools / 'cargo'
                cargo.write_text('#!/bin/sh\ntouch "' + str(marker) + '"\nexit 99\n')
                cargo.chmod(0o755)
                config = tmp / 'config.toml'
                config.write_text('')
                env = dict(os.environ, CEF_PATH=str(sdk), PATH=f'{tools}:{os.environ["PATH"]}',
                           MIDNIGHT_HTML_TARGET_DIR=str(tmp / 'target'))
                result = subprocess.run(['bash', str(PROJECT / 'scripts/build-linux.sh'),
                                         str(config)], env=env, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(marker.exists(), result.stderr)


if __name__ == '__main__':
    unittest.main()
