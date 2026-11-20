"""Build scripts wait out a brief hold of freshly written files, and nothing else.

Endpoint protection briefly holds files that were just written; while it does, Windows
refuses to rename them or a folder above them (access denied, 5) or reports a sharing
(32) or lock (33) violation. Each language has one small helper per script: Python's
replace_fresh and PowerShell's Invoke-FreshRename. Every copy is exercised here, with a
pinned 2 s default that the scripts never change and shorter budgets for real holds;
the scripts' own tests check that their renames use it (one per language waits the
real 2 s: the CEF SDK and thumbnail package swaps).
"""
import ast
import importlib.util
import inspect
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

    def test_scripts_keep_the_two_second_default(self):
        # Other tests pass shorter timeouts; this pins the one every script relies on.
        for name in PYTHON_SCRIPTS:
            with self.subTest(script=name):
                self.assertEqual(inspect.signature(load(name).replace_fresh).parameters["timeout"].default, 2.0)
                tree = ast.parse((ROOT / "scripts" / name).read_text(encoding="utf-8"))
                calls = [node for node in ast.walk(tree) if isinstance(node, ast.Call)
                         and getattr(node.func, "id", None) == "replace_fresh"]
                self.assertTrue(calls)
                self.assertTrue(all(len(call.args) == 2 and not call.keywords for call in calls),
                                "A script passes its own retry timeout")

    @unittest.skipUnless(os.name == "nt", "Windows refuses renames above held files")
    def test_real_hold_of_a_file_in_a_renamed_folder(self):
        # A 150 ms hold is waited out within the default budget. A hold far longer than a
        # 100 ms budget makes the rename give up with its own first error (WinError 5); the
        # hold is then released at once, so its length costs no time.
        for name in PYTHON_SCRIPTS:
            module = load(name)
            for seconds, timeout in ((0.15, None), (2.0, 0.1)):
                with self.subTest(script=name, hold=seconds, timeout=timeout):
                    stage = self.root / name / f"stage {seconds}"
                    stage.mkdir(parents=True)
                    (stage / "fresh.txt").write_text("fresh output\n")
                    target = self.root / name / f"published {seconds}"
                    held = hold(stage / "fresh.txt", seconds)
                    self.addCleanup(held.release)
                    started = time.monotonic()
                    if timeout is None:
                        module.replace_fresh(stage, target)
                        self.assertGreaterEqual(time.monotonic() - started, 0.1, "The hold was not met")
                        self.assertEqual((target / "fresh.txt").read_text(), "fresh output\n")
                        continue
                    with self.assertRaises(PermissionError) as caught:
                        module.replace_fresh(stage, target, timeout=timeout)
                    elapsed = time.monotonic() - started
                    held.release()
                    self.assertEqual(caught.exception.winerror, 5)
                    self.assertGreaterEqual(elapsed, timeout)
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
# The budget this script relies on, and its calls: none of them may pass another one.
$budget = @($definition.Body.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -eq 'TimeoutMilliseconds' })
$calls = @($ast.FindAll({ param($node) $node -is [Management.Automation.Language.CommandAst] -and
    $node.GetCommandName() -eq 'Invoke-FreshRename' }, $true))
''' + POWERSHELL_HOLD + r'''
function Measure-Rename([scriptblock]$Rename, [int]$Budget = 0) {
    $script:attempts = 0
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $code = 0
    try { if ($Budget) { Invoke-FreshRename $Rename -TimeoutMilliseconds $Budget } else { Invoke-FreshRename $Rename } }
    catch { $code = $_.Exception.HResult; if ($_.Exception.InnerException) { $code = $_.Exception.InnerException.HResult } }
    [ordered]@{ attempts = $script:attempts; milliseconds = $watch.ElapsedMilliseconds; code = $code -band 0xFFFF }
}
function New-Stage([string]$Name) {
    $stage = Join-Path $Root $Name
    New-Item -ItemType Directory -Path $stage | Out-Null
    [IO.File]::WriteAllText((Join-Path $stage 'fresh.txt'), 'fresh output')
    return $stage
}
$results = [ordered]@{ budget = $budget[0].DefaultValue.Value
    calls = $calls.Count; arguments = @($calls | ForEach-Object { $_.CommandElements.Count - 1 } | Sort-Object -Unique) }
