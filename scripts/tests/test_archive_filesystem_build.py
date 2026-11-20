"""Normal archive builds install the Debug worker using the matching public SDK."""
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
PLUGIN = ROOT / "plugins/midnight-fs-archive"


def plugin_version():
    with (PLUGIN / "Cargo.toml").open("rb") as stream:
        return tomllib.load(stream)["package"]["version"]


class ArchiveManifestVersionTests(unittest.TestCase):
    """The installed manifest names the package version that assigned the member IDs."""

    def test_both_build_scripts_install_the_plugin_package_version(self):
        # The host keys persistent archive thumbnails by provider ID and manifest
        # version, so a release whose member IDs changed must not install the
        # previous version (an old cache entry could then show another member).
        expected = plugin_version()
        for script in ("build.ps1", "build.sh"):
            with self.subTest(script=script):
                text = (PLUGIN / "scripts" / script).read_text(encoding="utf-8")
                manifest = text[text.index("schema_version = 1"):]
                self.assertIn('id = "midnight.fs.archive"', manifest)
                self.assertEqual(re.findall(r'^version = "([^"]*)"$', manifest, re.MULTILINE)[:1],
                                 [expected])


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux archive build helpers")
class ArchiveFilesystemBuild(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="archive build ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "checkout & dollar$"
        (self.root / "scripts").mkdir(parents=True)
        self.wrapper = self.root / "scripts/build-archive-filesystem.sh"
        wrapper = ROOT / "scripts/build-archive-filesystem.sh"
        if wrapper.exists():
            shutil.copyfile(wrapper, self.wrapper)
        self.plugin = self.root / "plugins/midnight-fs-archive"
        (self.plugin / "scripts").mkdir(parents=True)
        shutil.copyfile(ROOT / "plugins/midnight-fs-archive/scripts/build.sh", self.plugin / "scripts/build.sh")
        (self.plugin / "Cargo.toml").write_text('[package]\nname="midnight-fs-archive"\n')
        (self.plugin / "THIRD-PARTY.md").write_text("Pinned archive dependencies\n")
        (self.plugin / "scripts/notices.py").write_text(
            "import json, os, pathlib, sys\n"
            "with open(os.environ['ARCHIVE_TEST_LOG'], 'a') as log:\n"
            "    log.write(json.dumps(['notices', sys.argv[1:]]) + '\\n')\n"
            "if os.environ.get('ARCHIVE_TEST_NOTICE_FAILURE'): raise SystemExit(45)\n"
            "output = pathlib.Path(sys.argv[sys.argv.index('--output') + 1])\n"
            "output.mkdir(parents=True, exist_ok=True)\n"
            "(output / 'LICENSE').write_text('Exact archive dependency license\\n')\n")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "calls.jsonl"
        self.program("cargo", """
with open(os.environ['ARCHIVE_TEST_LOG'], 'a') as log:
    log.write(json.dumps(['cargo', sys.argv[1:]]) + '\\n')
failure = os.environ.get('ARCHIVE_TEST_CARGO_FAILURE')
if failure and (failure == '1' or failure == sys.argv[1]):
    raise SystemExit(43)
if sys.argv[1] == 'build':
    profile = 'release' if '--release' in sys.argv else 'debug'
    output = pathlib.Path(os.environ['CARGO_TARGET_DIR']) / profile
    output.mkdir(parents=True, exist_ok=True)
    (output / 'midnight-fs-archive').write_bytes(profile.encode())
""")
        self.program("rustc", "print('host: x86_64-unknown-linux-gnu')\n")
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                                ARCHIVE_TEST_LOG=str(self.log))
        for name in ("CARGO_TARGET_DIR", "MIDNIGHT_ARCHIVE_SDK_CONFIG", "MIDNIGHT_ARCHIVE_OFFLINE",
                     "MIDNIGHT_VIEWERS_OFFLINE", "MIDNIGHT_TEST_ARCHIVE_FILESYSTEM"):
            self.environment.pop(name, None)
        self.config = self.root / "worker SDK/registry.toml"
        self.config.parent.mkdir()
        self.config.write_text("# Matching public filesystem API bundle\n")
        self.application = self.root / "_build/debug"
        for name in ("api/tools/package", "worker-api", "thumbnail-api", "filesystem-api", "image-viewer-api"):
            directory = self.root / name
            directory.mkdir(parents=True)
            (directory / "source").write_text("Public source identity\n")

    def program(self, name, body):
        path = self.bin / name
        path.write_text(f"#!{sys.executable}\nimport json, os, pathlib, sys\n" + body)
        path.chmod(0o755)

    def calls(self, kind):
        rows = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return [row[1] for row in rows if row[0] == kind]

    def run_wrapper(self, *arguments):
        return subprocess.run(["bash", str(self.wrapper), *arguments], cwd=self.root.parent,
                              env=self.environment, text=True, capture_output=True, timeout=20)

    def sdk_cache(self, name="thumbnailers"):
        # Use the same external hashing boundary as the source-stamped worker SDK caches.
        result = subprocess.run(
            ["bash", "-c", "find api/tools/package worker-api thumbnail-api filesystem-api image-viewer-api -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1"],
            cwd=self.root, text=True, capture_output=True, check=True)
        cache = self.root / "_build" / name
        cache.mkdir(parents=True)
        (cache / "linux-sdk-stamp").write_text(result.stdout)
        (cache / "linux-sdk-config").write_text(str(self.config) + "\n")

    def test_normal_wrapper_uses_cached_sdk_and_installs_debug_without_running_plugin_tests(self):
        self.sdk_cache()
        result = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        commands = self.calls("cargo")
        self.assertEqual([command[0] for command in commands], ["--config", "build"])
        self.assertIn("fetch", commands[0])
        self.assertTrue(all("--locked" in command for command in commands))
        self.assertNotIn("--release", commands[1])
        package = self.application / "filesystems/midnight.fs.archive"
        self.assertEqual((package / "midnight-fs-archive").read_bytes(), b"debug")
        manifest = (package / "plugin.toml").read_text()
        self.assertIn('protocol = 3', manifest)
        self.assertIn(f'version = "{plugin_version()}"', manifest)
        self.assertEqual((package / "licenses/LICENSE").read_text(), "Exact archive dependency license\n")
        self.assertEqual((package / "THIRD-PARTY.md").read_text(), "Pinned archive dependencies\n")

    def test_explicit_profile_sdk_destination_and_offline_are_forwarded(self):
        self.environment["MIDNIGHT_ARCHIVE_OFFLINE"] = "1"
        self.environment["MIDNIGHT_TEST_ARCHIVE_FILESYSTEM"] = "1"
        destination = self.root / "_build/custom app"
        result = self.run_wrapper("release", str(self.config), str(destination))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        commands = self.calls("cargo")
        self.assertEqual([command[0] for command in commands], ["--config", "test", "build"])
        self.assertTrue(all("--offline" in command and "--locked" in command for command in commands))
        # Parser tests keep debug assertions and overflow checks; only the package is optimized.
        self.assertNotIn("--release", commands[1])
        self.assertIn("--release", commands[2])
        self.assertEqual((destination / "filesystems/midnight.fs.archive/midnight-fs-archive").read_bytes(), b"release")

    def test_image_viewer_cache_is_reused_but_stale_sdk_is_rejected_before_cargo(self):
        self.sdk_cache("image-viewer")
        self.assertEqual(self.run_wrapper().returncode, 0)
        self.log.unlink()
        (self.root / "thumbnail-api/source").write_text("Updated public ABI\n")
        result = self.run_wrapper()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("MIDNIGHT_ARCHIVE_SDK_CONFIG", result.stderr)
        self.assertEqual(self.calls("cargo"), [])

    def test_missing_sdk_and_invalid_profile_do_not_start_a_build(self):
        for arguments in ((), ("unknown",)):
            with self.subTest(arguments=arguments):
                result = self.run_wrapper(*arguments)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("MIDNIGHT_ARCHIVE_SDK_CONFIG" if not arguments else "Expected debug or release",
                              result.stderr)
                self.assertEqual(self.calls("cargo"), [])

    def test_fetch_failure_skips_build_and_staging(self):
        self.environment["ARCHIVE_TEST_CARGO_FAILURE"] = "1"
        result = self.run_wrapper("debug", str(self.config))
        self.assertEqual(result.returncode, 43, result.stdout + result.stderr)
        self.assertEqual(len(self.calls("cargo")), 1)
        self.assertFalse(self.application.exists())

    def test_three_argument_plugin_build_defaults_to_debug_and_keeps_test_opt_in(self):
        package = self.root / "_build/standalone package"
        result = subprocess.run(
            ["bash", str(self.plugin / "scripts/build.sh"), str(self.config),
             str(self.root / "_build/plugin target"), str(package)],
            cwd=self.root, env=self.environment, text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([command[0] for command in self.calls("cargo")], ["build"])
        self.assertEqual((package / "midnight-fs-archive").read_bytes(), b"debug")

    def test_opted_in_test_failure_skips_build_and_notices(self):
        self.environment["MIDNIGHT_TEST_ARCHIVE_FILESYSTEM"] = "1"
        self.environment["ARCHIVE_TEST_CARGO_FAILURE"] = "test"
        result = self.run_wrapper("debug", str(self.config))
        self.assertEqual(result.returncode, 43, result.stdout + result.stderr)
        self.assertEqual([command[0] for command in self.calls("cargo")], ["--config", "test"])
        self.assertEqual(self.calls("notices"), [])
        self.assertFalse(self.application.exists())

    def test_missing_distribution_notices_make_the_normal_build_fail(self):
        self.environment["ARCHIVE_TEST_NOTICE_FAILURE"] = "1"
        result = self.run_wrapper("debug", str(self.config))
        self.assertEqual(result.returncode, 45, result.stdout + result.stderr)
        self.assertEqual(len(self.calls("notices")), 1)


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux Cargo fixture executable")
class ArchiveNoticeTests(unittest.TestCase):
    """Regenerated archive notices never retain a dependency that is no longer shipped."""

    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="archive notices ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        cargo = self.bin / "cargo"
        cargo.write_text(f"#!{sys.executable}\nimport os, sys\n"
                         "sys.stdout.write(open(os.environ['ARCHIVE_TEST_METADATA']).read())\n")
        cargo.chmod(0o755)
        self.metadata = self.root / "metadata.json"
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                                ARCHIVE_TEST_METADATA=str(self.metadata))
        self.output = self.root / "_build/package/licenses"

    def dependency(self, name):
        source = self.root / "registry" / f"{name}-1.0.0"
        source.mkdir(parents=True, exist_ok=True)
        (source / "LICENSE-MIT").write_text(f"{name} terms\n")
        (source / "Cargo.toml").write_text("[package]\n")
        package = {"name": name, "version": "1.0.0", "license": "MIT",
                   "source": "registry+https://github.com/rust-lang/crates.io-index",
                   "manifest_path": str(source / "Cargo.toml")}
        if name == "broken":
            package["license_file"] = "missing/LICENSE"
        return package

    def generate(self, *names):
        self.metadata.write_text(json.dumps({"packages": [self.dependency(name) for name in names]}))
        return subprocess.run([sys.executable, "-B", str(ROOT / "plugins/midnight-fs-archive/scripts/notices.py"),
                               "--config", str(self.metadata), "--output", str(self.output)],
                              env=self.environment, text=True, capture_output=True, timeout=20)

    def assert_only_kept(self):
        self.assertEqual(sorted(path.name for path in self.output.iterdir()), ["inventory.json", "kept-1.0.0"])
        self.assertEqual([entry["name"] for entry in json.loads((self.output / "inventory.json").read_text())],
                         ["kept"])

    def test_regeneration_removes_notices_of_dropped_dependencies(self):
        self.assertEqual(self.generate("kept", "dropped").returncode, 0)
        self.assertTrue((self.output / "dropped-1.0.0/LICENSE-MIT").is_file())
        result = self.generate("kept")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_only_kept()

    def test_failed_regeneration_never_blocks_the_next_build(self):
        # The previous script removed its output, copied part of the new notices and
        # wrote the inventory last, so a failure left a directory it then refused.
        self.assertEqual(self.generate("kept", "dropped").returncode, 0)
        self.assertNotEqual(self.generate("kept", "broken").returncode, 0)
        result = self.generate("kept")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_only_kept()

    def test_directory_without_generated_inventory_is_never_replaced(self):
        self.output.mkdir(parents=True)
        (self.output / "user file").write_text("not generated\n")
        result = self.generate("kept")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.output / "user file").read_text(), "not generated\n")
        self.assertFalse((self.output / "kept-1.0.0").exists())

    def test_linked_output_is_refused_without_following_it(self):
        # Resolving first followed the link and replaced its target.
        target = self.root / "_build/elsewhere/licenses"
        target.mkdir(parents=True)
        (target / "inventory.json").write_text("[]\n")
        (target / "user notice").write_text("not this package\n")
        self.output.parent.mkdir(parents=True)
        self.output.symlink_to(target, target_is_directory=True)
        result = self.generate("kept")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("link", result.stderr)
        self.assertEqual(sorted(path.name for path in target.iterdir()), ["inventory.json", "user notice"])


