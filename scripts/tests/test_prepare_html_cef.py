"""Pinned CEF preparation validates archives and SDK caches before publication."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from unittest import mock

# Sibling fixture module, also when run as scripts.tests.<module> from the checkout.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from link_fixtures import directory_link, hidden_directory, hold  # noqa: E402


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/prepare-html-cef.py"


class PrepareHtmlCef(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), "Verified CEF preparation is missing")
        spec = importlib.util.spec_from_file_location("prepare_html_cef", SCRIPT)
        self.cef = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.cef)
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="CEF fixture ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / "_build/cef-sdk"
        self.cache = self.root / "cache"
        self.cache.mkdir()

    def archive(self, platform="linux", extra=(), cache=None):
        filename = f"cef_fixture_{platform}_minimal.tar.bz2"
        prefix = filename.removesuffix(".tar.bz2")
        files = {"LICENSE.txt": b"CEF license\n", "CREDITS.html": b"Component notices\n",
                 "CMakeLists.txt": b"CMake fixture\n", "include/cef_version.h": b"Version fixture\n",
                 "cmake/cef_variables.cmake": b"Sandbox configuration\n",
                 "libcef_dll/wrapper.cc": b"Wrapper fixture\n"}
        files.update({"libcef.so": b"Linux runtime\n", "chrome-sandbox": b"Sandbox fixture\n",
                      "locales/en-US.pak": b"Locale data\n"} if platform == "linux" else
                     {"libcef.dll": b"Windows runtime\n", "bootstrap.exe": b"Sandbox bootstrap\n",
                      "libcef.lib": b"Import library\n", "locales/en-US.pak": b"Locale data\n"})
        archive = (cache or self.cache) / filename
        archive.parent.mkdir(parents=True, exist_ok=True)
        with tarfile.open(archive, "w:bz2") as bundle:
            for name, contents in files.items():
                section = "Resources/" if name.startswith("locales/") else "Release/"
                original = name if "/" in name and not name.startswith("locales/") or name in (
                    "LICENSE.txt", "CREDITS.html", "CMakeLists.txt") else section + name
                info = tarfile.TarInfo(prefix + "/" + original)
                info.mode = 0o4755 if name == "chrome-sandbox" else 0o644
                info.size = len(contents)
                bundle.addfile(info, io.BytesIO(contents))
            for relative, kind in extra:
                info = tarfile.TarInfo(prefix + "/" + relative)
                info.type = kind
                info.linkname = "/outside"
                bundle.addfile(info)
        pin = {"archive": filename, "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
               "sha1": hashlib.sha1(archive.read_bytes()).hexdigest(), "bytes": archive.stat().st_size,
               "cef": "pinned fixture"}
        marker = {"type": "minimal", "name": filename, "sha1": pin["sha1"]}
        files["archive.json"] = (json.dumps(marker) + "\n").encode()
        pin["files"] = {name: hashlib.sha256(contents).hexdigest() for name, contents in files.items()}
        return archive, pin, files

    def prepare(self, pin, platform="linux"):
        return self.cef.prepare(self.output, offline=True, platform_name=platform,
                                cache_root=self.cache, environ={}, pin=pin)

    def test_verified_linux_archive_flattens_exact_inputs_and_repairs_changed_sdk_cache(self):
        _, pin, files = self.archive()
        with mock.patch.object(self.cef.urllib.request, "urlopen", side_effect=AssertionError("network")):
            result = self.prepare(pin)
            self.assertEqual(result, self.output)
            self.assertEqual({path.relative_to(result).as_posix(): path.read_bytes()
                              for path in result.rglob("*") if path.is_file()}, files)
            sandbox_mode = (result / "chrome-sandbox").stat().st_mode & 0o7777
            self.assertEqual(sandbox_mode & 0o7000, 0)
            if os.name == "nt":
                self.assertTrue(sandbox_mode & 0o200, "Windows fixture must remain writable")
            else:
                self.assertEqual(sandbox_mode, 0o755)
            (result / "libcef.so").write_bytes(b"Corrupt SDK cache")
            self.assertEqual(self.prepare(pin), self.output)
            self.assertEqual((result / "libcef.so").read_bytes(), b"Linux runtime\n")
            self.assertEqual(self.prepare(pin), result)

    def test_windows_archive_retains_bootstrap_notices_and_revalidates_receipt(self):
        _, pin, _ = self.archive("win32")
        result = self.prepare(pin, "win32")
        self.assertEqual((result / "bootstrap.exe").read_bytes(), b"Sandbox bootstrap\n")
        self.assertEqual((result / "CREDITS.html").read_bytes(), b"Component notices\n")
        (result / "libcef.dll").write_bytes(b"Changed runtime")
        self.prepare(pin, "win32")
        self.assertEqual((result / "libcef.dll").read_bytes(), b"Windows runtime\n")

    def test_wrong_archive_is_rejected_before_replacing_a_previously_verified_sdk(self):
        archive, pin, _ = self.archive()
        self.prepare(pin)
        archive.write_bytes(b"Wrong archive")
        with self.assertRaisesRegex(ValueError, "checksum|size"):
            self.prepare(pin)
        self.assertEqual((self.output / "libcef.so").read_bytes(), b"Linux runtime\n")

    def test_offline_cache_absence_never_downloads_or_creates_an_sdk(self):
        with mock.patch.object(self.cef.urllib.request, "urlopen", side_effect=AssertionError("network")):
            with self.assertRaisesRegex(ValueError, "[Oo]ffline"):
                self.cef.prepare(self.output, offline=True, cache_root=self.cache, environ={})
        self.assertFalse(self.output.exists())

    def test_parent_traversal_links_and_duplicate_archive_members_are_rejected(self):
        for relative, kind in (("../escape", tarfile.REGTYPE), ("Release/link", tarfile.SYMTYPE),
                               ("Release/libcef.so", tarfile.REGTYPE)):
            with self.subTest(relative=relative):
                _, pin, _ = self.archive(extra=((relative, kind),))
                with self.assertRaises(ValueError):
                    self.prepare(pin)
                self.assertFalse(self.output.exists())
                self.assertFalse((self.root / "escape").exists())

    def test_output_outside_build_or_through_a_link_is_rejected(self):
        _, pin, _ = self.archive()
        with self.assertRaisesRegex(ValueError, "_build"):
            self.cef.prepare(Path("/tmp") / self.root.name / "cef-sdk", offline=True,
                             cache_root=self.cache, environ={}, pin=pin)
        self.output.parent.mkdir(parents=True)
        outside = self.root / "outside"
        outside.mkdir()
        if os.name == "nt":
            subprocess.run(
                ["cmd.exe", "/d", "/c", "mklink", "/J", str(self.output), str(outside)],
                check=True, capture_output=True, text=True,
                creationflags=subprocess.CREATE_NO_WINDOW)
            self.addCleanup(os.rmdir, self.output)
            self.assertTrue(self.output.lstat().st_file_attributes & 0x400)
        else:
            self.output.symlink_to(outside, target_is_directory=True)
            self.addCleanup(self.output.unlink)
        self.assertTrue(self.output.samefile(outside))
        with self.assertRaisesRegex(ValueError, "links/reparse points"):
            self.prepare(pin)
        self.assertEqual(list(outside.iterdir()), [])

    def linked_checkout(self):
        """A checkout below hidden AppData, reached through a directory link to AppData.

        The script's checkout is pointed at it, as if the script were run from there:
        like a linked parent folder, Fedora Silverblue's /home or a profile's AppData.
        """
        real = hidden_directory(self.root / "AppData") / "checkout"
        (real / ".tools/cef").mkdir(parents=True)
        (real / "_build").mkdir()
        linked = directory_link(self.root / "linked", real.parent) / "checkout"
        patcher = mock.patch.object(self.cef, "ROOT", real)
        patcher.start()
        self.addCleanup(patcher.stop)
        return real, linked

    def prepare_in(self, checkout, output, pin, **environment):
        return self.cef.prepare(output, offline=True, platform_name="linux",
                                cache_root=checkout / ".tools/cef", environ=environment, pin=pin)

    def test_checkout_through_a_link_and_a_hidden_folder_prepares_below_its_own_build(self):
        real, checkout = self.linked_checkout()
        _, pin, files = self.archive(cache=real / ".tools/cef")
        output = checkout / "_build/html-viewer/cef-sdk/linux"
        with mock.patch.object(self.cef.urllib.request, "urlopen", side_effect=AssertionError("network")):
            self.assertEqual(self.prepare_in(checkout, output, pin), output)
            self.assertEqual(self.prepare_in(checkout, output, pin), output)
        installed = real / "_build/html-viewer/cef-sdk/linux"
        self.assertEqual({path.relative_to(installed).as_posix(): path.read_bytes()
                          for path in installed.rglob("*") if path.is_file()}, files)

    def test_links_from_build_down_and_outputs_outside_build_are_refused(self):
        real, checkout = self.linked_checkout()
        _, pin, _ = self.archive(cache=real / ".tools/cef")
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "keep").write_text("user data\n")
        directory_link(real / "_build/redirect", outside)
        directory_link(real / "_build/loop", real)
        # Outside the checkout the walk starts at the path's first _build, which for this
        # fixture is the repository's own _build above it; a link below that is refused.
        custom = directory_link(self.root / "custom target", outside)
        refusals = {"links/reparse points": (checkout / "_build/redirect/cef-sdk",
                                             checkout / "_build/loop/_build/cef-sdk",
                                             custom / "_build/cef-sdk"),
                    "must be under _build": (checkout / "cef-sdk", checkout / "sources/_build/cef-sdk")}
        for message, outputs in refusals.items():
            for output in outputs:
                with self.subTest(output=output):
                    with self.assertRaisesRegex(ValueError, message):
                        self.prepare_in(checkout, output, pin)
        self.assertEqual([path.name for path in outside.iterdir()], ["keep"])
        self.assertFalse((checkout / "cef-sdk").exists())

    def test_download_cache_stays_a_plain_folder_inside_the_checkout(self):
        # Kept stricter than the input rule: the verified archive is opened again by path.
        real, checkout = self.linked_checkout()
        shared = self.root / "shared tools"
        _, pin, _ = self.archive(cache=shared / "cef")
        output = checkout / "_build/cef-sdk"
        with self.assertRaisesRegex(ValueError, "inside the checkout"):
            self.cef.prepare(output, offline=True, platform_name="linux", cache_root=shared / "cef",
                             environ={}, pin=pin)
        (real / ".tools/cef").rmdir()
        (real / ".tools").rmdir()
        directory_link(real / ".tools", shared)
        with self.assertRaisesRegex(ValueError, "links/reparse points"):
            self.prepare_in(checkout, output, pin)
        self.assertFalse((real / "_build/cef-sdk").exists())

    def test_linux_cef_path_may_itself_be_a_link_but_nothing_inside_it(self):
        _, pin, _ = self.archive()
        self.prepare(pin)
        link = directory_link(self.root / "sdk link", self.output)
        override = {"CEF_PATH": str(link)}
        self.assertEqual(self.cef.prepare(self.root / "_build/unused", offline=True, platform_name="linux",
                                          cache_root=self.cache, environ=override, pin=pin), link)
        outside = self.root / "outside"
        outside.mkdir()
        directory_link(self.output / "linked-resources", outside)
        with self.assertRaisesRegex(ValueError, "SDK"):
            self.cef.prepare(self.root / "_build/unused", offline=True, platform_name="linux",
                             cache_root=self.cache, environ=override, pin=pin)

    def held_extraction(self, seconds):
        """Extract as usual, then keep a fresh file of the new stage open for `seconds`."""
        extract, holds = self.cef.extract_flat, []

        def extract_and_hold(archive, output, pin, platform_name):
            extract(archive, output, pin, platform_name)
            holds.append(hold(output / "LICENSE.txt", seconds))
        # Released at the end of the test; a longer hold needs no waiting.
        self.addCleanup(lambda: [held.release() for held in holds])
        return mock.patch.object(self.cef, "extract_flat", extract_and_hold)

    @unittest.skipUnless(os.name == "nt", "Windows refuses to rename a folder above a held file")
    def test_brief_hold_of_the_new_stage_is_waited_out(self):
        # TEST-04: endpoint protection held a freshly extracted file and the stage rename
        # failed with WinError 5; the rename now waits for about 2 s.
        _, pin, _ = self.archive()
        self.prepare(pin)
        (self.output / "libcef.so").write_bytes(b"Corrupt SDK cache")
        with self.held_extraction(0.3):
            self.assertEqual(self.prepare(pin), self.output)
        self.assertEqual((self.output / "libcef.so").read_bytes(), b"Linux runtime\n")
        self.assertEqual(list(self.output.parent.glob(".cef-stage-*")), [])

    @unittest.skipUnless(os.name == "nt", "Windows refuses to rename a folder above a held file")
    def test_hold_beyond_two_seconds_fails_with_the_rename_error_and_keeps_the_previous_sdk(self):
        _, pin, _ = self.archive()
        self.prepare(pin)
        (self.output / "libcef.so").write_bytes(b"Previous SDK")
        started = time.monotonic()
        with self.held_extraction(2.6), self.assertRaises(PermissionError) as caught:
            self.prepare(pin)
        elapsed = time.monotonic() - started
        self.assertEqual(caught.exception.winerror, 5)
        self.assertGreaterEqual(elapsed, 1.9)
        self.assertEqual((self.output / "libcef.so").read_bytes(), b"Previous SDK")


if __name__ == "__main__":
    unittest.main()