# The files that fresh ones replace are written first, so that they are not fresh themselves.
[IO.File]::WriteAllText((Join-Path $Root 'installed.txt'), 'installed')
foreach ($try in 1..3) { [IO.File]::WriteAllText((Join-Path $Root "record $try.txt"), 'old record') }
# First calls compile; warm up so that each short hold meets the rename it is meant for.
$warm = New-Stage 'warm'
$null = Measure-Rename { $script:attempts++; [IO.Directory]::Move($warm, (Join-Path $Root 'warm moved')) }
# A folder is renamed while a file inside it is held, as the package swaps do.
$stage = New-Stage 'folder'
Start-FreshHold (Join-Path $stage 'fresh.txt') 150 $true
$results.folder = Measure-Rename { $script:attempts++; [IO.Directory]::Move($stage, (Join-Path $Root 'folder moved')) }
$results.folder.moved = Test-Path -LiteralPath (Join-Path $Root 'folder moved/fresh.txt')
# A held file replaces an installed one, as the filesystem package and registration do.
$stage = New-Stage 'file'
Start-FreshHold (Join-Path $stage 'fresh.txt') 150 $false
$results.file = Measure-Rename {
    $script:attempts++; Move-Item -LiteralPath (Join-Path $stage 'fresh.txt') -Destination (Join-Path $Root 'installed.txt') -Force
}
$results.file.moved = [IO.File]::ReadAllText((Join-Path $Root 'installed.txt')) -eq 'fresh output'
# When something outside this test also holds the replaced file, File.Replace reports that it
# cannot remove it (1175), which the helper rightly does not retry; as for other uncontrolled
# holds (TEST-04), only that case is run again, on fresh files.
foreach ($try in 1..3) {
    $stage = New-Stage "replace $try"
    $record = Join-Path $Root "record $try.txt"
    Start-FreshHold (Join-Path $stage 'fresh.txt') 150 $false
    $results.replace = Measure-Rename {
        $script:attempts++; [IO.File]::Replace((Join-Path $stage 'fresh.txt'), $record, [NullString]::Value)
    }
    $results.replace.tries = $try
    $results.replace.moved = [IO.File]::ReadAllText($record) -eq 'fresh output'
    if ($results.replace.code -ne 1175) { break }
}
# A hold far longer than a 100 ms budget: the rename's first error is raised. The hold
# ends with this process, so its length costs no time.
$stage = New-Stage 'long'
Start-FreshHold (Join-Path $stage 'fresh.txt') 2000 $true
$results.long = Measure-Rename { $script:attempts++; [IO.Directory]::Move($stage, (Join-Path $Root 'long moved')) } 100
$results.long.moved = Test-Path -LiteralPath (Join-Path $Root 'long moved')
$results.missing = Measure-Rename { $script:attempts++; Move-Item -LiteralPath (Join-Path $Root 'absent') -Destination (Join-Path $Root 'x') }
$results.other = Measure-Rename { $script:attempts++; throw [IO.IOException]::new('fixture: not found', -2147024894) }
$results.first = Measure-Rename {
    $script:attempts++
    $code = if ($script:attempts -eq 1) { -2147024864 } else { -2147024863 }  # 32 first, then 33
    throw [IO.IOException]::new('fixture hold', $code)
} 100
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
        # Independent processes, run together: PowerShell's start-up dominates their time.
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
                self.assertEqual(results["budget"], 2000, "The scripts' retry budget changed")
                self.assertGreater(results["calls"], 0)
                self.assertEqual(results["arguments"], [1], "A call passes its own retry budget")
                for name in ("folder", "file", "replace"):
                    self.assertEqual(results[name]["code"], 0, results)
                    self.assertTrue(results[name]["moved"], results)
                    self.assertGreater(results[name]["attempts"], 1, "The hold was not exercised")
                self.assertIn(results["long"]["code"], (5, 32), results)
                self.assertFalse(results["long"]["moved"], results)
                self.assertGreater(results["long"]["attempts"], 1, results)
                self.assertGreaterEqual(results["long"]["milliseconds"], 100, results)
                self.assertLess(results["long"]["milliseconds"], 2000, results)
                for name in ("missing", "other"):
                    self.assertEqual(results[name]["attempts"], 1, results)
                self.assertEqual(results["other"]["code"], 2, results)
                self.assertEqual(results["first"]["code"], 32, "The first error must be raised")
                self.assertGreater(results["first"]["attempts"], 1, results)


if __name__ == "__main__":
    unittest.main()
