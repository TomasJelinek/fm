"""Native libraries coexist with archive workers in one release filesystem directory."""
from pathlib import Path
import shutil
import unittest

from scripts.tests import test_worker_packages as worker_fixtures

packaging = worker_fixtures.packaging


class NativeFilesystemPackages(unittest.TestCase):
    setUp = worker_fixtures.WorkerPackages.setUp
    filesystem = worker_fixtures.WorkerPackages.filesystem
    notices = worker_fixtures.WorkerPackages.notices
    def native(self):
        source = self.filesystem()
        package = source / "native.linux"
        package.mkdir()
        manifest = (
            'schema_version=1\nid="native.linux"\nkind="filesystem-native"\nversion="0.1.0"\n'
            f'target="{self.target}"\nprotocol=1\n'
            'entry_library="libmidnight_filesystem_native_linux.so"\nabi_major=1\n'
            'minimum_host_minor=0\naddress_encoding="unix-bytes"\n'
            'capabilities=["browse","read","stat","volumes","storage","native-address",'
            '"volume-details"]\n')
        (package / "plugin.toml").write_text(manifest)
        (package / "libmidnight_filesystem_native_linux.so").write_bytes(b"ELF fixture")
        (package / "THIRD-PARTY.md").write_text("Full upstream licence texts embedded here")
        return source, package

    def test_archive_and_native_packages_retain_separate_code_entries(self):
        source, package = self.native()
        result = packaging.copy_filesystem_packages(self.stage, source, self.target)
        self.assertEqual([item["kind"] for item in result],
                         ["filesystem-worker", "filesystem-native"])
        native = result[1]
        self.assertEqual(len(native["package_sha256"]), 64)
        binary = Path("filesystems") / package.name / native["entry_library"]
        self.assertEqual((self.stage / binary).read_bytes(), b"ELF fixture")
        entry = next(item for item in native["files"] if item["path"] == binary.as_posix())
        self.assertFalse(entry["executable"], "Shared libraries are not standalone workers")

    def test_invalid_native_schema_capabilities_encoding_and_library_are_rejected(self):
        source, package = self.native()
        path = package / "plugin.toml"
        original = path.read_text()
        for old, new in [('minimum_host_minor=0', 'minimum_host_minor=2'),
                         ('abi_major=1', 'abi_major=2'),
                         ('"unix-bytes"', '"windows-utf16le"'),
                         ('"volume-details"', '"invented"'),
                         ('"native-address"', '"read"'),
                         (self.target, 'wrong-target'),
                         ('libmidnight_filesystem_native_linux.so', '../escape.so')]:
            with self.subTest(change=new):
                path.write_text(original.replace(old, new))
                with self.assertRaises(ValueError):
                    packaging.copy_filesystem_packages(self.stage, source, self.target)
                self.assertEqual(list(self.stage.iterdir()), [])

    def test_required_native_provider_cannot_be_omitted_from_application_release(self):
        with self.assertRaises(ValueError):
            packaging.copy_filesystem_packages(self.stage, self.filesystem(), self.target,
                                               require_native=True)
        self.assertEqual(list(self.stage.iterdir()), [])

    def test_two_native_providers_are_rejected_before_copying(self):
        source, package = self.native()
        other = source / "other-native"
        shutil.copytree(package, other)
        manifest = other / "plugin.toml"
        manifest.write_text(manifest.read_text().replace('id="native.linux"', 'id="other.native"'))
        with self.assertRaises(ValueError):
            packaging.copy_filesystem_packages(self.stage, source, self.target, require_native=True)
        self.assertEqual(list(self.stage.iterdir()), [])

    def test_native_library_and_embedded_notices_are_required_before_copying(self):
        source, package = self.native()
        for name in ("libmidnight_filesystem_native_linux.so", "THIRD-PARTY.md"):
            path = package / name
            value = path.read_bytes()
            path.unlink()
            with self.subTest(missing=name), self.assertRaises(ValueError):
                packaging.copy_filesystem_packages(self.stage, source, self.target)
            self.assertEqual(list(self.stage.iterdir()), [])
            path.write_bytes(value)

    def test_native_version_uses_hosts_64_byte_utf8_limit(self):
        source, package = self.native()
        path = package / "plugin.toml"
        original = path.read_text()
        for version in ["a" * 65, "é" * 33]:
            with self.subTest(version=version):
                path.write_text(original.replace('version="0.1.0"', f'version="{version}"'),
                                encoding="utf-8")
                with self.assertRaises(ValueError):
                    packaging.copy_filesystem_packages(self.stage, source, self.target)
                self.assertEqual(list(self.stage.iterdir()), [])
        path.write_text(original.replace('version="0.1.0"', f'version="{"é" * 32}"'),
                        encoding="utf-8")
        result = packaging.copy_filesystem_packages(self.stage, source, self.target)
        self.assertEqual(result[1]["version"], "é" * 32)


if __name__ == "__main__":
    unittest.main()
