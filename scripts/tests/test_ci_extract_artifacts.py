"""CI extracts downloaded artifact archives itself, refusing unsafe or colliding members."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import stat
import tempfile
import unittest
import zipfile

SCRIPT = Path(__file__).resolve().parents[2] / '.github/actions/download-artifacts/extract.py'
spec = importlib.util.spec_from_file_location('ci_extract_artifacts', SCRIPT)
extractor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(extractor)


def archive(path, members, compression=zipfile.ZIP_DEFLATED):
    """Write a zip of (name, bytes) members; a name ending with '/' is a directory."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, 'w', compression) as output:
        for name, data in members:
            output.writestr(name, data)
    return path


def raw_member(path, name, data=b'x', mode=None):
    """A zip with one member whose stored name is exactly `name` (ZipInfo would normalise it)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    info = zipfile.ZipInfo('placeholder')
    info.filename = name
    if mode is not None:
        info.external_attr = mode << 16
    with zipfile.ZipFile(path, 'w') as output:
        output.writestr(info, data)
    return path


class ExtractTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.archives = self.root / 'archives'
        self.destination = self.root / 'out'

    def run_main(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = extractor.main([str(self.archives), str(self.destination)])
        return code, output.getvalue()

    def test_nested_and_hidden_files_are_extracted_with_their_bytes(self):
        archive(self.archives / 'released-plugin-sdk.zip', [
            ('.cargo/config.toml', b'[registries]\n'), ('registry/', b''),
            ('include/midnight/viewer.h', b'#pragma once\n'), ('artifacts.json', b'{}')])
        code, output = self.run_main()
        self.assertEqual(code, 0, output)
        self.assertEqual((self.destination / '.cargo/config.toml').read_bytes(), b'[registries]\n')
        self.assertEqual((self.destination / 'include/midnight/viewer.h').read_bytes(), b'#pragma once\n')
        self.assertTrue((self.destination / 'registry').is_dir())
        self.assertIn('Extracted 3 files (28 bytes) from 1 archive(s)', output)

    def test_archives_in_per_artifact_directories_merge_into_one_destination(self):
        archive(self.archives / 'viewer-Linux-basic/viewer-Linux-basic.zip',
                [('midnight-viewer-basic/plugin.toml', b'basic')])
        archive(self.archives / 'viewer-Linux-pdf/viewer-Linux-pdf.zip',
                [('midnight-viewer-pdf/plugin.toml', b'pdf'), ('midnight-viewer-pdf/midnight-viewer-pdf', b'elf')])
        code, output = self.run_main()
        self.assertEqual(code, 0, output)
        self.assertEqual(sorted(str(path.relative_to(self.destination)).replace('\\', '/')
                                for path in self.destination.rglob('*') if path.is_file()),
                         ['midnight-viewer-basic/plugin.toml', 'midnight-viewer-pdf/midnight-viewer-pdf',
                          'midnight-viewer-pdf/plugin.toml'])

    def test_unsafe_member_names_fail_before_anything_is_written(self):
        for name in ('/etc/passwd', '../escape', 'a/../../escape', 'a/./b', 'a//b', 'C:/escape', 'C:escape',
                     'file:stream', 'a\\..\\escape', 'nul\0byte', '.', '..', ''):
            with self.subTest(name=name):
                self.setUp()
                archive(self.archives / 'a-good.zip', [('good.txt', b'kept out')])
                raw_member(self.archives / 'b-bad.zip', name)
                code, output = self.run_main()
                self.assertEqual(code, 1)
                self.assertIn('::error::Artifact extraction failed: Unsafe member name', output)
                self.assertFalse(self.destination.exists(), 'validated before any write')

    def test_link_and_encrypted_members_fail(self):
        raw_member(self.archives / 'link.zip', 'link', b'/etc/passwd', mode=stat.S_IFLNK | 0o777)
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn("Link member 'link'", output)
        self.assertFalse(self.destination.exists())
        (self.archives / 'link.zip').unlink()
        # zipfile cannot encrypt; set the encryption flag in the local and central headers.
        path = archive(self.archives / 'encrypted.zip', [('secret.txt', b'data')])
        data = bytearray(path.read_bytes())
        for signature, offset in ((b'PK\x03\x04', 6), (b'PK\x01\x02', 8)):
            start = data.index(signature) + offset
            data[start] |= 0x1
        path.write_bytes(bytes(data))
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn("Encrypted member 'secret.txt'", output)
        self.assertFalse(self.destination.exists())

    def test_a_second_member_at_one_path_fails_instead_of_overwriting(self):
        archive(self.archives / 'one/one.zip', [('shared/plugin.toml', b'first')])
        archive(self.archives / 'two/two.zip', [('shared/plugin.toml', b'second')])
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn('Two archive members at one path', output)
        self.assertEqual((self.destination / 'shared/plugin.toml').read_bytes(), b'first')

    def test_destination_may_hold_empty_directories_but_no_file(self):
        # A restored rust-cache leaves the directories of _build/viewer-packages, not its files.
        archive(self.archives / 'a.zip', [('midnight-viewer-basic/plugin.toml', b'data')])
        (self.destination / 'midnight-viewer-basic/empty').mkdir(parents=True)
        (self.destination / 'midnight-viewer-basic/empty/old').write_bytes(b'old')
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn('Destination already holds files', output)
        self.assertFalse((self.destination / 'midnight-viewer-basic/plugin.toml').exists())
        (self.destination / 'midnight-viewer-basic/empty/old').unlink()
        code, output = self.run_main()
        self.assertEqual(code, 0, output)
        self.assertEqual((self.destination / 'midnight-viewer-basic/plugin.toml').read_bytes(), b'data')
        self.destination.joinpath('midnight-viewer-basic/plugin.toml').unlink()
        self.destination.joinpath('midnight-viewer-basic/empty').rmdir()
        self.destination.joinpath('midnight-viewer-basic').rmdir()
        self.destination.rmdir()
        self.destination.write_bytes(b'not a directory')
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn('Destination already holds files', output)

    def test_missing_non_zip_and_corrupt_archives_fail(self):
        self.archives.mkdir()
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn('No artifact archive', output)
        (self.archives / 'readme.txt').write_bytes(b'not a zip')
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn('Not a zip archive', output)
        (self.archives / 'readme.txt').unlink()
        payload = b'payload that the CRC protects'
        path = archive(self.archives / 'corrupt.zip', [('data.bin', payload)], zipfile.ZIP_STORED)
        data = path.read_bytes()
        offset = data.index(payload)
        path.write_bytes(data[:offset] + b'P' + data[offset + 1:])
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn('Bad CRC-32', output)

    @unittest.skipUnless(hasattr(os, 'symlink'), 'no symlinks')
    def test_a_link_among_the_downloads_fails(self):
        archive(self.archives / 'real/real.zip', [('file', b'data')])
        try:
            os.symlink(self.archives / 'real', self.archives / 'linked', target_is_directory=True)
        except OSError:
            self.skipTest('symlink creation is not permitted')
        code, output = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn('Link in the downloaded artifacts', output)

    def test_usage_without_two_arguments(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(extractor.main([]), 2)


if __name__ == '__main__':
    unittest.main()
