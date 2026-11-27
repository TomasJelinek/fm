"""Normal Windows deployment validates the complete runtime before replacing a build deployment.

The per-file module matrix runs in process (test_runtime_packages); these cases keep
the PowerShell boundary: validator success/failure and the forwarded deployment flags.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_runtime_packages import complete_runtime


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(os.name == "nt", "Windows normal deployment helper")
class NormalDeploymentTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="deploy build ", dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name)
        (self.root / "scripts").mkdir()
        shutil.copyfile(ROOT / "scripts/deploy-build.ps1", self.root / "scripts/deploy-build.ps1")
        shutil.copyfile(ROOT / "scripts/package-release.py", self.root / "scripts/package-release.py")
        (self.root / "scripts/deploy-private.ps1").write_text(
            'param([string]$SourceDirectory,[switch]$RegisterRuntime,[switch]$IncludeFilesystems,'
            '[switch]$IncludeThumbnailers,[switch]$ReplaceBuildDeployment)\n'
            'if (-not $IncludeThumbnailers -or -not $IncludeFilesystems -or -not $RegisterRuntime -or '
            '-not $ReplaceBuildDeployment) { throw "Missing required deployment flags" }\n'
            '[IO.File]::WriteAllText($env:DEPLOY_TEST_LOG,$SourceDirectory)\n')
        self.source = self.root / "_build/custom/debug"
        self.thumbnails = self.source / "thumbnailers"
        complete_runtime(self.source, 'x86_64-pc-windows-msvc')
        self.log = self.root / "deployment-called"
        self.environment = dict(os.environ, CARGO_TARGET_DIR=str(self.source.parent), DEPLOY_TEST_LOG=str(self.log))

    def deploy(self):
        # A guard against a hang, not a duration: the script takes about a second, but a
        # hosted runner's first Windows PowerShell start once took longer than 60 s.
        return subprocess.run(["powershell.exe", "-NoLogo", "-NoProfile", "-File",
                               str(self.root / "scripts/deploy-build.ps1")], env=self.environment,
                              capture_output=True, text=True, timeout=180)

    def test_complete_runtime_replaces_the_build_deployment(self):
        result = self.deploy()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.log.read_text(), str(self.source))

    def test_missing_ffmpeg_runtime_library_never_reaches_deployment(self):
        (self.thumbnails / "avcodec-63.dll").unlink()
        result = self.deploy()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.log.exists(), "Partial build reached deployment")

    def test_missing_thumbnail_directory_never_becomes_an_optional_deployment(self):
        shutil.rmtree(self.thumbnails)
        result = self.deploy()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
