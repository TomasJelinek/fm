"""Pinned CEF preparation validates archives and SDK caches before publication."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock


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

    def archive(self, platform="linux", extra=()):
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
        archive = self.cache / filename
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
            self.assertEqual((result / "chrome-sandbox").stat().st_mode & 0o7777, 0o755)
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
        self.output.symlink_to(outside, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.prepare(pin)
        self.assertEqual(list(outside.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
