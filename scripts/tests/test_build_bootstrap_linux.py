"""build.sh bootstraps Rust, selects its backend and cleans output without installing packages.

Argument, backend, Docker and clean-planning edge cases live in test_build_setup_linux.py,
test_build_docker_linux.py and test_build_clean_linux.py."""

from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


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
        for name in ("build-docker.sh", "docker-install-deps.sh", "install-rust-toolchain.sh", "clean-build.sh",
                     "check-clean-mounts-linux.sh", "setup-linux.sh"):
            shutil.copy2(ROOT / "scripts" / name, self.repo / "scripts" / name)
        # Package builds are separate from these host bootstrap tests.
        for name in ("build-native-filesystem.sh", "build-network-filesystem.sh", "build-viewers.sh",
                     "build-default-thumbnailers.sh", "build-archive-filesystem.sh"):
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

    def test_existing_setup_does_not_reinstall_and_preserves_exit_status(self):
        self.env["CARGO_STATUS"] = "42"
        result = self.run_build()
        self.assertEqual(result.returncode, 42)
        self.assertNotIn("apt-get", self.output)
        self.assertIn("cargo build -p midnight-explorer -p midnight-elevation --locked\n", self.output)
        self.assertIn(f"cwd={self.repo} tmp={self.repo}/_build/tmp", self.output)

    def test_prefers_cargo_already_on_path(self):
        other_cargo = Path(self.env["CARGO_HOME"]) / "bin/cargo"
        self.cargo(other_cargo)
        other_cargo.write_text("#!/bin/sh\nexit 99\n")
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cargo build", self.output)

    def rust_installer(self):
        (self.bin / "cargo").unlink()
        # The pinned toolchain is read from rust-toolchain.toml with python3.
        (self.bin / "python3").symlink_to(shutil.which("python3"))
        installer = self.root / "installer"
        installer.write_text(
            '#!/bin/sh\nset -eu\necho "rustup-init $*" >> "$TEST_LOG"\n'
            'mkdir -p "$CARGO_HOME/bin"\n'
            'for tool in cargo rustup; do\n'
            '    printf \'#!/bin/sh\\necho "%s $*" >> "$TEST_LOG"\\n\' "$tool" > "$CARGO_HOME/bin/$tool"\n'
            '    chmod +x "$CARGO_HOME/bin/$tool"\n'
            'done\n'
        )
        self.env["INSTALLER"] = str(installer)
        self.command("curl", 'while [ "$#" -gt 0 ]; do\nif [ "$1" = -o ]; then cat "$INSTALLER" > "$2"; exit; fi\nshift\ndone\nexit 1\n')

    def test_bootstraps_rust_and_builds_in_same_process(self):
        self.rust_installer()
        result = self.run_build("release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("rustup-init -y --profile minimal --default-toolchain none\n", self.output)
        # Explicitly, not through proxy auto-install, which users can disable.
        install = "rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy\n"
        build = "cargo build -p midnight-explorer -p midnight-elevation --locked --release\n"
        self.assertIn(install, self.output)
        self.assertLess(self.output.index(install), self.output.index(build))
        self.assertEqual(list((self.repo / "_build/tmp").iterdir()), [])

    def test_failed_toolchain_installation_stops_before_build(self):
        self.rust_installer()
        (self.repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "--force"\n')
        result = self.run_build()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid channel", result.stderr)
        self.assertNotIn("cargo build", self.output)

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

    def test_download_failure_stops_before_build(self):
        (self.bin / "cargo").unlink()
        self.command("curl", "exit 22\n")
        result = self.run_build()
        self.assertEqual(result.returncode, 22)
        self.assertNotIn("cargo build", self.output)
        self.assertEqual(list((self.repo / "_build/tmp").iterdir()), [])

    def test_missing_module_prerequisites_are_reported_before_the_host_build(self):
        # GUI libraries alone select a native build; the module tools are named
        # up front instead of failing after the complete host build.
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("missing native module build prerequisites: python3 gpg perl make xz cmake",
                      result.stderr)
        for name in ("python3", "gpg", "perl", "make", "xz", "cmake"):
            self.command(name, "exit 0\n")
        self.env["LIBCLANG_PATH"] = str(self.root)
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("missing native module build prerequisites", result.stderr)


if __name__ == "__main__":
    unittest.main()

