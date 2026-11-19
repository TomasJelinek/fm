"""One installed runtime contains matching video thumbnail and playback executables."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "package-release.py"


class InstalledVideoPackage(unittest.TestCase):
    def test_playback_profile_requires_the_companion_executable_and_resampler(self):
        spec = importlib.util.spec_from_file_location("media_package_release", SCRIPT)
        package = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(package)
        parent = SCRIPT.parents[1] / "_build" / "tests"
        parent.mkdir(parents=True, exist_ok=True)
        for target, suffix, libraries, resampler in (
            ("x86_64-pc-windows-msvc", ".exe",
             ("avcodec-63.dll", "avformat-63.dll", "avutil-61.dll", "swscale-10.dll"),
             "swresample-7.dll"),
            ("x86_64-unknown-linux-gnu", "",
             ("libavcodec.so.63", "libavformat.so.63", "libavutil.so.61", "libswscale.so.10"),
             "libswresample.so.7"),
        ):
            with self.subTest(target=target), tempfile.TemporaryDirectory(dir=parent) as temporary:
                root = Path(temporary)
                source = root / "thumbnailers"
                source.mkdir()
                for kind in ("image", "video"):
                    executable = "midnight-thumbnail-" + kind
                    (source / (executable + suffix)).write_bytes(b"test executable")
                    notice = source / "notices" / executable
                    notice.mkdir(parents=True)
                    (notice / "NOTICE").write_text("Test notice", encoding="utf-8")
                    version = "ffmpeg-9.0.2-playback-r3" if kind == "video" else "1"
                    (source / (executable + ".json")).write_text(json.dumps({
                        "protocol": 1, "id": "midnight-" + kind, "version": version,
                        "executable": executable + suffix,
                        "extensions": ["mp4" if kind == "video" else "png"],
                        "video": kind == "video",
                    }), encoding="utf-8")
                for library in libraries:
                    (source / library).write_bytes(b"test library")
                notices = source / "notices" / "ffmpeg-9.0.2"
                notices.mkdir()
                for name in ("ffmpeg-9.0.2.tar.xz", "COPYING.LGPLv2.1",
                             "build-options.txt", "config.h", "config.mak"):
                    (notices / name).write_bytes(b"test notice")
                def discover():
                    return package.copy_thumbnail_packages(
                        root / "stage", source, target, require_media=True, validate_only=True
                    )
                with self.assertRaisesRegex(ValueError, "matching worker and shared FFmpeg"):
                    discover()
                (source / ("midnight-video-playback" + suffix)).write_bytes(b"test executable")
                with self.assertRaisesRegex(ValueError, "matching worker and shared FFmpeg"):
                    discover()
                (source / resampler).write_bytes(b"test library")
                installed = discover()["providers"]
                self.assertEqual(len(installed), 2)
                self.assertTrue(any("mp4" in provider["extensions"] for provider in installed))


if __name__ == "__main__":
    unittest.main()
