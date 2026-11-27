"""Network packages retain their opaque-address ABI and independent notices."""
from pathlib import Path
import shutil
import tomllib
import unittest

import test_native_filesystem_package as native

packaging = native.packaging


class NetworkFilesystemPackages(unittest.TestCase):
    setUp = native.NativeFilesystemPackages.setUp
    filesystem = native.NativeFilesystemPackages.filesystem
    notices = native.NativeFilesystemPackages.notices
    native = native.NativeFilesystemPackages.native

    def network(self, *, legacy=False):
        source, _ = self.native()
        package = source / "network.linux"
        package.mkdir()
        root = Path(__file__).resolve().parents[2]
        shutil.copyfile(root / "plugins/midnight-filesystem-network/plugin.linux.toml",
                        package / "plugin.toml")
        if legacy:
            path = package / "plugin.toml"
            manifest = path.read_text().replace("minimum_host_minor = 5", "minimum_host_minor = 2")
            path.write_text(manifest.replace(', "sign-in"', ''))
        (package / "libmidnight_filesystem_network.so").write_bytes(b"network library")
        (package / "third-party.md").write_text("Embedded crate and system runtime notices")
        return source, package

    def test_archive_native_and_network_keep_independent_hashes_and_notices(self):
        source, _ = self.network()
        result = packaging.copy_filesystem_packages(self.stage, source, self.target,
                                                    require_native=True, require_network=True)
        self.assertEqual([item["kind"] for item in result],
                         ["filesystem-worker", "filesystem-native", "filesystem-network"])
        entry = result[2]
        self.assertEqual(len(entry["package_sha256"]), 64)
        self.assertEqual(entry["address_encoding"], "opaque")
        self.assertEqual(entry["minimum_host_minor"], 5)
        self.assertIn("sign-in", entry["capabilities"])
        library = self.stage / "filesystems/network.linux/libmidnight_filesystem_network.so"
        self.assertEqual(library.read_bytes(), b"network library")
        self.assertFalse(next(file for file in entry["files"]
                              if file["path"].endswith(".so"))["executable"])

    def test_invalid_network_abi_and_advertised_operations_fail_before_copy(self):
        source, package = self.network()
        path = package / "plugin.toml"
        original = path.read_text()
        for old, new in [('minimum_host_minor = 5', 'minimum_host_minor = 4'),
                         ('minimum_host_minor = 5', 'minimum_host_minor = 6'),
                         ('id = "network.linux"', 'id = "unknown.network"'),
                         ('abi_major = 1', 'abi_major = 2'),
                         ('"opaque"', '"unix-bytes"'),
                         ('"activate"', '"read"'),
                         ('"refresh"', '"activate"'),
                         ('"resolve-text"', '"invented"'),
                         ('libmidnight_filesystem_network.so', '../escape.so'),
                         (self.target, 'wrong-target')]:
            with self.subTest(change=new):
                path.write_text(original.replace(old, new))
                with self.assertRaises(ValueError):
                    packaging.copy_filesystem_packages(self.stage, source, self.target)
                self.assertEqual(list(self.stage.iterdir()), [])

    def test_legacy_network_manifests_remain_compatible(self):
        source, package = self.network(legacy=True)
        path = package / "plugin.toml"
        original = path.read_text()
        for minor in (2, 3, 4, 5):
            with self.subTest(minor=minor):
                path.write_text(original.replace("minimum_host_minor = 2",
                                                 f"minimum_host_minor = {minor}"))
                stage = self.root / f"release-minor-{minor}"
                stage.mkdir()
                result = packaging.copy_filesystem_packages(stage, source, self.target,
                                                            require_network=True)
                self.assertEqual(result[2]["minimum_host_minor"], minor)
                self.assertEqual(set(result[2]["capabilities"]),
                                 {"browse", "stat", "volumes", "activate", "refresh", "resolve-text"})
                self.assertEqual(result[2]["address_encoding"], "opaque")

    def test_prepared_rename_is_rejected_for_network_at_every_supported_minor(self):
        source, package = self.network(legacy=True)
        path = package / "plugin.toml"
        original = path.read_text()
        for minor in (2, 3, 4, 5):
            for replacement in ('"prepared-rename"', '"resolve-text", "prepared-rename"'):
                with self.subTest(minor=minor, replacement=replacement):
                    text = original.replace("minimum_host_minor = 2",
                                            f"minimum_host_minor = {minor}")
                    path.write_text(text.replace('"resolve-text"', replacement))
                    with self.assertRaises(ValueError):
                        packaging.copy_filesystem_packages(self.stage, source, self.target)
                    self.assertEqual(list(self.stage.iterdir()), [])

    def test_sign_in_is_rejected_on_windows_network_manifests(self):
        _, package = self.network()
        path = package / "plugin.toml"
        target = "x86_64-pc-windows-msvc"
        library = "midnight_filesystem_network.dll"
        manifest = path.read_text().replace(self.target, target).replace("network.linux", "network.windows")
        manifest = manifest.replace("libmidnight_filesystem_network.so", library)
        entry = package / library
        entry.write_bytes(b"network library")
        files = {entry, package / "third-party.md"}
        data = tomllib.loads(manifest)
        with self.assertRaises(ValueError):
            packaging.library_filesystem_manifest(data, package, files, target)

    def test_network_library_and_notices_required_before_copy(self):
        source, package = self.network()
        for name in ("libmidnight_filesystem_network.so", "third-party.md"):
            path = package / name
            value = path.read_bytes()
            path.unlink()
            with self.subTest(missing=name), self.assertRaises(ValueError):
                packaging.copy_filesystem_packages(self.stage, source, self.target)
            self.assertEqual(list(self.stage.iterdir()), [])
            path.write_bytes(value)

    def test_release_requires_one_network_provider(self):
        source, _ = self.native()
        with self.assertRaises(ValueError):
            packaging.copy_filesystem_packages(self.stage, source, self.target,
                                               require_network=True)
        self.assertEqual(list(self.stage.iterdir()), [])

    def test_duplicate_network_provider_is_rejected_before_copy(self):
        source, package = self.network()
        other = source / "other-network"
        shutil.copytree(package, other)
        manifest = other / "plugin.toml"
        manifest.write_text(manifest.read_text().replace('id = "network.linux"',
                                                       'id = "other.network"'))
        with self.assertRaises(ValueError):
            packaging.copy_filesystem_packages(self.stage, source, self.target)
        self.assertEqual(list(self.stage.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