class ArchiveNoticeReplacementTests(unittest.TestCase):
    """The inventory marker survives interruptions and links are never followed."""

    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "archive_notices", ROOT / "plugins/midnight-fs-archive/scripts/notices.py")
        self.notices = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.notices)
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="archive notice replacement ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.notices.checked_output(self.root / "_build/package/licenses")

    @staticmethod
    def plan(*names):
        return ({f"{name}-1.0.0": {"LICENSE": f"{name} terms\n".encode()} for name in names},
                [{"name": name, "version": "1.0.0", "license": "MIT"} for name in names])

    @staticmethod
    def link(link, target):
        if os.name == "nt":
            import _winapi  # A junction needs no symlink privilege.
            _winapi.CreateJunction(str(target), str(link))
        else:
            link.symlink_to(target, target_is_directory=True)

    @staticmethod
    def unlink(link):
        if os.name == "nt":
            os.rmdir(link)  # Removes the junction itself, never its target.
        else:
            link.unlink()

    def test_interrupted_regeneration_is_completed_by_the_next_build(self):
        self.notices.replace_notices(self.output, *self.plan("kept", "dropped"))
        original = Path.write_bytes
        writes = []

        def interrupted(path, content):
            writes.append(path)
            if len(writes) == 2:
                raise OSError("simulated interruption")
            return original(path, content)

        with mock.patch.object(Path, "write_bytes", interrupted):
            with self.assertRaises(OSError):
                self.notices.replace_notices(self.output, *self.plan("kept", "added"))
        self.assertTrue((self.output / "inventory.json").is_file())
        self.notices.replace_notices(self.output, *self.plan("kept", "added"))
        self.assertEqual(sorted(path.name for path in self.output.iterdir()),
                         ["added-1.0.0", "inventory.json", "kept-1.0.0"])

    def test_links_at_or_inside_the_output_are_never_followed(self):
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "keep").write_text("user data\n")
        self.notices.replace_notices(self.output, *self.plan("kept"))
        link = self.output / "kept-1.0.0/redirect"
        self.link(link, outside)
        with self.assertRaisesRegex(SystemExit, "link"):
            self.notices.replace_notices(self.output, *self.plan("kept"))
        self.assertEqual((outside / "keep").read_text(), "user data\n")
        self.unlink(link)
        moved = self.root / "_build/package/moved"
        self.output.rename(moved)
        self.link(self.output, moved)
        self.addCleanup(self.unlink, self.output)
        for check in (lambda: self.notices.checked_output(self.output),
                      lambda: self.notices.replace_notices(self.output, *self.plan("kept"))):
            with self.assertRaisesRegex(SystemExit, "link"):
                check()
        self.assertEqual(sorted(path.name for path in moved.iterdir()), ["inventory.json", "kept-1.0.0"])


