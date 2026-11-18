"""Exercise the real package wrapper with inert compiler/plugin fixtures."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
PWSH = shutil.which("pwsh")
WRAPPER_SCRIPTS = ("build-image-viewer.ps1", "utf8-native-output.ps1")


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
        for name in WRAPPER_SCRIPTS:
            shutil.copyfile(ROOT / "scripts" / name, self.root / "scripts" / name)
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


@unittest.skipUnless(os.name == "nt", "Windows PowerShell wrapper")
class ImageViewerWorkerSdkWindows(unittest.TestCase):
    """The real wrapper packages the worker SDK; only Cargo and the plugin build are stubs."""

    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="image sdk ", dir=temporary)
        self.addCleanup(owner.cleanup)
        self.owner = Path(owner.name)
        # cmd.exe reads a batch file in the console code page: keep the stubs on an ASCII path.
        tools = self.owner / "bin"
        tools.mkdir()
        self.log = self.owner / "cargo.jsonl"
        self.sdk_report = self.owner / "plugin-sdk.txt"
        (tools / "cargo.py").write_text(
            "import json, os, pathlib, sys\n"
            "arguments = sys.argv[1:]\n"
            "with open(os.environ['IMAGE_TEST_LOG'], 'a', encoding='utf-8') as log:\n"
            "    log.write(json.dumps(arguments) + '\\n')\n"
            "if 'run' in arguments:\n"
            "    # Like worker-packages: a new bundle, reported in UTF-8 whatever the console code page.\n"
            "    bundle = pathlib.Path(arguments[-1]) / 'worker-sdk-1'\n"
            "    (bundle / '.cargo').mkdir(parents=True)\n"
            "    (bundle / '.cargo/registry.toml').write_text('# worker SDK fixture\\n')\n"
            "    sys.stdout.buffer.write(b'Public worker API/SDK release bundle: ' + str(bundle).encode() + b'\\n')\n",
            encoding="utf-8")
        (tools / "cargo.cmd").write_text(f'@"{sys.executable}" "{tools / "cargo.py"}" %*\n')
        self.environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                                IMAGE_TEST_LOG=str(self.log), IMAGE_TEST_SDK=str(self.sdk_report))
        for name in ("CARGO_TARGET_DIR", "MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG", "MIDNIGHT_VIEWERS_OFFLINE"):
            self.environment.pop(name, None)

    def checkout(self, parent):
        checkout = parent / "checkout"
        (checkout / "scripts").mkdir(parents=True)
        for name in WRAPPER_SCRIPTS:
            shutil.copyfile(ROOT / "scripts" / name, checkout / "scripts" / name)
        for directory in ("api/tools/package", "worker-api", "thumbnail-api", "filesystem-api", "image-viewer-api"):
            (checkout / directory).mkdir(parents=True)
            (checkout / directory / "source").write_text("source fixture\n")
        plugin = checkout / "plugins/midnight-viewer-image/scripts"
        plugin.mkdir(parents=True)
        (plugin / "build.ps1").write_text(
            "param($SdkConfig, $BuildRoot, $OutputDirectory, $Profile, [switch]$NativeOffline)\n"
            "[IO.File]::WriteAllText($env:IMAGE_TEST_SDK, $SdkConfig)\n", encoding="utf-8")
        return checkout

    def run_wrapper(self, checkout, code_page=0):
        """Run the wrapper from a console of its own, set to `code_page` unless it is 0."""
        report = self.owner / "code-page.txt"
        driver = self.owner / "driver.ps1"
        driver.write_text(
            "param([string]$Script, [string]$App, [string]$Report, [int]$CodePage)\n"
            "if ($CodePage) { [Console]::OutputEncoding = [Text.Encoding]::GetEncoding($CodePage) }\n"
            "try { & $Script -AppDirectory $App }\n"
            "finally { [IO.File]::WriteAllText($Report, [string][Console]::OutputEncoding.CodePage) }\n",
            encoding="utf-8")
        # The code page belongs to the console, not to the process: CREATE_NO_WINDOW gives
        # the wrapper a console of its own and leaves the test runner's console untouched.
        result = subprocess.run(
            [PWSH or "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(driver),
             "-Script", str(checkout / "scripts/build-image-viewer.ps1"), "-App", str(checkout / "_build/app"),
             "-Report", str(report), "-CodePage", str(code_page)],
            cwd=checkout, env=self.environment, capture_output=True, encoding="utf-8", errors="replace",
            timeout=120, creationflags=subprocess.CREATE_NO_WINDOW)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return report.read_text()

    def cargo_calls(self, command):
        calls = [json.loads(line) for line in self.log.read_text(encoding="utf-8").splitlines()]
        return [call for call in calls if command in call]

    def test_a_fresh_checkout_packages_the_sdk_once_and_caches_it_beside_the_viewer_build(self):
        # No other module's build directory exists yet, as on a first build.
        checkout = self.checkout(self.owner)
        for _ in range(2):
            self.run_wrapper(checkout)
        self.assertEqual(len(self.cargo_calls("run")), 1)
        bundle = checkout / "_build/image-viewer/releases/worker-sdk-1"
        state = json.loads((checkout / "_build/image-viewer/sdk-state.json").read_text())
        self.assertEqual(Path(state["config"]), bundle / ".cargo/registry.toml")
        # Other modules' caches are only read.
        self.assertFalse((checkout / "_build/pdf-viewer").exists())
        self.assertFalse((checkout / "_build/thumbnailers").exists())
        self.assertEqual(Path(self.sdk_report.read_text(encoding="utf-8")), bundle / ".cargo/registry.toml")

    def test_worker_sdk_below_a_non_ascii_path_survives_a_legacy_console_code_page(self):
        # worker-packages prints its bundle path in UTF-8, but PowerShell decodes native
        # output with the console code page, so the SDK below a non-ASCII checkout was lost.
        checkout = self.checkout(self.owner / "Jiří 文档")
        self.assertEqual(self.run_wrapper(checkout, 850), "850", "The caller's console encoding was not restored")
        config = checkout / "_build/image-viewer/releases/worker-sdk-1/.cargo/registry.toml"
        self.assertEqual(Path(self.sdk_report.read_text(encoding="utf-8")), config)
        plugin_fetch = self.cargo_calls("fetch")[-1]
        self.assertEqual(Path(plugin_fetch[plugin_fetch.index("--config") + 1]), config)


if __name__ == "__main__":
    unittest.main()
