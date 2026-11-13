"""Normal viewer builds preserve Debug through every mandatory plugin boundary."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform == "linux", "Linux viewer build helpers")
class ViewerBuildProfiles(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="viewer profiles ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "calls.jsonl"
        self.config = self.root / "SDK config.toml"
        self.config.write_text("# fixture\n")
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                                VIEWER_TEST_LOG=str(self.log))
        for name in ("MIDNIGHT_TEST_VIEWERS", "MIDNIGHT_VIEWERS_OFFLINE", "CARGO_TARGET_DIR"):
            self.environment.pop(name, None)
        self.executable(self.bin / "rustc", "print('host: x86_64-unknown-linux-gnu')\n")
        self.executable(self.bin / "cargo", """
with open(os.environ['VIEWER_TEST_LOG'], 'a') as log:
    log.write(json.dumps(['cargo', sys.argv[1:]]) + '\\n')
if 'build' in sys.argv:
    manifest = pathlib.Path(sys.argv[sys.argv.index('--manifest-path') + 1])
    name = manifest.parent.name.removeprefix('midnight-viewer-')
    target = pathlib.Path(sys.argv[sys.argv.index('--target-dir') + 1])
    platform = sys.argv[sys.argv.index('--target') + 1]
    profile = 'release' if '--release' in sys.argv else 'debug'
    output = target / platform / profile
    output.mkdir(parents=True, exist_ok=True)
    (output / f'libmidnight_viewer_{name}.so').write_bytes(profile.encode())
""")

    def executable(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"#!{sys.executable}\nimport json, os, pathlib, sys\n" + body)
        path.chmod(0o755)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_legacy_plugin_default_and_explicit_profiles_select_matching_artifacts(self):
        for name in ("basic", "source", "markdown"):
            for profile in (None, "debug", "release"):
                with self.subTest(plugin=name, profile=profile):
                    project = self.root / f"{name}-{profile}" / "plugins" / f"midnight-viewer-{name}"
                    scripts = project / "scripts"
                    scripts.mkdir(parents=True)
                    shutil.copyfile(ROOT / "plugins" / f"midnight-viewer-{name}/scripts/build.sh", scripts / "build.sh")
                    (scripts / "prepare-sdk.py").write_text("import sys\nprint(sys.argv[1])\n")
                    for filename in ("Cargo.toml", "LICENSE", "UNICODE-LICENSE.txt", "README.md", "THIRD-PARTY.md"):
                        (project / filename).write_text("Pinned fixture notice\n")
                    (project / "plugin.toml").write_text('entry_library = "fixture"\ntarget = "fixture"\n')
                    (project / "notices").mkdir()
                    (project / "notices/LICENSE").write_text("Third-party terms\n")
                    build = project.parents[1] / "_build/native"
                    package = project.parents[1] / "_build/package"
                    arguments = [str(self.config), str(build), str(package)]
                    if profile:
                        arguments.append(profile)
                    result = subprocess.run(["bash", str(scripts / "build.sh"), *arguments],
                                            env=self.environment, text=True, capture_output=True, timeout=20)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertEqual((package / f"libmidnight_viewer_{name}.so").read_bytes(),
                                     (profile or "debug").encode())
                    self.assertEqual((package / "notices/LICENSE").read_text(), "Third-party terms\n")

    def wrapper(self):
        (self.root / "scripts").mkdir()
        shutil.copyfile(ROOT / "scripts/build-viewers.sh", self.root / "scripts/build-viewers.sh")
        logger = self.root / "log.py"
        logger.write_text("import json, os, sys\n"
                          "with open(os.environ['VIEWER_TEST_LOG'], 'a') as log:\n"
                          "    log.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                          "if sys.argv[1] == 'html' and os.environ.get('VIEWER_TEST_HTML_FAILURE'):\n"
                          "    raise SystemExit(51)\n")
        for name in ("basic", "source", "markdown"):
            project = self.root / "plugins" / f"midnight-viewer-{name}"
            (project / "scripts").mkdir(parents=True)
            (project / "Cargo.toml").write_text("# fixture\n")
            (project / "scripts/build.sh").write_text(
                f'#!/bin/bash\nexec python3 "{logger}" {name} "$@"\n')
        for name, arguments in (("image-viewer", '"$4" "$1"'), ("html-viewer", 'html "$1"')):
            (self.root / f"scripts/build-{name}.sh").write_text(
                f'#!/bin/bash\nexec python3 "{logger}" {arguments}\n')

    def test_normal_build_forwards_debug_and_requires_html_after_other_viewers(self):
        self.wrapper()
        result = subprocess.run(["bash", str(self.root / "scripts/build-viewers.sh"),
                                 "debug", str(self.config)], env=self.environment,
                                text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        builds = [row for row in self.calls() if row[0] != "cargo"]
        self.assertEqual([row[0] for row in builds], ["basic", "source", "markdown", "image", "pdf", "html"])
        self.assertTrue(all(row[-1] == "debug" for row in builds[:3]))
        self.assertTrue(all(row[1] == "debug" for row in builds[3:]))

    def test_missing_html_does_not_report_successful_normal_viewer_build(self):
        self.wrapper()
        self.environment["VIEWER_TEST_HTML_FAILURE"] = "1"
        result = subprocess.run(["bash", str(self.root / "scripts/build-viewers.sh"),
                                 "debug", str(self.config)], env=self.environment,
                                text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 51, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
