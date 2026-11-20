"""Viewer plugin builds write below _build, reached however the checkout is spelled.

The checkout and the folders above it may be links, a subst drive or hidden folders
such as AppData; _build and every folder below it must not be a reparse point. Image
and PDF builds refuse other outputs. Basic, Markdown and source builds may write
anywhere, but replace notices without their marker only below such a _build.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

# Sibling fixture module, also when run as scripts.tests.<module> from the checkout.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from link_fixtures import directory_link, hidden_directory  # noqa: E402


ROOT = Path(__file__).resolve().parents[2]
REQUIRED_OUTPUT = ("image", "pdf")
MARKED_NOTICES = ("basic", "markdown", "source")
PLUGIN_FILES = {"basic": ("LICENSE", "UNICODE-LICENSE.txt", "README.md", "THIRD-PARTY.md", "plugin.toml"),
                "markdown": ("LICENSE", "README.md", "THIRD-PARTY.md", "plugin.toml"),
                "source": ("LICENSE", "README.md", "THIRD-PARTY.md", "plugin.toml"),
                "image": ("LICENSE", "README.md", "THIRD-PARTY.md", "image-viewer.json"),
                "pdf": ("LICENSE", "README.md", "THIRD-PARTY.md", "image-viewer.json")}
DRIVER = r'''
param([string]$Cases)
$ErrorActionPreference = 'Stop'
# The native toolchain is a stub: rustc reports the Windows host, cargo writes every
# plugin's artifact into the requested target directory.
function global:rustc { 'host: x86_64-pc-windows-msvc'; $global:LASTEXITCODE = 0 }
function global:cargo {
    $directory = Join-Path $args[[array]::IndexOf($args, '--target-dir') + 1] 'x86_64-pc-windows-msvc/debug'
    New-Item -ItemType Directory -Force $directory | Out-Null
    foreach ($name in @('midnight_viewer_basic.dll', 'midnight_viewer_markdown.dll', 'midnight_viewer_source.dll',
            'midnight-viewer-image.exe', 'midnight-viewer-pdf.exe')) {
        [IO.File]::WriteAllText((Join-Path $directory $name), "built $name")
    }
    $global:LASTEXITCODE = 0
}
$results = @()
foreach ($case in (Get-Content -LiteralPath $Cases -Raw | ConvertFrom-Json)) {
    $arguments = @{ SdkConfig = $case.sdk; BuildRoot = $case.buildRoot; Profile = 'debug' }
    if ($case.output) { $arguments.OutputDirectory = $case.output }
    try { & $case.script @arguments | Out-Null; $results += 'ok' }
    catch { $results += [string]$_.Exception.Message }
}
ConvertTo-Json -InputObject @($results) -Compress
'''


@unittest.skipUnless(os.name == "nt", "Windows plugin build scripts")
class ViewerPluginBuildOutputs(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="plugin outputs ", dir=temporary)
        self.addCleanup(owner.cleanup)
        self.owner = Path(owner.name)
        # A repository below hidden AppData, also reached through a junction to AppData.
        self.real = hidden_directory(self.owner / "AppData") / "checkout"
        for name, files in PLUGIN_FILES.items():
            plugin = self.real / "plugins" / f"midnight-viewer-{name}"
            (plugin / "scripts").mkdir(parents=True)
            shutil.copyfile(ROOT / "plugins" / f"midnight-viewer-{name}/scripts/build.ps1",
                            plugin / "scripts/build.ps1")
            if name in ("markdown", "source"):
                shutil.copyfile(ROOT / "plugins" / f"midnight-viewer-{name}/scripts/prepare-sdk.ps1",
                                plugin / "scripts/prepare-sdk.ps1")
            for file in files:
                text = {"plugin.toml": 'entry_library = "template"\ntarget = "template"\n',
                        "image-viewer.json": '{"target": "template", "executable": "template"}\n'}
                (plugin / file).write_text(text.get(file, f"{name} {file}\n"))
            if name != "image":
                (plugin / "notices").mkdir()
                (plugin / "notices/dependency-LICENSE").write_text(f"{name} dependency notice\n")
        decoder = self.real / "plugins/image-decoder"
        (decoder / "native").mkdir(parents=True)
        (decoder / "native/bootstrap-windows.ps1").write_text("param($BuildRoot, [switch]$Offline)\n")
        (decoder / "native/build-windows.ps1").write_text("param($BuildRoot, $Jobs, [switch]$Offline)\n")
        (decoder / "notices").mkdir()
        (decoder / "notices/dependency-LICENSE").write_text("decoder dependency notice\n")
        registry = self.owner / "sdk/registry"
        registry.mkdir(parents=True)
        self.sdk = self.owner / "sdk/.cargo/config.toml"
        self.sdk.parent.mkdir()
        self.sdk.write_text('[source.midnight-releases]\nlocal-registry = "'
                            + registry.as_posix() + '"\n')
        self.checkout = directory_link(self.owner / "linked", self.real.parent) / "checkout"
        self.outside = self.owner / "outside"
        self.outside.mkdir()
        (self.real / "_build").mkdir()
        directory_link(self.real / "_build/redirect", self.outside)
        directory_link(self.real / "_build/loop", self.real)
        self.custom = directory_link(self.owner / "custom target", self.outside)

    def run_cases(self, cases):
        """Run (name, build root, output) cases through the linked checkout's scripts."""
        listing = self.owner / "cases.json"
        listing.write_text(json.dumps([
            {"script": str(self.checkout / "plugins" / f"midnight-viewer-{name}/scripts/build.ps1"),
             "sdk": str(self.sdk), "buildRoot": str(build_root), "output": str(output) if output else None}
            for name, build_root, output in cases]))
        driver = self.owner / "driver.ps1"
        driver.write_text(DRIVER, encoding="utf-8-sig")
        outcomes = {}
        for shell in [shell for shell in (shutil.which("pwsh"), "powershell.exe") if shell]:
            result = subprocess.run([shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(driver),
                                     "-Cases", str(listing)], capture_output=True, text=True, timeout=300)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            outcomes[Path(shell).stem.lower()] = json.loads(result.stdout.strip().splitlines()[-1])
        return outcomes

    def test_image_and_pdf_builds_accept_linked_hidden_checkouts_but_not_links_from_build_down(self):
        cases, expected = [], []
        for name in REQUIRED_OUTPUT:
            plugin = self.checkout / "plugins" / f"midnight-viewer-{name}"
            builds = self.checkout / "_build"
            for build_root, output, outcome in (
                    # The shared repository's _build, and the plugin's own as a standalone project.
                    (builds / f"viewers/{name}", builds / f"debug/viewers/midnight-viewer-{name}", "ok"),
                    (plugin / "_build/build", None, "ok"),
                    (builds / f"redirect/{name}", None, "must not traverse reparse points"),
                    (builds / f"viewers/{name}", builds / f"loop/_build/viewers/{name}", "must not traverse reparse points"),
                    # Outside the checkout the walk starts at the path's first _build, which for
                    # this fixture is the repository's own _build above it.
                    (self.custom / f"_build/{name}", None, "must not traverse reparse points"),
                    (self.checkout / f"built-{name}", None, "must be under _build"),
                    (builds / f"viewers/{name}", self.checkout / f"viewers/{name}", "must be under _build")):
                cases.append((name, build_root, output))
                expected.append(outcome)
        for shell, results in self.run_cases(cases).items():
            for case, outcome, result in zip(cases, expected, results):
                with self.subTest(shell=shell, case=case):
                    if outcome == "ok":
                        self.assertEqual(result, "ok")
                    else:
                        self.assertIn(outcome, result)
        for name in REQUIRED_OUTPUT:
            package = self.real / f"_build/debug/viewers/midnight-viewer-{name}"
            self.assertEqual((package / f"midnight-viewer-{name}.exe").read_text(),
                             f"built midnight-viewer-{name}.exe")
            self.assertTrue((package / "notices/dependency-LICENSE").is_file())
            self.assertTrue((self.real / "plugins" / f"midnight-viewer-{name}/_build/build/package"
                             / f"midnight-viewer-{name}/image-viewer.json").is_file())
        self.assertEqual(list(self.outside.iterdir()), [])
        self.assertFalse(any(self.checkout.glob("built-*")))

    def test_unmarked_notices_are_replaced_only_below_build(self):
        cases, legacy = [], {}
        for name in MARKED_NOTICES:
            builds = self.checkout / "_build"
            for key, output in (("linked", builds / f"debug/viewers/midnight-viewer-{name}"),
                                ("redirect", builds / f"redirect/{name}"),
                                ("outside", self.checkout / f"installed/{name}")):
                # Notices without this script's marker: an older build's or someone else's.
                notices = output / "notices"
                notices.mkdir(parents=True)
                (notices / "unmarked-LICENSE").write_text("not written by this script\n")
                legacy[name, key] = notices
                cases.append((name, builds / f"viewers/{name}", output))
        for shell, results in self.run_cases(cases).items():
            with self.subTest(shell=shell):
                for (name, _, output), result in zip(cases, results):
                    key = next(key for (plugin, key), notices in legacy.items()
                               if plugin == name and notices == output / "notices")
                    if key == "linked":
                        self.assertEqual(result, "ok", (name, key))
                    else:
                        self.assertIn("Refusing to replace notices", result, (name, key))
        for name in MARKED_NOTICES:
            replaced = self.real / f"_build/debug/viewers/midnight-viewer-{name}/notices"
            self.assertEqual(sorted(path.name for path in replaced.iterdir()),
                             [".midnight-generated", "dependency-LICENSE"])
            self.assertEqual((self.outside / name / "notices/unmarked-LICENSE").read_text(),
                             "not written by this script\n")
            self.assertEqual((self.real / f"installed/{name}/notices/unmarked-LICENSE").read_text(),
                             "not written by this script\n")


if __name__ == "__main__":
    unittest.main()