class ArchiveNoticeInputTests(unittest.TestCase):
    """Folders above a dependency's root may be links; nothing below it is followed."""

    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "archive_notices", ROOT / "plugins/midnight-fs-archive/scripts/notices.py")
        self.notices = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.notices)
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="archive notice inputs ", dir=temporary)
        self.addCleanup(owner.cleanup)
        self.root = Path(owner.name)
        self.outside = self.root / "outside"
        self.outside.mkdir()
        (self.outside / "LICENSE").write_text("Unrelated private content\n")

    def link(self, kind, link, target):
        target.mkdir(parents=True, exist_ok=True)
        link.parent.mkdir(parents=True, exist_ok=True)
        if kind == "junction":
            import _winapi  # A junction needs no symlink privilege.
            _winapi.CreateJunction(str(target), str(link))
        else:
            try:
                link.symlink_to(target, target_is_directory=True)
            except OSError as error:
                self.skipTest(f"OS does not permit fixture symlinks: {error}")
        return link

    def crate(self, directory, name="zip", files=None, license_file=None):
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "Cargo.toml").write_text("[package]\n")
        for relative, content in (files or {"LICENSE-MIT": f"{name} terms\n"}).items():
            (directory / relative).parent.mkdir(parents=True, exist_ok=True)
            (directory / relative).write_bytes(content.encode())
        return {"name": name, "version": "1.0.0", "license": "MIT", "license_file": license_file,
                "source": "registry+https://github.com/rust-lang/crates.io-index",
                "manifest_path": str(directory / "Cargo.toml")}

    def assert_linked_home_and_crate_root_are_accepted(self, kind):
        # A linked cargo home (~/.cargo -> /data/cargo, /home -> /var/home) and a registry
        # crate folder that is itself a link substitute nothing from outside the crate.
        home = self.link(kind, self.root / "home/.cargo", self.root / "disk/cargo")
        zip_crate = self.crate(home / "registry/src/index-0000/zip-1.0.0", license_file="COPYING.txt",
                               files={"LICENSE-MIT": "zip terms\n", "COPYING.txt": "zip copying\n"})
        crate = self.link(kind, self.root / "registry/xz-1.0.0", self.root / "disk/xz")
        plan, inventory = self.notices.notice_plan({"packages": [zip_crate, self.crate(crate, "xz")]})
        self.assertEqual(plan, {"zip-1.0.0": {"COPYING.txt": b"zip copying\n", "LICENSE-MIT": b"zip terms\n"},
                                "xz-1.0.0": {"LICENSE-MIT": b"xz terms\n"}})
        self.assertEqual([entry["name"] for entry in inventory], ["zip", "xz"])

    def test_symlinked_home_and_crate_root_are_accepted(self):
        self.assert_linked_home_and_crate_root_are_accepted("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_junction_home_and_crate_root_are_accepted(self):
        self.assert_linked_home_and_crate_root_are_accepted("junction")

    def assert_links_inside_a_crate_are_refused(self, kind):
        # A link named like a notice is refused when it is found, and a declared license
        # file is read without following a linked folder on its way.
        listed = self.root / "registry/listed-1.0.0"
        packages = [self.crate(listed, "listed")]
        self.link(kind, listed / "LICENSES", self.outside)
        declared = self.root / "registry/declared-1.0.0"
        packages.append(self.crate(declared, "declared", license_file="docs/LICENSE"))
        self.link(kind, declared / "docs", self.outside)
        for package in packages:
            with self.subTest(package=package["name"]), self.assertRaisesRegex(SystemExit, "link"):
                self.notices.notice_plan({"packages": [package]})

    def test_symlinks_inside_a_crate_are_refused(self):
        self.assert_links_inside_a_crate_are_refused("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_junctions_inside_a_crate_are_refused(self):
        self.assert_links_inside_a_crate_are_refused("junction")

    def test_symlinked_license_file_is_refused(self):
        crate = self.root / "registry/redirect-1.0.0"
        package = self.crate(crate, "redirect")
        try:
            (crate / "LICENSE").symlink_to(self.outside / "LICENSE")
        except OSError as error:
            self.skipTest(f"OS does not permit fixture symlinks: {error}")
        with self.assertRaisesRegex(SystemExit, "link"):
            self.notices.notice_plan({"packages": [package]})

    def test_declared_license_file_cannot_escape_its_crate(self):
        crate = self.root / "registry/escape-1.0.0"
        (crate.parent / "LICENSE").parent.mkdir(parents=True, exist_ok=True)
        (crate.parent / "LICENSE").write_text("Unrelated private content\n")
        for license_file in ("../LICENSE", str(self.outside / "LICENSE"), "LICENSE-MIT:stream"):
            with self.subTest(license_file=license_file):
                package = self.crate(crate, "escape", license_file=license_file)
                with self.assertRaisesRegex(SystemExit, "escapes its package"):
                    self.notices.notice_plan({"packages": [package]})


# Runs the notices command line in a child Python. Only Cargo is replaced: a process replays
# the fixture metadata's raw UTF-8 bytes, so the script's own decoding of the output is real.
REPLAYING_CARGO = """import importlib.util, subprocess, sys
script, metadata, *arguments = sys.argv[1:]
check_output = subprocess.check_output
def cargo(command, **options):
    replay = "import sys; sys.stdout.buffer.write(open(sys.argv[1], 'rb').read())"
    return check_output([sys.executable, "-c", replay, metadata], **options)
subprocess.check_output = cargo
spec = importlib.util.spec_from_file_location("archive_notices", script)
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)
sys.argv = [script, *arguments]
notices.main()
"""


@unittest.skipUnless(os.name == "nt", "Python before 3.15 decodes text output in the Windows ANSI code page")
class ArchiveNoticeEncodingWindows(unittest.TestCase):
    def test_cargo_metadata_below_a_non_ascii_path_is_decoded_as_utf8(self):
        # A crates.io dependency below a non-ASCII CARGO_HOME, such as a user profile. Cargo
        # writes UTF-8 JSON, which Python's pre-3.15 default (-X utf8=0) decoded as ANSI.
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="archive notice encoding ", dir=temporary)
        self.addCleanup(owner.cleanup)
        root = Path(owner.name)
        source = root / "Jiří 文档/.cargo/registry/src/index.crates.io-0000/zip-2.0.0"
        source.mkdir(parents=True)
        (source / "LICENSE-MIT").write_text("zip terms\n", encoding="utf-8")
        metadata = root / "metadata.json"
        metadata.write_bytes(json.dumps({"packages": [{
            "name": "zip", "version": "2.0.0", "license": "MIT",
            "source": "registry+https://github.com/rust-lang/crates.io-index",
            "manifest_path": str(source / "Cargo.toml")}]}, ensure_ascii=False).encode("utf-8"))
        output = root / "_build/package/licenses"
        result = subprocess.run(
            [sys.executable, "-X", "utf8=0", "-B", "-c", REPLAYING_CARGO, str(PLUGIN / "scripts/notices.py"),
             str(metadata), "--config", str(metadata), "--output", str(output)],
            capture_output=True, encoding="utf-8", errors="replace", timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((output / "zip-2.0.0/LICENSE-MIT").read_text(encoding="utf-8"), "zip terms\n")


if __name__ == "__main__":
    unittest.main()
