"""Independent Linux package safety tests; no host implementation is imported."""
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
            with self.assertRaises(FileExistsError):
                PACKAGER.package(sdk, destination, launcher, metadata, pin, None)

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
