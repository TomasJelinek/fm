"""Build entry points order app, plugins and private deployment, retaining failures."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class BuildHookTests(unittest.TestCase):
    # build.bat refuses to start without PowerShell 7; these cases need no pwsh.
    WITHOUT_POWERSHELL = {"test_missing_powershell_seven_fails_before_cargo_or_plugin_builds",
                          "test_invalid_profile_does_not_start_any_build",
                          "test_invalid_validate_combinations_do_not_start_any_build"}

    def setUp(self):
        if (os.name == "nt" and shutil.which("pwsh") is None
                and self._testMethodName not in self.WITHOUT_POWERSHELL):
            self.skipTest("build.bat requires PowerShell 7 (pwsh.exe)")
        repository = Path(__file__).resolve().parents[2]
        temporary = repository / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        self.log = self.root / "calls.log"
        fake_bin = self.root / "bin"
        fake_bin.mkdir()
        self.environment = dict(os.environ, PATH=str(fake_bin) + os.pathsep + os.environ["PATH"],
                                BUILD_TEST_LOG=str(self.log), BUILD_TEST_APP_EXIT="0",
                                BUILD_TEST_NATIVE_EXIT="0", BUILD_TEST_NETWORK_EXIT="0", BUILD_TEST_VIEWER_EXIT="0",
                                BUILD_TEST_DEPLOY_EXIT="0", BUILD_TEST_THUMBNAIL_EXIT="0",
                                BUILD_TEST_ARCHIVE_EXIT="0", BUILD_TEST_VALIDATE_EXIT="0")
        if os.name == "nt":
            entry = "build.bat"
            (fake_bin / "cargo.cmd").write_text(
                '@echo off\n>>"%BUILD_TEST_LOG%" echo cargo %*\nexit /b %BUILD_TEST_APP_EXIT%\n')
            (self.root / "scripts/build-viewers.ps1").write_text(
                'param([string]$Profile)\nAdd-Content -LiteralPath $env:BUILD_TEST_LOG "viewer $Profile"\n'
                'exit ([int]$env:BUILD_TEST_VIEWER_EXIT)\n')
            (self.root / "scripts/build-native-filesystem.ps1").write_text(
                'param([string]$Profile)\nAdd-Content -LiteralPath $env:BUILD_TEST_LOG "native $Profile"\n'
                'exit ([int]$env:BUILD_TEST_NATIVE_EXIT)\n')
            (self.root / "scripts/build-network-filesystem.ps1").write_text(
                'param([string]$Profile)\nAdd-Content -LiteralPath $env:BUILD_TEST_LOG "network $Profile"\n'
                'exit ([int]$env:BUILD_TEST_NETWORK_EXIT)\n')
            (self.root / "scripts/build-default-thumbnailers.ps1").write_text(
                'param([string]$Profile)\nAdd-Content -LiteralPath $env:BUILD_TEST_LOG "thumbnail $Profile"\n'
                'exit ([int]$env:BUILD_TEST_THUMBNAIL_EXIT)\n')
            (self.root / "scripts/build-archive-filesystem.ps1").write_text(
                'param([string]$Profile)\nAdd-Content -LiteralPath $env:BUILD_TEST_LOG "archive $Profile"\n'
                'exit ([int]$env:BUILD_TEST_ARCHIVE_EXIT)\n')
            (self.root / "scripts/deploy-build.ps1").write_text(
                'param([string]$Profile)\nAdd-Content -LiteralPath $env:BUILD_TEST_LOG "deploy $Profile"\n'
                'exit ([int]$env:BUILD_TEST_DEPLOY_EXIT)\n')
            (self.root / "scripts/validate.ps1").write_text(
                'Add-Content -LiteralPath $env:BUILD_TEST_LOG ("validate " + ($args -join " "))\n'
                'exit ([int]$env:BUILD_TEST_VALIDATE_EXIT)\n')
            self.command = ["cmd.exe", "/d", "/c", str(self.root / entry)]
        else:
            entry = "build.sh"
            self.environment["FM_BUILD_BACKEND"] = "native"
            cargo = fake_bin / "cargo"
            cargo.write_text('#!/bin/sh\nprintf "cargo %s\\n" "$*" >> "$BUILD_TEST_LOG"\n'
                             'exit "$BUILD_TEST_APP_EXIT"\n')
            cargo.chmod(0o755)
            (self.root / "scripts/build-viewers.sh").write_text(
                '#!/bin/sh\nprintf "viewer %s\\n" "$1" >> "$BUILD_TEST_LOG"\n'
                'exit "$BUILD_TEST_VIEWER_EXIT"\n')
            (self.root / "scripts/build-native-filesystem.sh").write_text(
                '#!/bin/sh\nprintf "native %s\\n" "$1" >> "$BUILD_TEST_LOG"\n'
                'exit "$BUILD_TEST_NATIVE_EXIT"\n')
            (self.root / "scripts/build-network-filesystem.sh").write_text(
                '#!/bin/sh\nprintf "network %s\\n" "$1" >> "$BUILD_TEST_LOG"\n'
                'exit "$BUILD_TEST_NETWORK_EXIT"\n')
            (self.root / "scripts/build-default-thumbnailers.sh").write_text(
                '#!/bin/sh\nprintf "thumbnail %s\\n" "$1" >> "$BUILD_TEST_LOG"\n'
                'exit "$BUILD_TEST_THUMBNAIL_EXIT"\n')
            (self.root / "scripts/build-archive-filesystem.sh").write_text(
                '#!/bin/sh\nprintf "archive %s\\n" "$1" >> "$BUILD_TEST_LOG"\n'
                'exit "$BUILD_TEST_ARCHIVE_EXIT"\n')
            (self.root / "scripts/validate.sh").write_text(
                '#!/bin/sh\nprintf "validate %s\\n" "$*" >> "$BUILD_TEST_LOG"\n'
                'exit "$BUILD_TEST_VALIDATE_EXIT"\n')
            self.command = ["sh", str(self.root / entry)]
        shutil.copyfile(repository / entry, self.root / entry)

    def run_build(self, profile="release", app_exit="0", native_exit="0", network_exit="0", viewer_exit="0", deploy_exit="0", thumbnail_exit="0", archive_exit="0"):
        environment = dict(self.environment, BUILD_TEST_APP_EXIT=app_exit,
                           BUILD_TEST_NATIVE_EXIT=native_exit, BUILD_TEST_NETWORK_EXIT=network_exit,
                           BUILD_TEST_VIEWER_EXIT=viewer_exit,
                           BUILD_TEST_DEPLOY_EXIT=deploy_exit, BUILD_TEST_THUMBNAIL_EXIT=thumbnail_exit,
                           BUILD_TEST_ARCHIVE_EXIT=archive_exit)
        result = subprocess.run(self.command + [profile], cwd=self.root, env=environment,
                                capture_output=True, text=True, timeout=30)
        calls = self.log.read_text().splitlines() if self.log.exists() else []
        self.build_output = result.stdout + result.stderr
        return result.returncode, calls

    def test_success_builds_application_then_viewers_for_same_profile(self):
        code, calls = self.run_build()
        self.assertEqual(code, 0, self.build_output)
        expected = ["cargo build -p midnight-explorer -p midnight-elevation --locked --release", "native release",
                    "network release", "viewer release", "thumbnail release", "archive release"]
        if os.name == "nt":
            expected.append("deploy release")
        self.assertEqual(calls, expected)

    def test_thumbnail_failure_prevents_deployment_and_is_returned(self):
        code, calls = self.run_build(thumbnail_exit="46")
        self.assertEqual(code, 46, self.build_output)
        self.assertEqual(calls[-1], "thumbnail release")
        self.assertNotIn("deploy release", calls)

    def test_default_build_includes_debug_thumbnailers(self):
        result = subprocess.run(self.command, cwd=self.root, env=self.environment,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("thumbnail debug", self.log.read_text().splitlines())
        self.assertIn("archive debug", self.log.read_text().splitlines())

    def test_archive_failure_prevents_deployment_and_is_returned(self):
        code, calls = self.run_build(archive_exit="48")
        self.assertEqual(code, 48, self.build_output)
        self.assertEqual(calls[-2:], ["thumbnail release", "archive release"])
        self.assertNotIn("deploy release", calls)

    @unittest.skipUnless(os.name == "nt", "Windows PowerShell prerequisite")
    def test_missing_powershell_seven_fails_before_cargo_or_plugin_builds(self):
        environment = dict(self.environment)
        # Keep cmd.exe/where.exe available while excluding the PowerShell 7 install.
        environment["PATH"] = str(self.root / "bin") + os.pathsep + str(
            Path(os.environ["SystemRoot"]) / "System32")
        result = subprocess.run(self.command, cwd=self.root, env=environment,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 127, result.stdout + result.stderr)
        self.assertIn("64-bit PowerShell 7", result.stderr)
        self.assertFalse(self.log.exists(), "Missing prerequisite started a build")

    @unittest.skipUnless(os.name == "nt", "Windows Python prerequisite")
    def test_missing_python_fails_before_cargo_or_plugin_builds(self):
        # A python.exe Store alias or a missing launcher used to fail only after the
        # complete host build, in the thumbnail/HTML/deployment steps.
        environment = dict(self.environment)
        environment["PATH"] = os.pathsep.join([
            str(self.root / "bin"), str(Path(os.environ["SystemRoot"]) / "System32"),
            os.path.dirname(shutil.which("pwsh"))])
        result = subprocess.run(self.command, cwd=self.root, env=environment,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 127, result.stdout + result.stderr)
        self.assertIn("Python 3.12", result.stderr)
        self.assertFalse(self.log.exists(), "Missing prerequisite started a build")

    def test_application_failure_skips_viewer_build(self):
        code, calls = self.run_build(app_exit="35")
        self.assertEqual(code, 35)
        self.assertEqual(calls, ["cargo build -p midnight-explorer -p midnight-elevation --locked --release"])

    def test_viewer_failure_is_returned_to_caller(self):
        code, calls = self.run_build(viewer_exit="43")
        self.assertEqual(code, 43)
        self.assertEqual(calls[-1], "viewer release")

    def test_native_failure_skips_viewers_and_private_deployment(self):
        code, calls = self.run_build(native_exit="41")
        self.assertEqual(code, 41, self.build_output)
        self.assertEqual(calls, ["cargo build -p midnight-explorer -p midnight-elevation --locked --release",
                                 "native release"])

    def test_invalid_profile_does_not_start_any_build(self):
        code, calls = self.run_build(profile="unknown")
        self.assertEqual(code, 2)
        self.assertEqual(calls, [])

    def test_network_failure_skips_viewers_and_private_deployment(self):
        code, calls = self.run_build(network_exit="42")
        self.assertEqual(code, 42, self.build_output)
        self.assertEqual(calls, ["cargo build -p midnight-explorer -p midnight-elevation --locked --release",
                                 "native release", "network release"])

    def run_arguments(self, *arguments, validate_exit="0"):
        environment = dict(self.environment, BUILD_TEST_VALIDATE_EXIT=validate_exit)
        result = subprocess.run(self.command + list(arguments), cwd=self.root, env=environment,
                                capture_output=True, text=True, timeout=30)
        self.build_output = result.stdout + result.stderr
        return result.returncode, self.log.read_text().splitlines() if self.log.exists() else []

    def test_validation_runs_after_every_build_step_and_before_deployment(self):
        for arguments, profile in ((("release", "--validate"), "release"), (("--validate", "release"), "release"),
                                   (("--validate",), "debug")):
            with self.subTest(arguments=arguments):
                if self.log.exists():
                    self.log.unlink()
                code, calls = self.run_arguments(*arguments)
                self.assertEqual(code, 0, self.build_output)
                build = "cargo build -p midnight-explorer -p midnight-elevation --locked"
                expected = [build + (" --release" if profile == "release" else ""), f"native {profile}",
                            f"network {profile}", f"viewer {profile}", f"thumbnail {profile}",
                            f"archive {profile}", f"validate {profile}"]
                if os.name == "nt":
                    expected.append(f"deploy {profile}")
                self.assertEqual(calls, expected)

    def test_validation_failure_prevents_deployment_and_is_returned(self):
        code, calls = self.run_arguments("--validate", validate_exit="1")
        self.assertEqual(code, 1, self.build_output)
        self.assertEqual(calls[-2:], ["archive debug", "validate debug"])
        code, calls = self.run_arguments("release")
        self.assertNotIn("validate release", calls, "validation runs only when requested")

    def test_build_failure_skips_validation(self):
        self.environment["BUILD_TEST_ARCHIVE_EXIT"] = "48"
        code, calls = self.run_arguments("release", "--validate")
        self.assertEqual(code, 48, self.build_output)
        self.assertEqual(calls[-1], "archive release")
        self.assertFalse([call for call in calls if call.startswith(("validate", "deploy"))])

    def test_invalid_validate_combinations_do_not_start_any_build(self):
        for arguments in (("clean", "--validate"), ("--validate", "--validate"), ("--validate", "debug", "release"),
                          ("--validated",)):
            with self.subTest(arguments=arguments):
                code, calls = self.run_arguments(*arguments)
                self.assertEqual(code, 2, self.build_output)
                self.assertEqual(calls, [])
                self.assertIn("--validate", self.build_output)

    @unittest.skipUnless(os.name == "nt", "Windows private deployment hook")
    def test_deployment_failure_is_returned_after_app_and_viewers(self):
        code, calls = self.run_build(deploy_exit="47")
        self.assertEqual(code, 47, self.build_output)
        self.assertEqual(calls, ["cargo build -p midnight-explorer -p midnight-elevation --locked --release",
                                 "native release", "network release", "viewer release", "thumbnail release", "archive release", "deploy release"])


if __name__ == "__main__":
    unittest.main()
