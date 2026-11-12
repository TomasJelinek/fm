"""Exercise the real package wrapper with inert compiler/plugin fixtures."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
PWSH = shutil.which("pwsh")


class ImageViewerBuildWrapperTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="image-wrapper-", dir=temporary)
        self.root = Path(self.owner.name)
        self.assertTrue(self.root.resolve().is_relative_to(temporary.resolve()))
        self.addCleanup(self.owner.cleanup)
        (self.root / "scripts").mkdir()
        (self.root / "sdk.toml").write_text("", encoding="utf-8")

    @unittest.skipUnless(PWSH and os.name == "nt", "Windows PowerShell wrapper")
    def test_native_environment_and_offline_are_restored_on_success_and_failure(self):
        self.powershell_case("image", False)
        self.powershell_case("image", True)

    @unittest.skipUnless(PWSH and os.name == "nt", "Windows PowerShell wrapper")
    def test_pdf_keeps_its_original_parameter_contract(self):
        self.powershell_case("pdf", False)

    def powershell_case(self, viewer, fail):
        shutil.copyfile(ROOT / "scripts/build-image-viewer.ps1",
                        self.root / "scripts/build-image-viewer.ps1")
        plugin = self.root / f"plugins/midnight-viewer-{viewer}/scripts"
        plugin.mkdir(parents=True, exist_ok=True)
        native_parameter = ", [switch]$NativeOffline" if viewer == "image" else ""
        offline_check = "if (-not $NativeOffline) { throw 'offline not forwarded' }" if viewer == "image" else ""
        (plugin / "ci.ps1").write_text(
            f"[CmdletBinding()]\nparam($SdkConfig,$BuildRoot{native_parameter})\n{offline_check}\n",
            encoding="utf-8")
        (plugin / "build.ps1").write_text(
            f"[CmdletBinding()]\nparam($SdkConfig,$BuildRoot,$OutputDirectory,$Profile{native_parameter})\n"
            f"{offline_check}\n"
            "$env:MIDNIGHT_TEST_NEW_NATIVE='introduced'\n"
            "$env:MIDNIGHT_TEST_EMPTY_NATIVE='changed'\n"
            "$env:INCLUDE='changed by native compiler'\n"
            + ("throw 'intentional fixture failure'\n" if fail else ""), encoding="utf-8")
        probe = self.root / "probe.ps1"
        probe.write_text(
            "$ErrorActionPreference='Stop'\n"
            "function global:cargo { $global:LASTEXITCODE=0 }\n"
            "Remove-Item -LiteralPath 'Env:MIDNIGHT_TEST_NEW_NATIVE' -ErrorAction SilentlyContinue\n"
            "[Environment]::SetEnvironmentVariable('MIDNIGHT_TEST_EMPTY_NATIVE','','Process')\n"
            "$env:INCLUDE='original fixture value'\n"
            "$before=[Environment]::GetEnvironmentVariables('Process')\n"
            "$failed=$false\n"
            f"try {{ & \"$PSScriptRoot/scripts/build-image-viewer.ps1\" -WorkerSdkConfig \"$PSScriptRoot/sdk.toml\" "
            f"-AppDirectory \"$PSScriptRoot/_build/app\" -Viewer {viewer} -Offline -Test }}\n"
            "catch { if ($_.Exception.Message -ne 'intentional fixture failure') { throw }; $failed=$true }\n"
            f"if ($failed -ne ${str(fail).lower()}) {{ throw 'unexpected success/failure' }}\n"
            "$after=[Environment]::GetEnvironmentVariables('Process')\n"
            "if ($after.Contains('MIDNIGHT_TEST_NEW_NATIVE')) { throw 'native variable leaked' }\n"
            "foreach($name in @('MIDNIGHT_TEST_EMPTY_NATIVE','INCLUDE','PATH','CARGO_TARGET_DIR','PYTHONPATH')) {\n"
            " if($before.Contains($name) -ne $after.Contains($name)) { throw 'environment presence changed' }\n"
            " if($before[$name] -cne $after[$name]) { throw 'environment value changed' }\n"
            "}\n", encoding="utf-8")
        result = subprocess.run([PWSH, "-NoProfile", "-File", str(probe)], cwd=self.root,
                                capture_output=True, text=True, timeout=30, check=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    @unittest.skipUnless(os.name == "posix", "native Bash wrapper")
    def test_bash_image_offline_does_not_change_pdf_environment(self):
        shutil.copyfile(ROOT / "scripts/build-image-viewer.sh",
                        self.root / "scripts/build-image-viewer.sh")
        tools = self.root / "tools"
        tools.mkdir()
        for name, body in [("cargo", "exit 0"), ("rustc", "echo 'host: x86_64-unknown-linux-gnu'")]:
            path = tools / name
            path.write_text("#!/usr/bin/env bash\n" + body + "\n", encoding="utf-8")
            path.chmod(0o700)
        environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                           MIDNIGHT_VIEWERS_OFFLINE="1", MIDNIGHT_TEST_VIEWERS="1")
        environment.pop("MIDNIGHT_IMAGE_NATIVE_OFFLINE", None)
        for viewer in ["image", "pdf"]:
            plugin = self.root / f"plugins/midnight-viewer-{viewer}/scripts"
            plugin.mkdir(parents=True)
            expected = "1" if viewer == "image" else "absent"
            for script in ["build.sh", "ci.sh"]:
                (plugin / script).write_text(
                    '#!/usr/bin/env bash\nset -euo pipefail\n'
                    f'[[ ${{MIDNIGHT_IMAGE_NATIVE_OFFLINE:-absent}} == {expected} ]]\n',
                    encoding="utf-8")
            result = subprocess.run(
                ["bash", str(self.root / "scripts/build-image-viewer.sh"), "debug",
                 str(self.root / "sdk.toml"), str(self.root / "_build/app"), viewer],
                cwd=self.root, env=environment, capture_output=True, text=True,
                timeout=30, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
