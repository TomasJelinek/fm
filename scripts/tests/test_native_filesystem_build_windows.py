"""Native build failures must leave the previous Windows package usable."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.name == "nt", "Windows native build helper")
class NativeFilesystemBuildWindows(unittest.TestCase):
    provider = "native"

    def setUp(self):
        repository = Path(__file__).resolve().parents[2]
        temporary = repository / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="native build ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        helper = f"build-{self.provider}-filesystem.ps1"
        self.script = self.root / "scripts" / helper
        shutil.copyfile(repository / "scripts" / helper, self.script)
        shutil.copyfile(repository / "scripts/build-filesystem-package.ps1",
                        self.root / "scripts/build-filesystem-package.ps1")
        (self.root / "scripts/cargo.ps1").write_text(
            '$arguments = @($args)\n'
            'ConvertTo-Json -InputObject $arguments -Compress | '
            'Add-Content -LiteralPath $env:NATIVE_TEST_LOG\n'
            'exit ([int]$env:NATIVE_TEST_EXIT)\n')
        suffix = "-windows" if self.provider == "native" else ""
        self.plugin = self.root / f"plugins/midnight-filesystem-{self.provider}{suffix}"
        self.plugin.mkdir(parents=True)
        for name in (("plugin.toml" if self.provider == "native" else "plugin.windows.toml"),
                     "README.md", "THIRD-PARTY.md"):
            (self.plugin / name).write_text("new " + name)
        self.build = self.root / f"_build/{self.provider}-filesystems/windows"
        library = f"midnight_filesystem_{self.provider}{suffix.replace('-', '_')}.dll"
        for profile in ("debug", "release"):
            (self.build / profile).mkdir(parents=True)
            (self.build / profile / library).write_bytes(b"new DLL")
        self.application = self.root / "application"
        self.package = self.application / f"filesystems/{self.provider}.windows"
        self.package.mkdir(parents=True)
        self.library = self.package / library
        self.library.write_bytes(b"old DLL")
        for name in ("plugin.toml", "README.md", "THIRD-PARTY.md"):
            (self.package / name).write_text("old " + name)
        (self.package / "unrelated-user-file").write_bytes(b"retain")
        self.log = self.root / "calls.jsonl"
        self.environment = dict(os.environ, NATIVE_TEST_LOG=str(self.log), NATIVE_TEST_EXIT="0")
        for name in (f"MIDNIGHT_{self.provider.upper()}_FILESYSTEM_OFFLINE",
                     f"MIDNIGHT_{self.provider.upper()}_FILESYSTEM_BUILD_ROOT",
                     f"MIDNIGHT_TEST_{self.provider.upper()}_FILESYSTEM"):
            self.environment.pop(name, None)

    def run_build(self, *options):
        return subprocess.run(
            ["powershell.exe", "-NoProfile", "-File", str(self.script),
             "-AppDirectory", str(self.application), *options],
            cwd=self.root, env=self.environment, capture_output=True, text=True, timeout=30)

    def snapshot(self):
        return {path.name: path.read_bytes() for path in self.package.iterdir()}

    def test_release_test_offline_build_preserves_unrelated_files(self):
        result = self.run_build("-Profile", "release", "-Test", "-Offline")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(calls), 2)
        for call, operation in zip(calls, ("test", "build")):
            self.assertEqual(call[0], operation)
            self.assertIn("--locked", call)
            self.assertIn("--offline", call)
            self.assertIn("--release", call)
        self.assertEqual(self.library.read_bytes(), b"new DLL")
        self.assertEqual((self.package / "unrelated-user-file").read_bytes(), b"retain")
        self.assertEqual(len(list(self.package.iterdir())), 5)

    def test_missing_notice_preserves_previous_package_before_publication(self):
        (self.plugin / "README.md").unlink()
        before = self.snapshot()
        result = self.run_build()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.snapshot(), before)

    def test_failed_build_does_not_publish_any_files(self):
        before = self.snapshot()
        self.environment["NATIVE_TEST_EXIT"] = "42"
        result = self.run_build()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.snapshot(), before)

    def test_failed_tests_skip_build_and_preserve_previous_package(self):
        before = self.snapshot()
        self.environment["NATIVE_TEST_EXIT"] = "42"
        result = self.run_build("-Test")
        self.assertNotEqual(result.returncode, 0)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0][0], "test")
        self.assertEqual(self.snapshot(), before)

    def test_locked_installed_library_keeps_previous_package(self):
        before = self.snapshot()
        with self.library.open("rb") as running_host:
            result = self.run_build()
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(running_host.read(), b"old DLL")
        self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
