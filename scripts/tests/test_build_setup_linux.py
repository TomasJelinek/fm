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
        for name in ("native-filesystem", "network-filesystem", "viewers", "default-thumbnailers", "archive-filesystem"):
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
                self.assertEqual(commands[-5:], [
                    f"Command: [bash] [scripts/build-native-filesystem.sh] [{profile}]",
                    f"Command: [bash] [scripts/build-network-filesystem.sh] [{profile}]",
                    f"Command: [bash] [scripts/build-viewers.sh] [{profile}]",
                    f"Command: [bash] [scripts/build-default-thumbnailers.sh] [{profile}]",
                    f"Command: [bash] [scripts/build-archive-filesystem.sh] [{profile}]",
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

    def test_validate_dry_run_prints_the_validation_plan_after_the_build_plan(self):
        shutil.copyfile(ROOT / "scripts/validate.sh", self.root / "scripts/validate.sh")
        result = self.run_script("build.sh", "--validate", "release", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = [line for line in result.stdout.splitlines() if line.startswith("Command:")]
        self.assertEqual(commands[-2:], ["Command: [bash] [scripts/build-archive-filesystem.sh] [release]",
                                         "Command: [bash] [scripts/validate.sh] [release]"])
        plan = result.stdout.split("Validation plan (dry run)", 1)[1]
        self.assertIn("workspace-tests - Test (workspace gates, doc tests included)", plan)
        self.assertIn("python3 -B scripts/ci-workspace-tests.py --keep-test-artifacts --no-fail-fast", plan)
        self.assertIn("host-build - Host build (release)", plan)
        self.assertEqual(self.calls(), "")
        self.assertFalse((self.root / "_build").exists())

    def test_validate_refuses_the_docker_backend_before_any_action(self):
        for backend, hint in (("docker", "FM_BUILD_BACKEND"), ("auto", "--install-deps")):
            for arguments in (("--validate",), ("release", "--validate", "--dry-run")):
                with self.subTest(backend=backend, arguments=arguments):
                    self.environment["FM_BUILD_BACKEND"] = backend
                    self.command("pkg-config", "exit 1\n")  # auto would select Docker
                    result = self.run_script("build.sh", *arguments)
                    self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                    self.assertIn("--validate runs the tests natively", result.stderr)
                    self.assertIn(hint, result.stderr)
                    self.assertEqual(self.calls(), "")
                    self.assertFalse((self.root / "_build").exists())

    def acceptance_stub(self):
        (self.root / "scripts/acceptance.sh").write_text(
            '#!/bin/sh\nprintf "acceptance [%s]\\n" "$*" >> "$BUILD_SETUP_LOG"\nexit "${ACCEPTANCE_EXIT:-0}"\n',
            encoding="utf-8")

    def test_acceptance_dry_run_prints_the_suite_command_after_the_build_plan(self):
        shutil.copyfile(ROOT / "scripts/acceptance.sh", self.root / "scripts/acceptance.sh")
        result = self.run_script("build.sh", "--acceptance", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = [line for line in result.stdout.splitlines() if line.startswith("Command:")]
        self.assertEqual(commands[-3:], [
            "Command: [bash] [scripts/build-archive-filesystem.sh] [debug]",
            "Command: [bash] [scripts/acceptance.sh] [debug]",
            "Command: [cargo] [test] [--workspace] [--locked] [--features] "
            "[midnight-acceptance/acceptance] [--test] [acceptance] [--]"])
        self.assertEqual(self.calls(), "")
        self.assertFalse((self.root / "_build").exists())

    def test_acceptance_runs_after_every_build_step(self):
        self.acceptance_stub()
        result = self.run_script("build.sh", "release", "--acceptance")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls().splitlines()
        self.assertEqual(calls[-2:], ["archive-filesystem [release]", "acceptance [release]"])
        self.environment["ACCEPTANCE_EXIT"] = "1"
        self.assertEqual(self.run_script("build.sh", "--acceptance").returncode, 1)

    def test_validate_and_acceptance_both_run_and_the_first_failure_decides(self):
        self.acceptance_stub()
        (self.root / "scripts/validate.sh").write_text(
            '#!/bin/sh\nprintf "validate [%s]\\n" "$*" >> "$BUILD_SETUP_LOG"\nexit "${VALIDATE_EXIT:-0}"\n',
            encoding="utf-8")
        self.environment["VALIDATE_EXIT"] = "1"
        result = self.run_script("build.sh", "--validate", "--acceptance")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(self.calls().splitlines()[-2:], ["validate [debug]", "acceptance [debug]"])

    def test_acceptance_uses_the_docker_backend(self):
        shutil.copyfile(ROOT / "scripts/build-docker.sh", self.root / "scripts/build-docker.sh")
        for name in ("docker-install-deps.sh", "install-rust-toolchain.sh"):
            shutil.copyfile(ROOT / "scripts" / name, self.root / "scripts" / name)
        shutil.copyfile(ROOT / "rust-toolchain.toml", self.root / "rust-toolchain.toml")
        self.environment["FM_BUILD_BACKEND"] = "docker"
        result = self.run_script("build.sh", "--acceptance", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        run = [line for line in result.stdout.splitlines() if "[run]" in line]
        self.assertEqual(len(run), 1, result.stdout)
        self.assertTrue(run[0].endswith("[sh] [/workspace/build.sh] [debug] [--acceptance]"), run[0])
        self.assertEqual(self.calls(), "")

    def test_acceptance_script_rejects_invalid_arguments(self):
        shutil.copyfile(ROOT / "scripts/acceptance.sh", self.root / "scripts/acceptance.sh")
        for arguments in (("release", "debug"), ("--only",), ("--only", ""), ("--only", "a,-x"),
                          ("--scale", "huge"), ("--unknown",), ("--run-dir",),
                          ("--run-dir", "relative"), ("--run-dir", "/")):
            with self.subTest(arguments=arguments):
                result = subprocess.run(["bash", str(self.root / "scripts/acceptance.sh"), *arguments],
                                        cwd=self.root.parent, env=self.environment, text=True,
                                        capture_output=True, timeout=20)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertEqual(self.calls(), "")
        # A gated volume's folder must be absolute.
        for variable in ("MIDNIGHT_ACCEPTANCE_FAT_DIR", "MIDNIGHT_ACCEPTANCE_NO_TMPFILE_DIR"):
            with self.subTest(variable=variable):
                result = subprocess.run(["bash", str(self.root / "scripts/acceptance.sh"), "--dry-run"],
                                        cwd=self.root.parent, env=dict(self.environment, **{variable: "relative"}),
                                        text=True, capture_output=True, timeout=20)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn(f"{variable} relative is not an absolute folder", result.stderr)
        runs = self.root / "runs & $(literal)"
        fat = self.root / "fat volume"
        result = subprocess.run(["bash", str(self.root / "scripts/acceptance.sh"), "--dry-run",
                                 "--only", "startup,ui_review", "--scale", "smoke", "--strict-review",
                                 "--run-dir", str(runs), "--stress"],
                                cwd=self.root.parent,
                                env=dict(self.environment, MIDNIGHT_ACCEPTANCE_FAT_DIR=str(fat)),
                                text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.splitlines()
        self.assertIn("[--] [startup] [ui_review] [--scale] [smoke] [--strict-review] [--stress]",
                      result.stdout)
        self.assertIn(f"Run folders: {runs}", lines)
        self.assertIn("Stress sizes: on (MIDNIGHT_ACCEPTANCE_STRESS=1)", lines)
        self.assertIn(f"Gated folder: MIDNIGHT_ACCEPTANCE_FAT_DIR = {fat}", lines)
        self.assertEqual(self.calls(), "")

    def test_acceptance_script_exports_the_run_folder_stress_and_gated_folders(self):
        shutil.copyfile(ROOT / "scripts/acceptance.sh", self.root / "scripts/acceptance.sh")
        # The stub cargo records the variables the suite (and its staged children) inherit.
        self.command("cargo", 'printf "cargo [%s] [%s] [%s]\\n" "${MIDNIGHT_ACCEPTANCE_RUN_DIR-unset}" '
                     '"${MIDNIGHT_ACCEPTANCE_STRESS-unset}" "${MIDNIGHT_ACCEPTANCE_FAT_DIR-unset}" '
                     '>> "$BUILD_SETUP_LOG"\n')
        runs = self.root / "runs"
        fat = self.root / "fat volume"
        environment = dict(self.environment, MIDNIGHT_ACCEPTANCE_FAT_DIR=str(fat))
        for name in ("MIDNIGHT_ACCEPTANCE_RUN_DIR", "MIDNIGHT_ACCEPTANCE_STRESS"):
            environment.pop(name, None)
        result = subprocess.run(["bash", str(self.root / "scripts/acceptance.sh"), "--run-dir", str(runs),
                                 "--stress", "--only", "startup"],
                                cwd=self.root.parent, env=environment, text=True,
                                capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.calls().splitlines(), [f"cargo [{runs}] [1] [{fat}]"])
        self.assertIn(f"Gated folder: MIDNIGHT_ACCEPTANCE_FAT_DIR = {fat}", result.stdout.splitlines())

    def test_acceptance_script_stages_packages_from_elsewhere_in_its_own_build(self):
        # A staged scenario makes its stage beside the packages' folder unless
        # MIDNIGHT_ACCEPTANCE_STAGE_ROOT names another: packages outside the checkout
        # (another checkout's or a shared build) must not get stages written beside them.
        shutil.copyfile(ROOT / "scripts/acceptance.sh", self.root / "scripts/acceptance.sh")
        self.command("cargo", 'printf "cargo [%s] [%s]\\n" "${MIDNIGHT_ACCEPTANCE_PLUGINS-unset}" '
                     '"${MIDNIGHT_ACCEPTANCE_STAGE_ROOT-unset}" >> "$BUILD_SETUP_LOG"\n')
        checkout = self.root.resolve()
        shared = self.root.parent / "shared build" / "debug"
        own = checkout / "_build" / "release"
        stages = self.root.parent / "my stages"
        environment = {name: value for name, value in self.environment.items()
                       if name not in ("MIDNIGHT_ACCEPTANCE_PLUGINS", "MIDNIGHT_ACCEPTANCE_STAGE_ROOT")}
        for variables, plugins, stage_root in (
                ({"MIDNIGHT_ACCEPTANCE_PLUGINS": str(shared)}, shared, checkout / "_build"),
                ({"MIDNIGHT_ACCEPTANCE_PLUGINS": str(own)}, own, None),
                ({"MIDNIGHT_ACCEPTANCE_PLUGINS": str(shared),
                  "MIDNIGHT_ACCEPTANCE_STAGE_ROOT": str(stages)}, shared, stages)):
            with self.subTest(variables=variables):
                self.log.unlink(missing_ok=True)
                for arguments in (("--dry-run",), ("--only", "startup")):
                    result = subprocess.run(["bash", str(self.root / "scripts/acceptance.sh"), *arguments],
                                            cwd=self.root.parent, env=dict(environment, **variables),
                                            text=True, capture_output=True, timeout=20)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    if arguments == ("--dry-run",):
                        stage_lines = [line for line in result.stdout.splitlines()
                                       if line.startswith("Stages: ")]
                        self.assertEqual(stage_lines, [] if stage_root is None else [f"Stages: {stage_root}"])
                self.assertEqual(self.calls().splitlines(),
                                 [f"cargo [{plugins}] [{stage_root or 'unset'}]"])
                if stage_root is not None:
                    self.assertTrue(Path(stage_root).is_dir())

    def test_invalid_or_conflicting_arguments_stop_before_any_tool(self):
        for arguments in (("release", "debug"), ("clean", "--install-deps"),
                          ("clean", "--validate"), ("--validate", "--validate"),
                          ("clean", "--acceptance"), ("--acceptance", "--acceptance"),
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

    def install_commands(self, identifier):
        result = self.run_script("scripts/setup-linux.sh", "--install", "--dry-run",
                                 "--os-release", self.release_file(identifier))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), "")
        return [line for line in result.stdout.splitlines()
                if line.startswith("Command:") and "[install]" in line]

    def test_install_plans_add_the_runtime_libraries_after_the_build_packages(self):
        # The application's desktop libraries and every library of the HTML viewer's
        # Chromium: Ubuntu 24.04 package names, and sonames as RPM package capabilities.
        runtime = {
            "ubuntu": ("libnss3", "libnspr4", "libasound2t64", "libatk1.0-0t64",
                       "libatk-bridge2.0-0t64", "libatspi2.0-0t64", "libcups2t64", "libgbm1",
                       "libdrm2", "libpango-1.0-0", "libcairo2", "libxcomposite1", "libxdamage1",
                       "libxfixes3", "libxrandr2", "libxcursor1", "libxi6", "libvulkan1", "xdg-utils"),
            "fedora": ("libnss3.so()(64bit)", "libnspr4.so()(64bit)", "libasound.so.2()(64bit)",
                       "libatk-1.0.so.0()(64bit)", "libatk-bridge-2.0.so.0()(64bit)",
                       "libcups.so.2()(64bit)", "libgbm.so.1()(64bit)", "libXi.so.6()(64bit)",
                       "libvulkan.so.1()(64bit)"),
        }
        runtime["linuxmint"] = runtime["ubuntu"]
        runtime["opensuse-leap"] = runtime["opensuse-tumbleweed"] = runtime["fedora"]
        for identifier, libraries in runtime.items():
            with self.subTest(identifier=identifier):
                build, libraries_command = self.install_commands(identifier)
                self.assertIn("[python3]", build)
                for library in libraries:
                    self.assertIn(f"[{library}]", libraries_command)
                    self.assertNotIn(f"[{library}]", build)
                # Each capability stays one literal argument.
                self.assertNotIn("[(64bit)]", libraries_command)
                self.assertNotIn("'", libraries_command)

    def test_opted_in_setup_installs_runtime_libraries_only_after_the_build_packages(self):
        result = self.run_script("scripts/setup-linux.sh", "--install", "--os-release",
                                 self.release_file("fedora"))
        self.assertEqual(result.returncode, 0, result.stderr)
        installs = [line for line in self.calls().splitlines() if line.startswith("dnf [install]")]
        self.assertEqual(len(installs), 2, self.calls())
        self.assertIn("[gcc]", installs[0])
        self.assertIn("[libnss3.so()(64bit)]", installs[1])
        self.environment["BUILD_SETUP_EXIT"] = "44"
        self.log.unlink()
        result = self.run_script("scripts/setup-linux.sh", "--install", "--os-release",
                                 self.release_file("opensuse-leap"))
        self.assertEqual(result.returncode, 44, result.stderr)
        self.assertEqual(self.calls().count("zypper [install]"), 1, self.calls())
        self.assertNotIn("[libnss3.so()(64bit)]", self.calls())

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
                                            "network-filesystem [release]\nviewers [release]\ndefault-thumbnailers [release]\n"
                                            "archive-filesystem [release]\n"))
        self.assertNotIn("sudo", self.calls())


if __name__ == "__main__":
    unittest.main()
