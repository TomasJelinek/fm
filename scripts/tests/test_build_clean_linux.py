"""Plan repository clean operations without executing destructive commands."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux build clean planning")
class LinuxBuildCleanTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="clean plan ", dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name) / "checkout with spaces"
        (self.root / "scripts").mkdir(parents=True)
        shutil.copyfile(ROOT / "build.sh", self.root / "build.sh")
        cleaner = ROOT / "scripts/clean-build.sh"
        if cleaner.exists():
            shutil.copyfile(cleaner, self.root / "scripts/clean-build.sh")
        mount_check = ROOT / "scripts/check-clean-mounts-linux.sh"
        if mount_check.exists():
            shutil.copyfile(mount_check, self.root / "scripts/check-clean-mounts-linux.sh")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "unexpected-commands"
        for name in ("cargo", "sudo", "apt-get", "dnf", "zypper", "rm", "docker"):
            path = self.bin / name
            path.write_text(f'#!/bin/sh\nprintf "{name}\\n" >> "$CLEAN_TEST_LOG"\nexit 99\n',
                            encoding="utf-8")
            path.chmod(0o755)
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + "/usr/bin:/bin",
                                CLEAN_TEST_LOG=str(self.log))

    def run_clean(self, *arguments):
        return subprocess.run(["sh", str(self.root / "build.sh"), "clean", "--dry-run", *arguments],
                              cwd=self.root.parent, env=self.environment, capture_output=True,
                              text=True, timeout=20)

    def test_plan_contains_generated_entries_but_preserves_caches_and_fixture_bytes(self):
        contents = {
            "_build/debug/me": b"current executable",
            "_build/.rustc_info.json": b"generated metadata",
            "_build/qa/result.json": b"receipt",
            "_build/docker/cargo/cache": b"legacy cache",
            ".tools/cargo/cache": b"tool cache",
            "source.rs": b"source",
        }
        for relative, data in contents.items():
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        outside = Path(self.owner.name) / "outside"
        outside.mkdir()
        marker = outside / "keep"
        marker.write_bytes(b"outside")
        (self.root / "_build/linked output").symlink_to(outside, target_is_directory=True)
        self.environment["CARGO_TARGET_DIR"] = str(outside)
        result = self.run_clean()
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in ("debug", ".rustc_info.json", "qa", "linked output"):
            self.assertIn(f"Would remove: {self.root}/_build/{name}\n", result.stdout)
        for protected in ("docker", ".tools", str(outside)):
            self.assertNotIn(f"Would remove: {self.root}/_build/{protected}\n", result.stdout)
        self.assertNotIn(f"Would remove: {outside}", result.stdout)
        for relative, data in contents.items():
            self.assertEqual((self.root / relative).read_bytes(), data)
        self.assertEqual(marker.read_bytes(), b"outside")
        self.assertTrue((self.root / "_build/linked output").is_symlink())
        self.assertFalse(self.log.exists())

    def test_absent_build_directory_is_a_noop_without_creating_output(self):
        result = self.run_clean()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "_build").exists())
        self.assertFalse(self.log.exists())

    def test_symlinked_build_root_is_refused_even_for_dry_run(self):
        outside = Path(self.owner.name) / "outside"
        outside.mkdir()
        (outside / "keep").write_bytes(b"keep")
        (self.root / "_build").symlink_to(outside, target_is_directory=True)
        result = self.run_clean()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlink", result.stderr)
        self.assertEqual((outside / "keep").read_bytes(), b"keep")
        self.assertFalse(self.log.exists())

    def test_non_directory_build_root_is_refused(self):
        (self.root / "_build").write_bytes(b"not generated directory")
        result = self.run_clean()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not a directory", result.stderr)
        self.assertEqual((self.root / "_build").read_bytes(), b"not generated directory")
        self.assertFalse(self.log.exists())

    def test_failed_mount_guard_stops_clean_planning(self):
        (self.root / "_build").mkdir()
        (self.root / "scripts/check-clean-mounts-linux.sh").write_text(
            "#!/bin/sh\nexit 47\n", encoding="utf-8")
        result = self.run_clean()
        self.assertEqual(result.returncode, 47, result.stderr)
        self.assertNotIn("Would remove:", result.stdout)
        self.assertFalse(self.log.exists())

    def test_direct_child_mounts_and_bind_mounts_are_refused_from_metadata(self):
        output = self.root / "_build"
        output.mkdir()
        for device in ("0:4", "8:1"):
            with self.subTest(device=device):
                table = self.root / "mount info"
                encoded = str(output / "mounted directory").replace(" ", r"\040")
                table.write_text(f"36 25 {device} /other {encoded} rw - ext4 /dev/example rw\n",
                                 encoding="utf-8")
                result = subprocess.run([
                    "sh", str(self.root / "scripts/check-clean-mounts-linux.sh"), str(output), str(table),
                ], env=self.environment, capture_output=True, text=True, timeout=10)
                self.assertNotEqual(result.returncode, 0, result.stderr + repr(table.read_bytes()))
                self.assertIn("mounted", result.stderr)
                self.assertIn(str(output / "mounted directory"), result.stderr)
                self.assertFalse(self.log.exists())

    def test_parent_mount_does_not_block_ordinary_output_directory(self):
        output = self.root / "_build"
        output.mkdir()
        table = self.root / "mount info"
        encoded = str(self.root).replace(" ", r"\040")
        table.write_text(f"36 25 8:1 / {encoded} rw - ext4 /dev/example rw\n", encoding="utf-8")
        result = subprocess.run([
            "sh", str(self.root / "scripts/check-clean-mounts-linux.sh"), str(output), str(table),
        ], env=self.environment, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_trailing_newline_mount_name_is_not_the_preserved_docker_cache(self):
        output = self.root / "_build"
        output.mkdir()
        table = self.root / "mount info"
        encoded = str(output).replace(" ", r"\040") + r"/docker\012"
        table.write_text(f"36 25 8:1 /other {encoded} rw - ext4 /dev/example rw\n", encoding="utf-8")
        result = subprocess.run([
            "sh", str(self.root / "scripts/check-clean-mounts-linux.sh"), str(output), str(table),
        ], env=self.environment, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0, result.stderr + repr(table.read_bytes()))
        self.assertIn("Cannot clean mounted output:", result.stderr)
        self.assertFalse(self.log.exists())

    def test_unsupported_host_stops_before_clean_or_build_commands(self):
        (self.root / "_build").mkdir()
        (self.root / "_build/output").write_bytes(b"keep")
        uname = self.bin / "uname"
        uname.write_text("#!/bin/sh\nprintf 'Darwin\\n'\n", encoding="utf-8")
        uname.chmod(0o755)
        for arguments in (("clean", "--dry-run"), ("debug",), ("release", "--install-deps")):
            with self.subTest(arguments=arguments):
                result = subprocess.run(["sh", str(self.root / "build.sh"), *arguments],
                                        env=self.environment, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("Linux", result.stderr)
                self.assertEqual(result.stdout, "")
        self.assertEqual((self.root / "_build/output").read_bytes(), b"keep")
        self.assertFalse(self.log.exists())

    def test_owned_fixture_clean_never_walks_retained_cache_or_external_symlink(self):
        output = self.root / "_build"
        self.assertNotEqual(output.resolve(), (ROOT / "_build").resolve())
        self.assertTrue(output.resolve().is_relative_to((ROOT / "_build/tmp").resolve()))
        cache = output / "docker"
        cache.mkdir(parents=True)
        (cache / "keep").write_bytes(b"retained cache sentinel")
        outside = Path(self.owner.name) / "outside"
        outside.mkdir()
        (outside / "keep").write_bytes(b"external sentinel")
        (output / "linked output").symlink_to(outside, target_is_directory=True)
        (output / "debug").mkdir()
        (output / "debug/obsolete").write_bytes(b"owned disposable output")
        (output / ".hidden").write_bytes(b"owned disposable metadata")
        source = self.root / "source.rs"
        source.write_bytes(b"source sentinel")
        # Simulate EACCES even on WSL drives which do not enforce chmod(000).
        find = self.bin / "find"
        find.write_text('#!/bin/sh\ncase "$1" in\n'
                        ' .|"$CLEAN_TEST_OUTPUT"|"$CLEAN_TEST_OUTPUT/."|'
                        '"$CLEAN_TEST_OUTPUT/docker"|"$CLEAN_TEST_OUTPUT/docker/"*)\n'
                        ' printf "protected cache traversal attempted\\n" >&2; exit 97;;\n'
                        'esac\nexec /usr/bin/find "$@"\n', encoding="utf-8")
        find.chmod(0o755)
        (self.bin / "rm").unlink()  # Real removal is authorized only for this new owned fixture.
        cache.chmod(0o000)
        self.addCleanup(cache.chmod, 0o700)
        environment = dict(self.environment, CLEAN_TEST_OUTPUT=str(output))
        result = subprocess.run(["sh", str(self.root / "build.sh"), "clean"],
                                cwd=self.root.parent, env=environment, capture_output=True,
                                text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        cache.chmod(0o700)
        self.assertEqual(sorted(path.name for path in output.iterdir()), ["docker"])
        self.assertEqual((cache / "keep").read_bytes(), b"retained cache sentinel")
        self.assertEqual((outside / "keep").read_bytes(), b"external sentinel")
        self.assertEqual(source.read_bytes(), b"source sentinel")
        self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
