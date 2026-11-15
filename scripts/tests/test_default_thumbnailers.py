"""Normal thumbnail builds prepare both independent providers or fail as a unit."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux thumbnail build wrapper")
class DefaultThumbnailerTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="thumbnail wrapper ", dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name) / "checkout & dollar$"
        (self.root / "scripts").mkdir(parents=True)
        wrapper = ROOT / "scripts/build-default-thumbnailers.sh"
        if wrapper.exists():
            shutil.copyfile(wrapper, self.root / "scripts/build-default-thumbnailers.sh")
        self.log = self.root / "calls.jsonl"
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.config = self.root / "worker SDK/registry.toml"
        self.config.parent.mkdir()
        self.config.write_text("# local registry fixture\n")
        for directory in ("api/tools/package", "worker-api", "thumbnail-api", "filesystem-api", "image-viewer-api"):
            path = self.root / directory
            path.mkdir(parents=True)
            (path / "source").write_text("source fixture\n")
        for name in ("image", "video"):
            path = self.root / f"plugins/midnight-thumbnail-{name}"
            path.mkdir(parents=True)
            (path / "Cargo.toml").write_text(f'[package]\nname="midnight-thumbnail-{name}"\n')
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                                THUMBNAIL_TEST_LOG=str(self.log), THUMBNAIL_TEST_SDK=str(self.config))
        for name in ("CARGO_TARGET_DIR", "MIDNIGHT_THUMBNAIL_SDK_CONFIG", "MIDNIGHT_THUMBNAILERS_OFFLINE",
                     "MIDNIGHT_VIEWERS_OFFLINE", "MIDNIGHT_IMAGE_NATIVE_OFFLINE"):
            self.environment.pop(name, None)
        self.program(self.bin / "cargo", '''
record("cargo", sys.argv[1:])
if "run" in sys.argv:
    print("Public worker API/SDK release bundle: " + str(Path(os.environ["THUMBNAIL_TEST_SDK"]).parent))
''')
        # The SDK packager's directory contains .cargo/registry.toml.
        (self.config.parent / ".cargo").mkdir()
        shutil.copyfile(self.config, self.config.parent / ".cargo/registry.toml")
        self.program(self.root / "scripts/prepare-thumbnail-ffmpeg.py", '''
record("ffmpeg", sys.argv[1:])
if os.environ.get("THUMBNAIL_TEST_FFMPEG_FAILURE"):
    raise SystemExit(41)
print(json.dumps({"prefix": str(Path.cwd() / "_build/native prefix"),
                  "source_archive": str(Path.cwd() / "_build/source archive.tar.xz"),
                  "libclang_path": "" if os.environ.get("THUMBNAIL_TEST_MISSING_CLANG") else str(Path.cwd() / ".tools/libclang")}))
''')
        self.program(self.root / "scripts/thumbnail-notices.py", '''
record("notices", sys.argv[1:])
if os.environ.get("THUMBNAIL_TEST_NOTICES_FAILURE"):
    raise SystemExit(42)
''')
        self.program(self.root / "scripts/build-thumbnailers.py", '''
record("build", sys.argv[1:])
if os.environ.get("THUMBNAIL_TEST_BUILD_FAILURE"):
    raise SystemExit(43)
''')

        (self.root / "scripts/build-thumbnailers.sh").write_text(
            '#!/bin/bash\nexec python3 "$(dirname -- "$0")/build-thumbnailers.py" "$@"\n')

    def program(self, path, body):
        # Executables replace compiler/network/license acquisition boundaries only.
        path.write_text(f"#!{sys.executable}\nimport json, os, sys\nfrom pathlib import Path\n"
                        "def record(name, args):\n"
                        "    with open(os.environ['THUMBNAIL_TEST_LOG'], 'a') as output:\n"
                        "        output.write(json.dumps([name, args, os.environ.get('MIDNIGHT_IMAGE_NATIVE_OFFLINE')]) + '\\n')\n"
                        + body)
        path.chmod(0o755)

    def run_build(self, *arguments):
        result = subprocess.run(["bash", str(self.root / "scripts/build-default-thumbnailers.sh"), *arguments],
                                cwd=self.root.parent, env=self.environment, text=True,
                                capture_output=True, timeout=20)
        self.output = result.stdout + result.stderr
        return result.returncode

    def calls(self, name):
        records = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return [record for record in records if record[0] == name]

    def test_default_build_packages_sdk_and_fetches_both_providers_in_debug(self):
        self.assertEqual(self.run_build(), 0, self.output)
        builds = self.calls("build")
        self.assertEqual(len(builds), 1)
        self.assertEqual(builds[0][1][-1], "debug")
        self.assertEqual(builds[0][1][1], str(self.root / "_build/debug/thumbnailers"))
        fetches = [call[1] for call in self.calls("cargo") if "fetch" in call[1]]
        for provider in ("image", "video"):
            manifest = str(self.root / f"plugins/midnight-thumbnail-{provider}/Cargo.toml")
            self.assertTrue(any(manifest in args and "--locked" in args for args in fetches), fetches)
        self.assertEqual(len([call for call in self.calls("cargo") if "run" in call[1]]), 1)
        self.assertEqual(self.calls("build")[0][1][4:7], [
            str(self.root / "_build/native prefix"), str(self.root / "_build/source archive.tar.xz"),
            str(self.root / ".tools/libclang")])

    def test_existing_image_viewer_sdk_cache_is_reused_when_current(self):
        self.assertEqual(self.run_build(), 0, self.output)
        image_cache = self.root / "_build/image-viewer"
        image_cache.mkdir()
        for name in ("linux-sdk-config", "linux-sdk-stamp"):
            (self.root / "_build/thumbnailers" / name).rename(image_cache / name)
        self.assertEqual(self.run_build(), 0, self.output)
        self.assertEqual(len([call for call in self.calls("cargo") if "run" in call[1]]), 1)

    def test_explicit_sdk_profile_and_destination_are_preserved(self):
        destination = self.root / "_build/custom app"
        self.assertEqual(self.run_build("release", str(self.config), str(destination)), 0, self.output)
        arguments = self.calls("build")[0][1]
        self.assertEqual(arguments[0], str(self.config))
        self.assertEqual(arguments[1], str(destination / "thumbnailers"))
        self.assertEqual(arguments[-1], "release")
        self.assertFalse(any("run" in call[1] for call in self.calls("cargo")))

    def test_sdk_cache_is_reused_until_source_changes(self):
        self.assertEqual(self.run_build(), 0, self.output)
        self.assertEqual(self.run_build(), 0, self.output)
        self.assertEqual(len([call for call in self.calls("cargo") if "run" in call[1]]), 1)
        (self.root / "thumbnail-api/source").write_text("new SDK\n")
        self.assertEqual(self.run_build(), 0, self.output)
        self.assertEqual(len([call for call in self.calls("cargo") if "run" in call[1]]), 2)

    def test_offline_applies_to_fetches_ffmpeg_and_image_native_build(self):
        self.environment["MIDNIGHT_THUMBNAILERS_OFFLINE"] = "1"
        self.assertEqual(self.run_build("debug", str(self.config)), 0, self.output)
        self.assertTrue(all("--offline" in call[1] for call in self.calls("cargo")))
        self.assertIn("--offline", self.calls("ffmpeg")[0][1])
        self.assertEqual(self.calls("build")[0][2], "1")

    def test_target_directory_selects_application_destination(self):
        self.environment["CARGO_TARGET_DIR"] = str(self.root / "_build/custom target")
        self.assertEqual(self.run_build("debug", str(self.config)), 0, self.output)
        self.assertEqual(self.calls("build")[0][1][1], str(self.root / "_build/custom target/debug/thumbnailers"))

    def test_ffmpeg_failure_prevents_any_provider_build(self):
        self.environment["THUMBNAIL_TEST_FFMPEG_FAILURE"] = "1"
        self.assertEqual(self.run_build("debug", str(self.config)), 41, self.output)
        self.assertEqual(self.calls("build"), [])

    def test_notice_failure_prevents_any_provider_build(self):
        self.environment["THUMBNAIL_TEST_NOTICES_FAILURE"] = "1"
        self.assertEqual(self.run_build("debug", str(self.config)), 42, self.output)
        self.assertEqual(self.calls("build"), [])

    def test_incomplete_native_configuration_is_rejected(self):
        self.environment["THUMBNAIL_TEST_MISSING_CLANG"] = "1"
        self.assertNotEqual(self.run_build("debug", str(self.config)), 0, self.output)
        self.assertEqual(self.calls("build"), [])

    def test_provider_build_failure_is_returned(self):
        self.environment["THUMBNAIL_TEST_BUILD_FAILURE"] = "1"
        self.assertEqual(self.run_build("debug", str(self.config)), 43, self.output)

    def test_invalid_profile_does_not_prepare_anything(self):
        self.assertEqual(self.run_build("unknown"), 2, self.output)
        self.assertEqual(self.calls("cargo"), [])
        self.assertEqual(self.calls("ffmpeg"), [])


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux thumbnail build inputs")
class ThumbnailerInputTests(unittest.TestCase):
    def test_low_level_build_rejects_missing_ffmpeg_before_native_bootstrap(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary) as directory:
            fixture = Path(directory)
            config = fixture / "registry.toml"
            config.write_text("# fixture\n")
            (fixture / "scripts").mkdir()
            shutil.copyfile(ROOT / "scripts/build-thumbnailers.sh", fixture / "scripts/build-thumbnailers.sh")
            native = fixture / "plugins/image-decoder/native"
            native.mkdir(parents=True)
            (native / "bootstrap-linux.sh").write_text("return 97\n")
            result = subprocess.run(["bash", str(fixture / "scripts/build-thumbnailers.sh"), str(config),
                                     str(fixture / "destination"), str(fixture / "targets"), str(fixture)],
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("FFMPEG", result.stderr)
            self.assertFalse((fixture / "destination").exists())


if __name__ == "__main__":
    unittest.main()
