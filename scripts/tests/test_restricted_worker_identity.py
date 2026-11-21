"""The build and deployment scripts grant library access to the SID the workers run with.

The restricted workers run in the AppContainer `MidnightExplorer.RestrictedWorker.v1`, created
by the unpackaged application. Its loader opens the FFmpeg libraries beside the playback and
thumbnail executables only when their ACL grants that AppContainer's SID. Windows derives a
different SID for the same name inside a packaged process: PowerShell 7 from the Microsoft
Store got a child AppContainer of its own package from DeriveAppContainerSidFromAppContainerName,
so the libraries were granted to a SID no worker has and every video helper failed with
STATUS_DLL_NOT_FOUND. Each script's Get-RestrictedWorkerSid is run here under PowerShell 7 and
Windows PowerShell 5.1 and compared with the SID the operating system assigns to an unpackaged
caller (crates/midnight-worker-platform pins the same value for the workers themselves).
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
# The scripts that must grant the workers' libraries; any other script that defines the
# function (the Windows acceptance runner does) is checked too.
REQUIRED = ("build-thumbnailers.ps1", "deploy-private.ps1")
SCRIPTS = tuple(sorted({*REQUIRED, *(
    path.name for path in (ROOT / "scripts").glob("*.ps1")
    if "function Get-RestrictedWorkerSid" in path.read_text(encoding="utf-8"))}))
# Every PowerShell script, tests included: pwsh may run any of them as a packaged app.
POWERSHELL = sorted(path for pattern in ("*.ps1", "*.psm1") for path in (ROOT / "scripts").rglob(pattern))
# The SID of the top-level AppContainer `MidnightExplorer.RestrictedWorker.v1`.
WORKER_SID = "S-1-15-2-210492624-3469538162-618285203-1868907-2672626540-1166629363-1118814998"

HARNESS = r'''
param([string]$Script)
$ErrorActionPreference = 'Stop'
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($Script, [ref]$null, [ref]$errors)
$definition = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
    $node.Name -eq 'Get-RestrictedWorkerSid' }, $true)
if (-not $definition) { throw "Get-RestrictedWorkerSid is missing: $Script" }
. ([scriptblock]::Create($definition.Extent.Text))
$sid = Get-RestrictedWorkerSid
if ($sid -isnot [Security.Principal.SecurityIdentifier]) { throw 'Expected a SecurityIdentifier' }
$sid.Value
'''

# An unpackaged process asks Windows itself. Windows PowerShell 5.1 is never packaged.
OPERATING_SYSTEM = r'''
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Security.Principal;
public static class RestrictedWorkerIdentityOracle {
    [DllImport("userenv.dll", CharSet=CharSet.Unicode)] static extern int DeriveAppContainerSidFromAppContainerName(string name, out IntPtr sid);
    [DllImport("advapi32.dll")] static extern IntPtr FreeSid(IntPtr sid);
    public static string Get() {
        IntPtr sid;
        int result = DeriveAppContainerSidFromAppContainerName("MidnightExplorer.RestrictedWorker.v1", out sid);
        if (result != 0) Marshal.ThrowExceptionForHR(result);
        try { return new SecurityIdentifier(sid).Value; } finally { FreeSid(sid); }
    }
}
"@
[RestrictedWorkerIdentityOracle]::Get()
'''


def shells():
    return [shell for shell in (shutil.which("pwsh"), shutil.which("powershell.exe")) if shell]


@unittest.skipUnless(os.name == "nt", "Windows AppContainer identity")
class RestrictedWorkerIdentity(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="worker identity ", dir=temporary)
        self.addCleanup(owner.cleanup)
        self.root = Path(owner.name)

    def run_powershell(self, shell, text, *arguments):
        script = self.root / f"{Path(shell).stem}-{len(list(self.root.iterdir()))}.ps1"
        script.write_text(text, encoding="utf-8-sig")
        result = subprocess.run([shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script),
                                 *arguments], capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout.strip().splitlines()[-1]

    def test_the_pinned_sid_is_the_one_windows_assigns_an_unpackaged_caller(self):
        shell = shutil.which("powershell.exe")
        if not shell:
            self.skipTest("Windows PowerShell 5.1 is unavailable")
        self.assertEqual(self.run_powershell(shell, OPERATING_SYSTEM), WORKER_SID)

    def test_each_script_grants_the_sid_the_workers_run_with_under_every_powershell(self):
        self.assertTrue(shells(), "No PowerShell is available")
        harness = HARNESS
        for script in SCRIPTS:
            for shell in shells():
                with self.subTest(script=script, shell=Path(shell).name):
                    self.assertEqual(self.run_powershell(shell, harness, "-Script", str(ROOT / "scripts" / script)),
                                     WORKER_SID)

    def test_no_script_asks_windows_for_the_sid_in_its_own_process(self):
        # In a packaged PowerShell these APIs return a child AppContainer of the package.
        self.assertTrue(POWERSHELL, "No PowerShell scripts were found")
        for script in POWERSHELL:
            with self.subTest(script=str(script.relative_to(ROOT))):
                # A declaration or call; comments may name the API to explain why not.
                text = script.read_text(encoding="utf-8")
                self.assertNotIn("DeriveAppContainerSidFromAppContainerName(", text)
                self.assertNotIn("CreateAppContainerProfile(", text)
        for script in REQUIRED:
            with self.subTest(required=script):
                self.assertIn("function Get-RestrictedWorkerSid",
                              (ROOT / "scripts" / script).read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
