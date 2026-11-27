"""Plugin Linux build and CI scripts write below _build, reached however a checkout is spelled.

The plugin and the repository that holds it in plugins/ count as checkouts: their own
_build, or outside them a custom directory's first _build, holds generated output, and
that _build and every folder below it must not be a symlink; the checkout and the
folders above it may be links. Image and PDF builds and the image, PDF and thumbnail CI
scripts refuse other build roots before Cargo runs, the archive worker refuses other
build roots and package folders, and Basic, Markdown and Source builds may install
anywhere but replace notices without their marker only below such a _build.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
# Every Linux plugin script that carries the walk, with its plugin.
WALKERS = (("midnight-viewer-image", "build.sh"), ("midnight-viewer-image", "ci.sh"),
           ("midnight-viewer-pdf", "build.sh"), ("midnight-viewer-pdf", "ci.sh"),
           ("midnight-thumbnail-image", "ci.sh"), ("midnight-viewer-basic", "build.sh"),
           ("midnight-viewer-markdown", "build.sh"), ("midnight-viewer-source", "build.sh"),
           ("midnight-fs-archive", "build.sh"))
CARGO = r'''
arguments = sys.argv[1:]
with open(os.environ['PLUGIN_TEST_LOG'], 'a') as log:
    log.write(json.dumps(arguments) + '\n')
if 'build' in arguments:
    plugin = pathlib.Path(arguments[arguments.index('--manifest-path') + 1]).parent.name
    profile = 'release' if '--release' in arguments else 'debug'
    if '--target-dir' in arguments:
        output = pathlib.Path(arguments[arguments.index('--target-dir') + 1])
    else:
        output = pathlib.Path(os.environ['CARGO_TARGET_DIR'])
    if '--target' in arguments:
        output = output / arguments[arguments.index('--target') + 1]
    output = output / profile
    output.mkdir(parents=True, exist_ok=True)
    name = plugin.removeprefix('midnight-viewer-')
    for artifact in (plugin, f'libmidnight_viewer_{name}.so'):
        (output / artifact).write_bytes(b'built')
'''


def generated_output_function(text):
    match = re.search(r"^generated_output_problem\(\) \{\n.*?^\}\n", text, re.MULTILINE | re.DOTALL)
    return match.group(0) if match else None


class WalkCopies(unittest.TestCase):
    def test_every_plugin_script_carries_the_same_walk(self):
        # The plugins are independent projects, so each script has its own copy.
        copies = {}
        for plugin, script in WALKERS:
            text = (ROOT / "plugins" / plugin / "scripts" / script).read_text(encoding="utf-8")
            copies[plugin, script] = generated_output_function(text)
            with self.subTest(plugin=plugin, script=script):
                self.assertIsNotNone(copies[plugin, script], "The walk is missing")
                self.assertNotIn("realpath -m --", text, "Arguments are resolved before the walk")
        self.assertEqual(len(set(copies.values())), 1, "The copies of the walk differ")
        for plugin in ("midnight-viewer-basic", "midnight-viewer-markdown", "midnight-viewer-source"):
            with self.subTest(plugin=plugin):
                text = (ROOT / "plugins" / plugin / "scripts/ci.sh").read_text(encoding="utf-8")
                self.assertIn('build_root="$(realpath -ms -- "$2")"', text)


def outside_build_temporary(test):
    """A temporary directory with no _build component (the tests point TMPDIR into _build)."""
    for candidate in (os.environ.get("RUNNER_TEMP"), "/tmp"):
        if candidate and os.path.isdir(candidate) and "_build" not in Path(candidate).resolve().parts:
            directory = tempfile.TemporaryDirectory(prefix="plugin outputs ", dir=candidate)
            test.addCleanup(directory.cleanup)
            return Path(directory.name)
    raise unittest.SkipTest("No temporary directory outside _build")


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux plugin build scripts")
class PluginLinuxBuildOutputs(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="plugin outputs ", dir=temporary)
        self.addCleanup(owner.cleanup)
        self.owner = Path(owner.name)
        # A repository reached through a linked parent folder, as /home -> /var/home.
        self.real = self.owner / "var-home/checkout"
        for plugin, script in WALKERS:
            scripts = self.real / "plugins" / plugin / "scripts"
            scripts.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / "plugins" / plugin / "scripts" / script, scripts / script)
            for name in ("Cargo.toml", "LICENSE", "UNICODE-LICENSE.txt", "README.md", "THIRD-PARTY.md"):
                (scripts.parent / name).write_text(f"{plugin} {name}\n")
            (scripts.parent / "plugin.toml").write_text('entry_library = "template"\ntarget = "template"\n')
            (scripts.parent / "image-viewer.json").write_text('{"target": "template", "executable": "template"}\n')
            (scripts.parent / "notices").mkdir(exist_ok=True)
            (scripts.parent / "notices/dependency-LICENSE").write_text(f"{plugin} dependency notice\n")
            (scripts.parent / "notices/UPSTREAM_NOTICE.md").write_text(f"{plugin} upstream notice\n")
            (scripts / "prepare-sdk.py").write_text("import sys\nprint(sys.argv[1])\n")
            (scripts / "notices.py").write_text(
                "import pathlib, sys\n"
                "output = pathlib.Path(sys.argv[sys.argv.index('--output') + 1])\n"
                "output.mkdir(parents=True, exist_ok=True)\n"
                "(output / 'LICENSE').write_text('archive dependency license\\n')\n")
        native = self.real / "plugins/image-decoder/native"
        native.mkdir(parents=True)
        for helper in ("bootstrap-linux.sh", "build-linux.sh"):
            (native / helper).write_text(f'printf "%s\\n" "{helper} $1" >> "$NATIVE_TEST_LOG"\n')
        (self.real / "plugins/image-decoder/notices").mkdir()
        (self.real / "plugins/image-decoder/notices/decoder-LICENSE").write_text("decoder notice\n")
        (self.real / "plugins/image-decoder/notices/UPSTREAM_NOTICE.md").write_text("decoder upstream notice\n")
        self.checkout = self.owner / "home"
        self.checkout.symlink_to(self.real.parent, target_is_directory=True)
        self.checkout = self.checkout / "checkout"
        self.outside = self.owner / "outside"
        self.outside.mkdir()
        (self.real / "_build").mkdir()
        (self.real / "_build/redirect").symlink_to(self.outside, target_is_directory=True)
        (self.real / "_build/loop").symlink_to(self.real, target_is_directory=True)
        self.bin = self.owner / "bin"
        self.bin.mkdir()
        self.log = self.owner / "cargo.jsonl"
        for name, body in (("rustc", "print('host: x86_64-unknown-linux-gnu')\n"), ("cargo", CARGO)):
            (self.bin / name).write_text(f"#!{sys.executable}\nimport json, os, pathlib, sys\n" + body)
            (self.bin / name).chmod(0o755)
        self.sdk = self.owner / "sdk/config.toml"
        self.sdk.parent.mkdir()
        self.sdk.write_text("# fixture\n")
        self.environment = dict(os.environ, PATH=f"{self.bin}{os.pathsep}{os.environ['PATH']}",
                                PLUGIN_TEST_LOG=str(self.log), NATIVE_TEST_LOG=str(self.owner / "native.log"))
        for name in ("CARGO_TARGET_DIR", "MIDNIGHT_IMAGE_NATIVE_OFFLINE", "MIDNIGHT_TEST_ARCHIVE_FILESYSTEM"):
            self.environment.pop(name, None)

    def assert_lowercase_markdown(self, package):
        """Markdown names in a package are lowercase, with '-' for '_' (README.md is readme.md)."""
        names = {path.relative_to(package).as_posix() for path in package.rglob("*.md")}
        self.assertEqual(names, {"readme.md", "third-party.md", "notices/upstream-notice.md"})

    def run_script(self, plugin, script, *arguments):
        if self.log.exists():
            self.log.unlink()
        result = subprocess.run(["bash", str(self.checkout / "plugins" / plugin / "scripts" / script),
                                 str(self.sdk), *map(str, arguments)],
                                env=self.environment, text=True, capture_output=True, timeout=60)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, calls

    def test_image_and_pdf_builds_accept_linked_checkouts_but_not_links_from_build_down(self):
        builds = self.checkout / "_build"
        for name in ("image", "pdf"):
            plugin = f"midnight-viewer-{name}"
            for build_root, output, outcome in (
                    # The shared repository's _build, and the plugin's own as a standalone project.
                    (builds / f"viewers/{name}", builds / f"debug/viewers/{plugin}", None),
                    (self.checkout / "plugins" / plugin / "_build/build", None, None),
                    # Outside the checkout the walk starts at the path's first _build, which for
                    # this fixture is the test checkout's own _build above it.
                    (self.owner / f"custom/_build/{name}", None, None),
                    (builds / f"redirect/{name}", None, "must not traverse symlinks"),
                    (builds / f"viewers/{name}", self.real / f"_build/loop/_build/viewers/{name}",
                     "must not traverse symlinks"),
                    (self.checkout / f"built-{name}", None, "must be under _build"),
                    # The checkout spelled without the link that the script was started
                    # through is still the checkout, not a custom directory below _build.
                    (self.real / f"built-{name}", None, "must be under _build"),
                    (self.checkout / "plugins" / plugin / f"target/{name}", None, "must be under _build"),
                    (builds / f"viewers/{name}", self.checkout / f"viewers/{name}", "must be under _build")):
                with self.subTest(plugin=plugin, build_root=build_root, output=output):
                    result, calls = self.run_script(plugin, "build.sh", build_root,
                                                    *([output] if output else []))
                    if outcome is None:
                        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                        package = output or build_root / f"package/{plugin}"
                        self.assertEqual((package / plugin).read_bytes(), b"built")
                        self.assertTrue((package / "notices").is_dir())
                        self.assert_lowercase_markdown(package)
                    else:
                        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                        self.assertIn(f"Generated output {outcome}", result.stderr)
                        self.assertEqual(calls, [], "Cargo ran before the output was checked")
        self.assertEqual(list(self.outside.iterdir()), [])
        self.assertFalse(any(self.real.glob("built-*")) or (self.real / "viewers").exists())

    def test_image_pdf_and_thumbnail_ci_refuse_links_from_build_down_before_cargo(self):
        builds = self.checkout / "_build"
        for plugin in ("midnight-viewer-image", "midnight-viewer-pdf", "midnight-thumbnail-image"):
            for build_root, outcome in ((builds / "ci", None),
                                        (builds / "redirect/ci", "must not traverse symlinks"),
                                        (self.real / "_build/loop/_build/ci", "must not traverse symlinks"),
                                        (self.checkout / "ci", "must be under _build")):
                with self.subTest(plugin=plugin, build_root=build_root):
                    result, calls = self.run_script(plugin, "ci.sh", build_root)
                    if outcome is None:
                        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertIn("test", [call[0] if call[0] != "--config" else call[2] for call in calls])
                    else:
                        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                        self.assertIn(f"Generated output {outcome}", result.stderr)
                        self.assertEqual(calls, [], "Cargo ran before the build root was checked")
        self.assertEqual(list(self.outside.iterdir()), [])

    def test_unmarked_notices_are_replaced_only_below_build(self):
        outside = outside_build_temporary(self)
        builds = self.checkout / "_build"
        for name in ("basic", "markdown", "source"):
            plugin = f"midnight-viewer-{name}"
            for output, replaced in ((builds / f"debug/viewers/{plugin}", True),
                                     (builds / f"redirect/{name}", False),
                                     (self.checkout / f"installed/{name}", False),
                                     (outside / f"app/viewers/{plugin}", False)):
                with self.subTest(plugin=plugin, output=output):
                    # Notices without this script's marker: an older build's or someone else's.
                    (output / "notices").mkdir(parents=True)
                    (output / "notices/unmarked-LICENSE").write_text("not written by this script\n")
                    result, _ = self.run_script(plugin, "build.sh", builds / f"viewers/{name}", output)
                    if replaced:
                        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertEqual(sorted(path.name for path in (output / "notices").iterdir()),
                                         [".midnight-generated", "dependency-LICENSE", "upstream-notice.md"])
                        self.assert_lowercase_markdown(output)
                    else:
                        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                        self.assertIn("Refusing to replace notices", result.stderr)
                        self.assertEqual((output / "notices/unmarked-LICENSE").read_text(),
                                         "not written by this script\n")

    def test_the_walk_refuses_a_relative_path_instead_of_looping(self):
        text = (ROOT / "plugins/midnight-viewer-image/scripts/build.sh").read_text(encoding="utf-8")
        script = generated_output_function(text) + 'plugin_root=$1\ngenerated_output_problem "$2"\n'
        result = subprocess.run(["bash", "-c", script, "walk", str(self.real / "plugins/midnight-viewer-image"),
                                 "_build/relative"], text=True, capture_output=True, timeout=10)
        self.assertEqual(result.stdout, "must be an absolute path: _build/relative\n", result.stderr)

    def test_archive_worker_build_root_and_package_follow_the_walk(self):
        builds = self.checkout / "_build"
        plugin = "midnight-fs-archive"
        for build_root, package, outcome in (
                (builds / "archive", builds / "debug/filesystems/midnight.fs.archive", None),
                (self.checkout / "plugins" / plugin / "_build/archive", None, None),
                (builds / "redirect/archive", None, "Build output must not traverse symlinks"),
                (self.checkout / "archive", None, "Build output must be under _build"),
                (builds / "archive", self.real / "_build/loop/_build/package",
                 "Package staging must not traverse symlinks"),
                (builds / "archive", self.checkout / "package", "Package staging must be under _build")):
            with self.subTest(build_root=build_root, package=package):
                result, calls = self.run_script(plugin, "build.sh", build_root, *([package] if package else []))
                if outcome is None:
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    if package:
                        self.assertEqual((package / "midnight-fs-archive").read_bytes(), b"built")
                else:
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertIn(outcome, result.stderr)
                    if outcome.startswith("Build output"):
                        self.assertEqual(calls, [], "Cargo ran before the build root was checked")
        self.assertEqual(list(self.outside.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
