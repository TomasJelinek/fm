"""Normal Windows deployment requires both thumbnail providers and runtime notices."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
REQUIRED = (
    "midnight-thumbnail-image.exe", "midnight-thumbnail-image.json",
    "midnight-thumbnail-video.exe", "midnight-thumbnail-video.json",
    "avcodec-63.dll", "avformat-63.dll", "avutil-61.dll", "swscale-10.dll",
    "notices/midnight-thumbnail-image/inventory.json",
    "notices/midnight-thumbnail-video/inventory.json",
    "notices/ffmpeg-9.0.2/ffmpeg-9.0.2.tar.xz", "notices/ffmpeg-9.0.2/COPYING.LGPLv2.1",
    "notices/ffmpeg-9.0.2/build-options.txt", "notices/ffmpeg-9.0.2/config.h", "notices/ffmpeg-9.0.2/config.mak",
)


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
        (self.root / "scripts/deploy-private.ps1").write_text(
            'param([string]$SourceDirectory,[switch]$RegisterRuntime,[switch]$IncludeFilesystems,'
            '[switch]$IncludeThumbnailers)\n'
            'if (-not $IncludeThumbnailers -or -not $RegisterRuntime) { throw "Missing required deployment flags" }\n'
            '[IO.File]::WriteAllText($env:DEPLOY_TEST_LOG,$SourceDirectory)\n')
        self.source = self.root / "_build/custom/debug"
        self.thumbnails = self.source / "thumbnailers"
        for relative in REQUIRED:
            path = self.thumbnails / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture\n")
        self.log = self.root / "deployment-called"
        self.environment = dict(os.environ, CARGO_TARGET_DIR=str(self.source.parent), DEPLOY_TEST_LOG=str(self.log))

    def deploy(self):
        return subprocess.run(["powershell.exe", "-NoLogo", "-NoProfile", "-File",
                               str(self.root / "scripts/deploy-build.ps1")], env=self.environment,
                              capture_output=True, text=True, timeout=30)

    def test_complete_thumbnail_installation_is_always_deployed(self):
        result = self.deploy()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.log.read_text(), str(self.source))

    def test_every_required_thumbnail_artifact_is_checked_before_deployment(self):
        for relative in REQUIRED:
            with self.subTest(missing=relative):
                path = self.thumbnails / relative
                path.unlink()
                result = self.deploy()
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertFalse(self.log.exists(), "Partial build reached deployment")
                path.write_text("fixture\n")

    def test_missing_thumbnail_directory_never_becomes_an_optional_deployment(self):
        shutil.rmtree(self.thumbnails)
        result = self.deploy()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
