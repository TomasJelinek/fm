"""Every script names the FFmpeg release, source digest and libraries that the preparer pins.

The pinned runtime appears in the Windows and Linux build, deployment and release
scripts. An upgrade must change all of them together; this test names any copy
left behind instead of letting a build ship mismatched libraries or notices.
"""
import importlib.util
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"


def pinned():
    spec = importlib.util.spec_from_file_location("ffmpeg_pin", SCRIPTS / "prepare-thumbnail-ffmpeg.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    windows = {Path(name).name for name in module.runtime_names("win32")}
    linux = {Path(name).name for name in module.runtime_names("linux")}
    return module.VERSION, module.SOURCE_SHA256, windows, linux


def libraries(text):
    return set(re.findall(r"\b(?:avcodec|avformat|avutil|swscale)-\d+\.dll\b", text)) | set(
        re.findall(r"\blib(?:avcodec|avformat|avutil|swscale)\.so\.\d+\b", text))


class FfmpegPinConsistency(unittest.TestCase):
    def test_build_deployment_and_release_scripts_use_the_pinned_runtime(self):
        version, digest, windows, linux = pinned()
        expected_libraries = {
            "build-thumbnailers.ps1": windows, "deploy-private.ps1": windows,
            "build-thumbnailers.sh": linux, "package-release.py": windows | linux,
        }
        for name, expected in expected_libraries.items():
            with self.subTest(script=name):
                self.assertEqual(libraries((SCRIPTS / name).read_text(encoding="utf-8")), expected)
        for name in ("build-thumbnailers.ps1", "build-thumbnailers.sh", "package-release.py",
                     "build-media-ffmpeg.sh", "prepare-thumbnail-ffmpeg.py"):
            with self.subTest(script=name):
                text = (SCRIPTS / name).read_text(encoding="utf-8")
                versions = set(re.findall(r"(?i)ffmpeg[- ](\d+\.\d+\.\d+)", text))
                versions |= set(re.findall(r"== '(\d+\.\d+\.\d+)'", text))
                self.assertEqual(versions, {version})
        for name in ("build-thumbnailers.ps1", "build-thumbnailers.sh"):
            with self.subTest(script=name):
                text = (SCRIPTS / name).read_text(encoding="utf-8")
                self.assertEqual(set(re.findall(r"\b[0-9a-f]{64}\b", text)), {digest})


if __name__ == "__main__":
    unittest.main()
