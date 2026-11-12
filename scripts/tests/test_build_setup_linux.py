"""Linux build setup runs only explicitly requested, literal commands."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux build setup")
class LinuxBuildSetupTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="build setup ", dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name) / "checkout with spaces & dollar$"
        (self.root / "scripts").mkdir(parents=True)
        shutil.copyfile(ROOT / "build.sh", self.root / "build.sh")
        setup = ROOT / "scripts/setup-linux.sh"
        if setup.exists():
            shutil.copyfile(setup, self.root / "scripts/setup-linux.sh")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "commands.log"
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + "/usr/bin:/bin",
                                HOME=str(self.root / "home"),
                                FM_BUILD_BACKEND="native",
                                BUILD_SETUP_LOG=str(self.log), BUILD_SETUP_EXIT="0")
        for name in ("CARGO_HOME", "RUSTUP_HOME", "CARGO_TARGET_DIR"):
            self.environment.pop(name, None)
        self.command("cargo", 'printf "cargo" >> "$BUILD_SETUP_LOG"\n'
                     'printf " [%s]" "$@" >> "$BUILD_SETUP_LOG"\n'
                     'printf "\\n" >> "$BUILD_SETUP_LOG"\n')
        self.command("id", 'printf "1000\\n"\n')
        self.command("sudo", 'printf "sudo\\n" >> "$BUILD_SETUP_LOG"\n'
                     'if [ "${1-}" = -- ]; then shift; fi\nexec "$@"\n')
        for name in ("apt-get", "dnf", "zypper"):
            self.command(name, f'printf "{name}" >> "$BUILD_SETUP_LOG"\n'
                         'printf " [%s]" "$@" >> "$BUILD_SETUP_LOG"\n'
                         'printf "\\n" >> "$BUILD_SETUP_LOG"\n'
                         'exit "$BUILD_SETUP_EXIT"\n')
        for name in ("curl", "rustup", "docker"):
            self.command(name, f'printf "UNEXPECTED {name}\\n" >> "$BUILD_SETUP_LOG"\nexit 99\n')
        for name in ("native-filesystem", "network-filesystem", "viewers"):
            (self.root / f"scripts/build-{name}.sh").write_text(
                f'#!/bin/sh\nprintf "{name} [%s]\\n" "$1" >> "$BUILD_SETUP_LOG"\n',
                encoding="utf-8")

    def command(self, name, body):
        path = self.bin / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/bin/sh\nset -eu\n" + body, encoding="utf-8")
        path.chmod(0o755)
        return path

    def run_script(self, script, *arguments):
        return subprocess.run(["sh", str(self.root / script), *arguments], cwd=self.root.parent,
                              env=self.environment, text=True, capture_output=True, timeout=20)

    def calls(self):
        return self.log.read_text(encoding="utf-8") if self.log.exists() else ""

    def release_file(self, identifier, like=""):
        path = self.root / "os release"
        path.write_text(f'ID="{identifier}"\nID_LIKE="{like}"\n', encoding="utf-8")
        return str(path)

    def test_build_dry_run_keeps_full_plugin_order_without_running_tools(self):
        for profile, release in (("debug", ""), ("release", " [--release]")):
            with self.subTest(profile=profile):
                result = self.run_script("build.sh", "--dry-run", profile)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("[cargo] [build] [-p] [midnight-explorer] [-p] "
                              "[midnight-elevation] [--locked]" + release, result.stdout)
                commands = [line for line in result.stdout.splitlines() if line.startswith("Command:")]
                self.assertEqual(commands[-3:], [
                    f"Command: [bash] [scripts/build-native-filesystem.sh] [{profile}]",
                    f"Command: [bash] [scripts/build-network-filesystem.sh] [{profile}]",
                    f"Command: [bash] [scripts/build-viewers.sh] [{profile}]",
                ])
                self.assertEqual(self.calls(), "")
                self.assertFalse((self.root / "_build").exists())

    def test_dry_run_can_plan_setup_without_installing_or_building(self):
        result = self.run_script("build.sh", "release", "--install-deps", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[scripts/setup-linux.sh] [--install] [--dry-run]", result.stdout)
        self.assertEqual(self.calls(), "")
        self.assertFalse((self.root / "_build").exists())

    def test_native_install_option_overrides_auto_docker_fallback(self):
        self.environment["FM_BUILD_BACKEND"] = "auto"
        self.command("pkg-config", "exit 1\n")
        (self.root / "scripts/setup-linux.sh").write_text(
            '#!/bin/sh\nprintf "native setup\\n" >> "$BUILD_SETUP_LOG"\n', encoding="utf-8")
        result = self.run_script("build.sh", "--install-deps")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.calls().startswith("native setup\n"))
        self.assertIn("cargo [build]", self.calls())
        self.assertNotIn("UNEXPECTED", self.calls())

    def test_explicit_docker_rejects_host_install_option_before_actions(self):
        self.environment["FM_BUILD_BACKEND"] = "docker"
        result = self.run_script("build.sh", "--install-deps", "--dry-run")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertEqual(self.calls(), "")
        self.assertFalse((self.root / "_build").exists())

    def test_invalid_backend_is_rejected_even_during_dry_run(self):
        self.environment["FM_BUILD_BACKEND"] = "unsupported"
        result = self.run_script("build.sh", "--dry-run")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertEqual(self.calls(), "")
        self.assertFalse((self.root / "_build").exists())

    def test_invalid_or_conflicting_arguments_stop_before_any_tool(self):
        for arguments in (("release", "debug"), ("clean", "--install-deps"),
                          ("--unknown",), ("release", "extra")):
            with self.subTest(arguments=arguments):
                result = self.run_script("build.sh", *arguments)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(self.calls(), "")
                self.assertFalse((self.root / "_build").exists())

    def test_native_package_plans_cover_supported_distributions_without_execution(self):
        plans = {
            "ubuntu": ("apt-get", "build-essential", "libfontconfig1-dev"),
            "linuxmint": ("apt-get", "build-essential", "libxkbcommon-x11-0"),
            "fedora": ("dnf", "pkgconf-pkg-config", "libXcursor-devel"),
            "opensuse-leap": ("zypper", "pkgconfig(fontconfig)", "pkgconfig(libudev)"),
            "opensuse-tumbleweed": ("zypper", "pkgconfig(xkbcommon-x11)", "xdg-utils"),
        }
        for identifier, expected in plans.items():
            with self.subTest(identifier=identifier):
                result = self.run_script("scripts/setup-linux.sh", "--install", "--dry-run",
                                         "--os-release", self.release_file(identifier))
                self.assertEqual(result.returncode, 0, result.stderr)
                for argument in expected:
                    self.assertIn(f"[{argument}]", result.stdout)
                self.assertIn("[python3]", result.stdout)
                self.assertEqual(self.calls(), "")

    def test_os_release_is_data_and_supported_id_like_can_select_the_plan(self):
        marker = self.root / "unexpected-execution"
        result = self.run_script("scripts/setup-linux.sh", "--dry-run", "--os-release",
                                 self.release_file(f"$(touch {marker})", "ubuntu debian"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[apt-get]", result.stdout)
        self.assertFalse(marker.exists())
        self.assertEqual(self.calls(), "")

    def test_unknown_distribution_stops_without_installing(self):
        result = self.run_script("scripts/setup-linux.sh", "--install", "--os-release",
                                 self.release_file("unsupported"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unsupported", result.stderr)
        self.assertEqual(self.calls(), "")

    def test_setup_without_opt_in_only_prints_commands(self):
        result = self.run_script("scripts/setup-linux.sh", "--os-release", self.release_file("fedora"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[dnf] [install]", result.stdout)
        self.assertIn("--install", result.stdout)
        self.assertEqual(self.calls(), "")

    def test_opted_in_setup_preserves_literal_package_arguments(self):
        result = self.run_script("scripts/setup-linux.sh", "--install", "--os-release",
                                 self.release_file("opensuse-tumbleweed"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("sudo\nzypper [install]", self.calls())
        self.assertIn("[pkgconfig(fontconfig)]", self.calls())
        self.assertNotIn("UNEXPECTED", self.calls())

    def test_setup_failure_stops_before_any_cargo_or_plugin_build(self):
        (self.root / "scripts/setup-linux.sh").write_text("#!/bin/sh\nexit 47\n", encoding="utf-8")
        result = self.run_script("build.sh", "--install-deps")
        self.assertEqual(result.returncode, 47, result.stderr)
        self.assertEqual(self.calls(), "")

    def test_package_failure_is_returned_without_further_setup(self):
        self.environment["BUILD_SETUP_EXIT"] = "43"
        result = self.run_script("scripts/setup-linux.sh", "--install", "--os-release",
                                 self.release_file("ubuntu"))
        self.assertEqual(result.returncode, 43, result.stderr)
        self.assertIn("apt-get [update]", self.calls())
        self.assertNotIn("[install]", self.calls())

    def test_cargo_in_configured_home_is_used_when_missing_from_path(self):
        self.assert_cargo_home("custom cargo", "CARGO_HOME")

    def test_cargo_in_default_home_is_used_when_missing_from_path(self):
        self.assert_cargo_home("home/.cargo", None)

    def assert_cargo_home(self, relative, variable):
        cargo = self.bin / "cargo"
        destination = self.root / relative / "bin/cargo"
        destination.parent.mkdir(parents=True)
        cargo.rename(destination)
        if variable:
            self.environment[variable] = str(destination.parents[1])
        result = self.run_script("build.sh", "release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cargo [build] [-p] [midnight-explorer] [-p] [midnight-elevation] "
                      "[--locked] [--release]", self.calls())
        self.assertTrue(self.calls().endswith("native-filesystem [release]\n"
                                            "network-filesystem [release]\nviewers [release]\n"))
        self.assertNotIn("sudo", self.calls())


if __name__ == "__main__":
    unittest.main()
