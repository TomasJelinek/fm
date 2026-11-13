"""Exercise build setup without installing packages or downloading a toolchain."""

from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux build bootstrap")
class BuildSetupTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="fm build ", dir=temporary)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.repo = self.root / "checkout with spaces"
        self.repo.mkdir()
        shutil.copy2(ROOT / "build.sh", self.repo / "build.sh")
        (self.repo / "scripts").mkdir()
        for name in ("build-docker.sh", "docker-install-deps.sh", "clean-build.sh",
                     "check-clean-mounts-linux.sh", "setup-linux.sh"):
            shutil.copy2(ROOT / "scripts" / name, self.repo / "scripts" / name)
        # Package builds are separate from these host bootstrap tests.
        for name in ("build-native-filesystem.sh", "build-network-filesystem.sh", "build-viewers.sh",
                     "build-default-thumbnailers.sh"):
            (self.repo / "scripts" / name).write_text("#!/bin/sh\nexit 0\n")
        shutil.copy2(ROOT / "rust-toolchain.toml", self.repo / "rust-toolchain.toml")
        self.log = self.root / "commands"
        self.env = {
            "PATH": str(self.bin),
            "HOME": str(self.root / "home"),
            "CARGO_HOME": str(self.root / "custom cargo"),
            "TEST_LOG": str(self.log),
        }
        for name in ("dirname", "mkdir", "mktemp", "rm", "rmdir", "sh", "bash", "cat",
                     "chmod", "sha256sum", "find", "uname", "sed"):
            (self.bin / name).symlink_to(shutil.which(name))
        self.command("cc", "exit 0\n")
        self.command("pkg-config", 'exit "${NATIVE_STATUS:-0}"\n')
        self.command("id", 'case "$1" in -u) echo 1000;; -g) echo 1001;; esac\n')
        self.command("sudo", 'echo "UNEXPECTED SUDO" >> "$TEST_LOG"; exit 99\n')
        self.command("apt-get", 'echo "UNEXPECTED APT" >> "$TEST_LOG"; exit 99\n')
        self.command("docker", 'echo "docker $*" >> "$TEST_LOG"\ncase "$1" in image) exit "${IMAGE_MISSING:-0}";; create) echo setup-container;; inspect) echo "${IMAGE_STATUS:-0}";; run) exit "${BUILD_STATUS:-0}";; esac\n')
        self.cargo(self.bin / "cargo")

    def command(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/sh\nset -eu\n" + body)
        path.chmod(0o755)

    def cargo(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            '#!/bin/sh\necho "cargo $*" >> "$TEST_LOG"\n'
            'echo "cwd=$PWD tmp=$TMPDIR" >> "$TEST_LOG"\n'
            'exit "${CARGO_STATUS:-0}"\n'
        )
        path.chmod(0o755)

    def run_build(self, *args):
        result = subprocess.run(
            ["/bin/sh", str(self.repo / "build.sh"), *args],
            cwd=self.root, env=self.env, text=True, capture_output=True, timeout=20,
        )
        self.output = self.log.read_text() if self.log.exists() else ""
        return result

    def missing_packages(self):
        self.env["NATIVE_STATUS"] = "1"

    def test_clean_removes_outputs_but_preserves_docker_cache_and_source(self):
        for name in ("debug/app", "release/app", "tmp/file", ".rustc_info.json",
                     "x86_64-unknown-linux-gnu/release/app", "docker/cargo/cache", "docker/home/config"):
            path = self.repo / "_build" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("keep or clean")
        source = self.repo / "source.rs"
        source.write_text("source")
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "keep").write_text("keep")
        (self.repo / "_build/linked-output").symlink_to(outside, target_is_directory=True)
        for name in ("cargo", "docker", "sudo", "apt-get"):
            (self.bin / name).unlink()
        result = self.run_build("clean")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sorted(p.name for p in (self.repo / "_build").iterdir()), ["docker"])
        self.assertEqual((self.repo / "_build/docker/cargo/cache").read_text(), "keep or clean")
        self.assertEqual((self.repo / "_build/docker/home/config").read_text(), "keep or clean")
        self.assertEqual(source.read_text(), "source")
        self.assertEqual((outside / "keep").read_text(), "keep")
        self.assertEqual(self.output, "")

    def test_clean_is_safe_before_first_build_and_on_repeated_runs(self):
        for _ in range(2):
            result = self.run_build("clean")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse((self.repo / "_build").exists())
            self.assertEqual(self.output, "")

    def test_clean_rejects_symlinked_build_root(self):
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "keep").write_text("keep")
        (self.repo / "_build").symlink_to(outside, target_is_directory=True)
        result = self.run_build("clean")
        self.assertEqual(result.returncode, 1)
        self.assertEqual((outside / "keep").read_text(), "keep")

    def test_clean_reports_deletion_failures(self):
        output = self.repo / "_build/release/app"
        output.parent.mkdir(parents=True)
        output.write_text("binary")
        (self.bin / "rm").unlink()
        self.command("rm", "exit 1\n")
        result = self.run_build("clean")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(output.exists())
        self.assertNotIn("Cleaned", result.stdout)

    def test_missing_native_dependencies_build_in_docker_without_sudo(self):
        self.missing_packages()
        result = self.run_build("release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("docker run --rm --user 1000:1001", self.output)
        self.assertIn("sh /workspace/build.sh release", self.output)
        self.assertIn("FM_BUILD_BACKEND=native", self.output)
        self.assertIn(f'"src={self.repo}",dst=/workspace,readonly', self.output)
        self.assertNotIn("UNEXPECTED", self.output)

    def test_explicit_docker_works_even_with_native_dependencies(self):
        self.env["FM_BUILD_BACKEND"] = "docker"
        result = self.run_build("debug")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("docker run", self.output)
        self.assertNotIn("--release", self.output)

    def test_dns_override_reaches_image_setup_and_build(self):
        self.missing_packages()
        self.env["FM_DOCKER_DNS"] = "1.1.1.1"
        self.env["IMAGE_MISSING"] = "1"
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("docker create --dns 1.1.1.1", self.output)
        self.assertIn("docker commit setup-container", self.output)
        self.assertIn("--dns 1.1.1.1", self.output)

    def test_missing_docker_stops_with_actionable_error(self):
        self.missing_packages()
        (self.bin / "docker").unlink()
        result = self.run_build()
        self.assertEqual(result.returncode, 1)
        self.assertIn("Docker", result.stderr)
        self.assertNotIn("cargo build", self.output)
        self.assertNotIn("UNEXPECTED", self.output)

    def test_existing_setup_does_not_reinstall_and_preserves_exit_status(self):
        self.env["CARGO_STATUS"] = "42"
        result = self.run_build()
        self.assertEqual(result.returncode, 42)
        self.assertNotIn("apt-get", self.output)
        self.assertIn("cargo build -p midnight-explorer -p midnight-elevation --locked\n", self.output)
        self.assertIn(f"cwd={self.repo} tmp={self.repo}/_build/tmp", self.output)

    def test_finds_cargo_in_custom_home_without_reinstall(self):
        (self.bin / "cargo").unlink()
        self.cargo(Path(self.env["CARGO_HOME"]) / "bin/cargo")
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cargo build", self.output)

    def test_finds_cargo_in_default_home_without_reinstall(self):
        (self.bin / "cargo").unlink()
        del self.env["CARGO_HOME"]
        self.cargo(Path(self.env["HOME"]) / ".cargo/bin/cargo")
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cargo build", self.output)

    def test_prefers_cargo_already_on_path(self):
        other_cargo = Path(self.env["CARGO_HOME"]) / "bin/cargo"
        self.cargo(other_cargo)
        other_cargo.write_text("#!/bin/sh\nexit 99\n")
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cargo build", self.output)

    def rust_installer(self):
        (self.bin / "cargo").unlink()
        installer = self.root / "installer"
        installer.write_text(
            '#!/bin/sh\nset -eu\necho "rustup $*" >> "$TEST_LOG"\n'
            'mkdir -p "$CARGO_HOME/bin"\n'
            'cat > "$CARGO_HOME/bin/cargo" <<\'CARGO\'\n'
            '#!/bin/sh\necho "cargo $*" >> "$TEST_LOG"\nCARGO\n'
            'chmod +x "$CARGO_HOME/bin/cargo"\n'
        )
        self.env["INSTALLER"] = str(installer)
        self.command("curl", 'while [ "$#" -gt 0 ]; do\nif [ "$1" = -o ]; then cat "$INSTALLER" > "$2"; exit; fi\nshift\ndone\nexit 1\n')

    def test_bootstraps_rust_and_builds_in_same_process(self):
        self.rust_installer()
        result = self.run_build("release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("rustup -y --profile minimal --default-toolchain none\n", self.output)
        self.assertIn("cargo build -p midnight-explorer -p midnight-elevation --locked --release\n", self.output)
        self.assertEqual(list((self.repo / "_build/tmp").iterdir()), [])

    def test_unconfigured_rust_bootstrap_uses_repository_tool_cache(self):
        self.rust_installer()
        del self.env["CARGO_HOME"]
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.repo / ".tools/cargo/bin/cargo").is_file())
        self.assertFalse((Path(self.env["HOME"]) / ".cargo").exists())
        self.assertEqual(list((self.repo / "_build/tmp").iterdir()), [])

    def test_automatic_docker_dry_run_never_invokes_docker_or_cargo(self):
        self.missing_packages()
        result = self.run_build("release", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("/workspace/build.sh", result.stdout)
        self.assertIn("[release]", result.stdout)
        self.assertEqual(self.output, "")
        self.assertFalse((self.repo / "_build").exists())
        self.assertFalse((self.repo / ".tools").exists())

    def test_image_failure_stops_before_container_run(self):
        self.missing_packages()
        self.env["IMAGE_STATUS"] = "23"
        self.env["IMAGE_MISSING"] = "1"
        result = self.run_build()
        self.assertEqual(result.returncode, 23)
        self.assertNotIn("docker run", self.output)
        self.assertNotIn("docker commit", self.output)
        self.assertIn("docker rm -f setup-container", self.output)

    def test_container_exit_status_is_preserved(self):
        self.missing_packages()
        self.env["BUILD_STATUS"] = "42"
        result = self.run_build()
        self.assertEqual(result.returncode, 42)

    def test_invalid_backend_stops_before_setup(self):
        self.env["FM_BUILD_BACKEND"] = "invalid"
        result = self.run_build()
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.output, "")

    def test_download_failure_stops_before_build(self):
        (self.bin / "cargo").unlink()
        self.command("curl", "exit 22\n")
        result = self.run_build()
        self.assertEqual(result.returncode, 22)
        self.assertNotIn("cargo build", self.output)
        self.assertEqual(list((self.repo / "_build/tmp").iterdir()), [])

    def test_invalid_arguments_do_not_install_anything(self):
        self.missing_packages()
        result = self.run_build("invalid")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.output, "")


if __name__ == "__main__":
    unittest.main()
