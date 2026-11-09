"""Rebuilding a native provider must preserve the inode used by a running host."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux native publication")
class NativeFilesystemBuildLinux(unittest.TestCase):
    def setUp(self):
        repository = Path(__file__).resolve().parents[2]
        temporary = repository / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="native build ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        self.script = self.root / "scripts/build-native-filesystem.sh"
        shutil.copyfile(repository / "scripts/build-native-filesystem.sh", self.script)
        self.plugin = self.root / "plugins/midnight-filesystem-native-linux"
        self.plugin.mkdir(parents=True)
        for name in ("plugin.toml", "README.md", "THIRD-PARTY.md"):
            (self.plugin / name).write_text("new " + name)
        self.build = self.root / "_build/native-filesystems/linux"
        for profile in ("debug", "release"):
            (self.build / profile).mkdir(parents=True)
            (self.build / profile / "libmidnight_filesystem_native_linux.so").write_bytes(b"new library bytes")
        self.application = self.root / "application"
        self.package = self.application / "filesystems/native.linux"
        self.package.mkdir(parents=True)
        self.library = self.package / "libmidnight_filesystem_native_linux.so"
        self.library.write_bytes(b"original mapped library")
        for name in ("plugin.toml", "README.md", "THIRD-PARTY.md"):
            (self.package / name).write_text("old " + name)
        self.unrelated = self.package / "unrelated-user-file"
        self.unrelated.write_bytes(b"retain unrelated entry")
        fake_bin = self.root / "bin"
        fake_bin.mkdir()
        cargo = fake_bin / "cargo"
        cargo.write_text('#!/bin/sh\nprintf "%s\\n" "$@" >> "$NATIVE_TEST_LOG"\n'
                         'printf "%s\\n" "--end--" >> "$NATIVE_TEST_LOG"\nexit "$NATIVE_TEST_EXIT"\n')
        cargo.chmod(0o755)
        self.log = self.root / "cargo-arguments"
        self.environment = dict(os.environ, PATH=str(fake_bin) + os.pathsep + os.environ["PATH"],
                                NATIVE_TEST_LOG=str(self.log), NATIVE_TEST_EXIT="0")
        for name in ("CARGO_TARGET_DIR", "MIDNIGHT_NATIVE_FILESYSTEM_BUILD_ROOT",
                     "MIDNIGHT_TEST_NATIVE_FILESYSTEM", "MIDNIGHT_NATIVE_FILESYSTEM_OFFLINE"):
            self.environment.pop(name, None)

    def run_build(self, profile="debug"):
        return subprocess.run(["bash", str(self.script), profile, str(self.application)],
                              cwd=self.root, env=self.environment, capture_output=True,
                              text=True, timeout=20)

    def test_rebuild_replaces_path_without_rewriting_open_library_inode(self):
        with self.library.open("rb") as running_host:
            original = os.fstat(running_host.fileno())
            result = self.run_build()
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(running_host.read(), b"original mapped library")
            self.assertNotEqual((original.st_dev, original.st_ino),
                                (self.library.stat().st_dev, self.library.stat().st_ino))
        self.assertEqual(self.library.read_bytes(), b"new library bytes")
        self.assertEqual((self.package / "plugin.toml").read_text(), "new plugin.toml")
        self.assertEqual(self.unrelated.read_bytes(), b"retain unrelated entry")
        self.assertEqual(len(list(self.package.iterdir())), 5, "No owned staging entries remain")

    def test_staging_failure_preserves_previous_package_and_only_cleans_owned_temp(self):
        # Failure after the library could already have copied, before publication.
        (self.plugin / "README.md").unlink()
        before = {path.name: path.read_bytes() for path in self.package.iterdir()}
        with self.library.open("rb") as running_host:
            result = self.run_build()
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(running_host.read(), b"original mapped library")
        self.assertEqual({path.name: path.read_bytes() for path in self.package.iterdir()}, before)

    def test_release_tests_and_build_receive_locked_offline_manifest_arguments(self):
        self.environment.update(MIDNIGHT_TEST_NATIVE_FILESYSTEM="1",
                                MIDNIGHT_NATIVE_FILESYSTEM_OFFLINE="1")
        result = self.run_build("release")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.log.read_text().split("--end--\n")
        self.assertEqual(len(calls), 3)
        for call, operation in zip(calls, ("test", "build")):
            self.assertEqual(call.splitlines(), [operation, "--locked", "--manifest-path",
                                                str(self.plugin / "Cargo.toml"), "--offline", "--release"])

    def test_failed_tests_skip_build_and_preserve_previous_package(self):
        self.environment.update(MIDNIGHT_TEST_NATIVE_FILESYSTEM="1", NATIVE_TEST_EXIT="42")
        before = {path.name: path.read_bytes() for path in self.package.iterdir()}
        result = self.run_build()
        self.assertEqual(result.returncode, 42, result.stdout + result.stderr)
        self.assertEqual(self.log.read_text().splitlines()[0], "test")
        self.assertEqual(self.log.read_text().count("--end--\n"), 1)
        self.assertEqual({path.name: path.read_bytes() for path in self.package.iterdir()}, before)


if __name__ == "__main__":
    unittest.main()
