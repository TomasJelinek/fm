"""Build scripts wait out a brief hold of freshly written files, and nothing else.

Endpoint protection briefly holds files that were just written; while it does, Windows
refuses to rename them or a folder above them (access denied, 5) or reports a sharing
(32) or lock (33) violation. Each language has one small helper per script: Python's
replace_fresh and PowerShell's Invoke-FreshRename. Every copy is exercised here; the
scripts' own tests check that their renames use it.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

# Sibling fixture module, also when run as scripts.tests.<module> from the checkout.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from link_fixtures import POWERSHELL_HOLD, hold  # noqa: E402


ROOT = Path(__file__).resolve().parents[2]
PYTHON_SCRIPTS = ("prepare-html-cef.py", "prepare-thumbnail-ffmpeg.py")
POWERSHELL_SCRIPTS = ("build-html-viewer.ps1", "build-thumbnailers.ps1", "build-filesystem-package.ps1",
                      "deploy-private.ps1")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_").removesuffix(".py"),
                                                  ROOT / "scripts" / name)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def hold_error(code):
    error = PermissionError(13, f"fixture hold {code}")
    error.winerror = code
    return error


class PythonFreshRename(unittest.TestCase):
    """Scripted holds with a fake clock: deterministic on every platform."""

    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="fresh rename ", dir=temporary)
        self.addCleanup(owner.cleanup)
        self.root = Path(owner.name)

    def scripted(self, module, failures, holds=(5, 32, 33)):
        """Rename a new file while the first renames raise `failures` (then succeed)."""
        source, target = self.root / "fresh", self.root / "published"
        source.write_text("fresh output\n")
        target.unlink(missing_ok=True)
        clock = {"now": 100.0, "sleeps": []}
        real_replace = Path.replace
        attempts = []

        def replace(path, destination):
            attempts.append(path)
            if len(attempts) <= len(failures):
                raise failures[len(attempts) - 1]
            return real_replace(path, destination)

        def sleep(seconds):
            clock["sleeps"].append(seconds)
            clock["now"] += seconds

        with mock.patch.object(module, "FRESH_FILE_HOLDS", holds), \
                mock.patch.object(Path, "replace", replace), \
                mock.patch.object(module.time, "monotonic", side_effect=lambda: clock["now"]), \
                mock.patch.object(module.time, "sleep", side_effect=sleep):
            try:
                module.replace_fresh(source, target)
                outcome = None
            except OSError as error:
                outcome = error
        return outcome, attempts, clock, target

    def test_each_hold_error_is_waited_out_with_a_short_backoff(self):
        for name in PYTHON_SCRIPTS:
            with self.subTest(script=name):
                module = load(name)
                failures = [hold_error(5), hold_error(32), hold_error(33)]
                outcome, attempts, clock, target = self.scripted(module, failures)
                self.assertIsNone(outcome)
                self.assertEqual(len(attempts), 4)
                self.assertEqual(target.read_text(), "fresh output\n")
                self.assertTrue(all(0 < delay <= 0.2 for delay in clock["sleeps"]), clock)
                self.assertEqual(clock["sleeps"], sorted(clock["sleeps"]), "The backoff must not shrink")

    def test_a_hold_beyond_two_seconds_raises_the_first_error(self):
        for name in PYTHON_SCRIPTS:
            with self.subTest(script=name):
                module = load(name)
                first = hold_error(32)
                failures = [first] + [hold_error(33) for _ in range(100)]
                outcome, attempts, clock, target = self.scripted(module, failures)
                self.assertIs(outcome, first)
                self.assertAlmostEqual(clock["now"] - 100.0, 2.0)
                self.assertGreater(len(attempts), 5)
                self.assertLess(len(attempts), 30)
                self.assertFalse(target.exists())

    def test_other_errors_and_other_platforms_are_never_retried(self):
        missing = FileNotFoundError(2, "fixture missing")
        missing.winerror = 2
        cases = [(missing, (5, 32, 33)), (PermissionError(13, "plain permission"), (5, 32, 33)),
                 (hold_error(5), ()), (hold_error(32), ())]  # () is every other platform
        for name in PYTHON_SCRIPTS:
            module = load(name)
            for failure, holds in cases:
                with self.subTest(script=name, failure=failure, holds=holds):
                    outcome, attempts, clock, _ = self.scripted(module, [failure], holds)
                    self.assertIs(outcome, failure)
                    self.assertEqual(len(attempts), 1)
                    self.assertEqual(clock["sleeps"], [])

    def test_only_windows_retries(self):
        for name in PYTHON_SCRIPTS:
            with self.subTest(script=name):
                self.assertEqual(load(name).FRESH_FILE_HOLDS, (5, 32, 33) if os.name == "nt" else ())

    @unittest.skipUnless(os.name == "nt", "Windows refuses renames above held files")
    def test_real_hold_of_a_file_in_a_renamed_folder(self):
        for name in PYTHON_SCRIPTS:
            module = load(name)
            for seconds, succeeds in ((0.3, True), (2.6, False)):
                with self.subTest(script=name, hold=seconds):
                    stage = self.root / name / f"stage {seconds}"
                    stage.mkdir(parents=True)
                    (stage / "fresh.txt").write_text("fresh output\n")
                    target = self.root / name / f"published {seconds}"
                    timer = hold(stage / "fresh.txt", seconds)
                    started = time.monotonic()
                    try:
                        if succeeds:
                            module.replace_fresh(stage, target)
                        else:
                            with self.assertRaises(PermissionError) as caught:
                                module.replace_fresh(stage, target)
                        elapsed = time.monotonic() - started
                    finally:
                        timer.join()
                    if succeeds:
                        self.assertEqual((target / "fresh.txt").read_text(), "fresh output\n")
                    else:
                        self.assertEqual(caught.exception.winerror, 5)
                        self.assertGreaterEqual(elapsed, 1.9)
                        self.assertLess(elapsed, seconds)
                        self.assertTrue(stage.is_dir())
                        self.assertFalse(target.exists())


HARNESS = r'''
param([string]$Script, [string]$Root)
$ErrorActionPreference = 'Stop'
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($Script, [ref]$null, [ref]$errors)
$definition = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
    $node.Name -eq 'Invoke-FreshRename' }, $true)
if (-not $definition) { throw "Invoke-FreshRename is missing: $Script" }
. ([scriptblock]::Create($definition.Extent.Text))
''' + POWERSHELL_HOLD + r'''
function Measure-Rename([scriptblock]$Rename) {
    $script:attempts = 0
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $code = 0
    try { Invoke-FreshRename $Rename }
    catch { $code = $_.Exception.HResult; if ($_.Exception.InnerException) { $code = $_.Exception.InnerException.HResult } }
    [ordered]@{ attempts = $script:attempts; milliseconds = $watch.ElapsedMilliseconds; code = $code -band 0xFFFF }
}
function New-Stage([string]$Name) {
    $stage = Join-Path $Root $Name
    New-Item -ItemType Directory -Path $stage | Out-Null
    [IO.File]::WriteAllText((Join-Path $stage 'fresh.txt'), 'fresh output')
    return $stage
}
$results = [ordered]@{}
# A folder is renamed while a file inside it is held, as the package swaps do.
$stage = New-Stage 'folder'
[MidnightFreshHold]::Hold((Join-Path $stage 'fresh.txt'), 300, $true)
$results.folder = Measure-Rename { $script:attempts++; [IO.Directory]::Move($stage, (Join-Path $Root 'folder moved')) }
$results.folder.moved = Test-Path -LiteralPath (Join-Path $Root 'folder moved/fresh.txt')
# A held file replaces an installed one, as the filesystem package and registration do.
$stage = New-Stage 'file'
[IO.File]::WriteAllText((Join-Path $Root 'installed.txt'), 'installed')
[MidnightFreshHold]::Hold((Join-Path $stage 'fresh.txt'), 300, $false)
$results.file = Measure-Rename {
    $script:attempts++; Move-Item -LiteralPath (Join-Path $stage 'fresh.txt') -Destination (Join-Path $Root 'installed.txt') -Force
}
$results.file.moved = [IO.File]::ReadAllText((Join-Path $Root 'installed.txt')) -eq 'fresh output'
$stage = New-Stage 'replace'
[IO.File]::WriteAllText((Join-Path $Root 'record.txt'), 'old record')
[MidnightFreshHold]::Hold((Join-Path $stage 'fresh.txt'), 300, $false)
$results.replace = Measure-Rename {
    $script:attempts++; [IO.File]::Replace((Join-Path $stage 'fresh.txt'), (Join-Path $Root 'record.txt'), [NullString]::Value)
}
$results.replace.moved = [IO.File]::ReadAllText((Join-Path $Root 'record.txt')) -eq 'fresh output'
$stage = New-Stage 'long'
[MidnightFreshHold]::Hold((Join-Path $stage 'fresh.txt'), 2600, $true)
$results.long = Measure-Rename { $script:attempts++; [IO.Directory]::Move($stage, (Join-Path $Root 'long moved')) }
$results.long.moved = Test-Path -LiteralPath (Join-Path $Root 'long moved')
$results.missing = Measure-Rename { $script:attempts++; Move-Item -LiteralPath (Join-Path $Root 'absent') -Destination (Join-Path $Root 'x') }
$results.other = Measure-Rename { $script:attempts++; throw [IO.IOException]::new('fixture: not found', -2147024894) }
$results.first = Measure-Rename {
    $script:attempts++
    $code = if ($script:attempts -eq 1) { -2147024864 } else { -2147024863 }  # 32 first, then 33
    throw [IO.IOException]::new('fixture hold', $code)
}
$results | ConvertTo-Json -Compress
'''


@unittest.skipUnless(os.name == "nt", "Windows PowerShell build helpers")
class PowerShellFreshRename(unittest.TestCase):
    """Each script's Invoke-FreshRename, under PowerShell 7 and Windows PowerShell 5.1."""

    def test_every_copy_waits_out_brief_holds_only(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="fresh rename ps ", dir=temporary)
        self.addCleanup(owner.cleanup)
        harness = Path(owner.name) / "harness.ps1"
        harness.write_text(HARNESS, encoding="utf-8-sig")
        shells = [shell for shell in (shutil.which("pwsh"), "powershell.exe") if shell]
        runs = {}
        # Independent processes, run together: each waits about 2 s for its long hold.
        for script in POWERSHELL_SCRIPTS:
            for shell in shells:
                root = Path(owner.name) / f"{Path(script).stem} {Path(shell).stem}"
                root.mkdir()
                runs[script, shell] = subprocess.Popen(
                    [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(harness),
                     "-Script", str(ROOT / "scripts" / script), "-Root", str(root)],
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        for (script, shell), process in runs.items():
            with self.subTest(script=script, shell=Path(shell).name):
                stdout, stderr = process.communicate(timeout=120)
                self.assertEqual(process.returncode, 0, stdout + stderr)
                results = json.loads(stdout.strip().splitlines()[-1])
                for name in ("folder", "file", "replace"):
                    self.assertEqual(results[name]["code"], 0, results)
                    self.assertTrue(results[name]["moved"], results)
                    self.assertGreater(results[name]["attempts"], 1, "The hold was not exercised")
                self.assertIn(results["long"]["code"], (5, 32), results)
                self.assertFalse(results["long"]["moved"], results)
                self.assertGreaterEqual(results["long"]["milliseconds"], 1900, results)
                self.assertLess(results["long"]["milliseconds"], 2600, results)
                for name in ("missing", "other"):
                    self.assertEqual(results[name]["attempts"], 1, results)
                self.assertEqual(results["other"]["code"], 2, results)
                self.assertEqual(results["first"]["code"], 32, "The first error must be raised")
                self.assertGreater(results["first"]["attempts"], 5, results)


if __name__ == "__main__":
    unittest.main()
