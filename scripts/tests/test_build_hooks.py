"""Build entry points order app, plugins and private deployment, retaining failures."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class BuildHookTests(unittest.TestCase):
    def setUp(self):
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
                                BUILD_TEST_DEPLOY_EXIT="0")
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
            (self.root / "scripts/deploy-build.ps1").write_text(
                'param([string]$Profile)\nAdd-Content -LiteralPath $env:BUILD_TEST_LOG "deploy $Profile"\n'
                'exit ([int]$env:BUILD_TEST_DEPLOY_EXIT)\n')
            self.command = ["cmd.exe", "/d", "/c", str(self.root / entry)]
        else:
            entry = "build.sh"
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
            self.command = ["sh", str(self.root / entry)]
        shutil.copyfile(repository / entry, self.root / entry)

    def run_build(self, profile="release", app_exit="0", native_exit="0", network_exit="0", viewer_exit="0", deploy_exit="0"):
        environment = dict(self.environment, BUILD_TEST_APP_EXIT=app_exit,
                           BUILD_TEST_NATIVE_EXIT=native_exit, BUILD_TEST_NETWORK_EXIT=network_exit,
                           BUILD_TEST_VIEWER_EXIT=viewer_exit,
                           BUILD_TEST_DEPLOY_EXIT=deploy_exit)
        result = subprocess.run(self.command + [profile], cwd=self.root, env=environment,
                                capture_output=True, text=True, timeout=30)
        calls = self.log.read_text().splitlines() if self.log.exists() else []
        self.build_output = result.stdout + result.stderr
        return result.returncode, calls

    def test_success_builds_application_then_viewers_for_same_profile(self):
        code, calls = self.run_build()
        self.assertEqual(code, 0, self.build_output)
        expected = ["cargo build -p midnight-explorer -p midnight-elevation --locked --release", "native release",
                    "network release", "viewer release"]
        if os.name == "nt":
            expected.append("deploy release")
        self.assertEqual(calls, expected)

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

    @unittest.skipUnless(os.name == "nt", "Windows private deployment hook")
    def test_deployment_failure_is_returned_after_app_and_viewers(self):
        code, calls = self.run_build(deploy_exit="47")
        self.assertEqual(code, 47, self.build_output)
        self.assertEqual(calls, ["cargo build -p midnight-explorer -p midnight-elevation --locked --release",
                                 "native release", "network release", "viewer release", "deploy release"])


if __name__ == "__main__":
    unittest.main()
