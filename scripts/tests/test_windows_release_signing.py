"""Release signing changes bytes without weakening package integrity checks."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest
import zipfile

from test_runtime_packages import complete_runtime, packaging, write_file

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "windows_release_signing", ROOT / "scripts/windows-release-signing.py")
signing = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(signing)


class WindowsReleaseSigning(unittest.TestCase):
    def setUp(self):
        parent = ROOT / "_build/tmp"
        parent.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(dir=parent)
        self.addCleanup(owner.cleanup)
        self.base = Path(owner.name)
        self.version = "0.3.0"
        self.revision = "b" * 40
        self.name = "midnight-explorer-0.3.0-x86_64-pc-windows-msvc"
        self.source = self.base / "source"
        complete_runtime(self.source, signing.TARGET)
        for kind in ("native", "network"):
            directory = self.source / "filesystems" / kind
            before = kind + ".dll"
            after = ("midnight_filesystem_native_windows.dll" if kind == "native"
                     else "midnight_filesystem_network.dll")
            (directory / before).rename(directory / after)
            manifest = directory / "plugin.toml"
            manifest.write_text(manifest.read_text().replace(before, after))
        directory = self.source / "filesystems/archive"
        (directory / "archive-worker.exe").rename(directory / "midnight-fs-archive.exe")
        manifest = directory / "plugin.toml"
        manifest.write_text(manifest.read_text().replace("archive-worker.exe", "midnight-fs-archive.exe"))
        thumbs = self.source / "thumbnailers"
        write_file(thumbs, "midnight-video-playback.exe")
        write_file(thumbs, "swresample-7.dll")
        manifest = thumbs / "midnight-thumbnail-video.json"
        data = json.loads(manifest.read_text())
        data["version"] = "ffmpeg-9.0.2-playback-r3"
        manifest.write_text(json.dumps(data))
        self.stage = self.base / self.name
        self.stage.mkdir()
        for name in ("me.exe", "midnight-elevated-listing.exe"):
            shutil.copyfile(self.source / name, self.stage / name)
        viewers = packaging.copy_viewer_packages(
            self.stage, self.source / "viewers", signing.TARGET,
            required_ids=packaging.REQUIRED_VIEWERS)
        filesystems = packaging.copy_filesystem_packages(
            self.stage, self.source / "filesystems", signing.TARGET,
            require_native=True, require_network=True, require_archive=True)
        thumbnails = packaging.copy_thumbnail_packages(
            self.stage, thumbs, signing.TARGET, require_media=True)
        self.info = {"version": self.version, "revision": self.revision,
                     "target": signing.TARGET, "binary": "me.exe", "profile": "release",
                     "binary_sha256": packaging.sha256_file(self.stage / "me.exe"),
                     "authorization_helper": self.record("midnight-elevated-listing.exe"),
                     "viewer_plugins": viewers, "filesystem_plugins": filesystems,
                     "thumbnail_workers": thumbnails}
        self.archive = self.base / (self.name + ".zip")
        self.work = self.base / "work"

    def record(self, name):
        path = self.stage / name
        return {"path": name, "bytes": path.stat().st_size,
                "sha256": packaging.sha256_file(path), "executable": True}

    def bundle(self, extras=()):
        (self.stage / "BUILD-INFO.json").write_text(json.dumps(self.info))
        with zipfile.ZipFile(self.archive, "w", zipfile.ZIP_DEFLATED) as bundle:
            for path in sorted(self.stage.rglob("*")):
                if path.is_file():
                    bundle.write(path, path.relative_to(self.stage.parent).as_posix())
            for name, data in extras:
                entry = zipfile.ZipInfo()
                # ZipInfo(name) normalizes backslashes on Windows. Exercise raw
                # archive names, including those produced on other platforms.
                entry.filename = name
                bundle.writestr(entry, data)

    def prepare(self):
        self.bundle()
        return signing.prepare(self.archive, self.work, self.revision, self.version)

    def simulate_signing_receipt(self, plan):
        root = Path(plan["root"])
        records = []
        for name in plan["files"]:
            with (root / name).open("ab") as stream:
                stream.write(b" simulated signature")
            records.append({"path": name, "sha256": packaging.sha256_file(root / name),
                            "thumbprint": "A" * 40})
        receipt = {"schema": 1, "plan_sha256": packaging.sha256_file(self.work / "signing-plan.json"),
                   "publisher": "CN=Fixture", "files": records}
        (self.work / "verification.json").write_text(json.dumps(receipt))

    def test_signing_scope_is_explicit_and_preserves_upstream_runtime(self):
        plan = self.prepare()
        self.assertEqual(set(plan["files"]), set(signing.SIGNING_FILES))
        self.assertNotIn("viewers/midnight-viewer-html/libcef.dll", plan["files"])
        catalog = (self.work / "signing-files.txt").read_text().splitlines()
        self.assertEqual(len(catalog), len(signing.SIGNING_FILES))
        self.assertTrue(all(not Path(name).is_absolute() for name in catalog))

    def test_finalize_refreshes_inner_and_outer_hashes_and_keeps_other_bytes(self):
        plan = self.prepare()
        before = (self.stage / "viewers/midnight-viewer-html/libcef.dll").read_bytes()
        self.simulate_signing_receipt(plan)
        archive = signing.finalize(self.work)
        root = Path(plan["root"])
        packaging.validate_runtime_installation(root, signing.TARGET)
        data = json.loads((root / "BUILD-INFO.json").read_text())
        self.assertEqual(data["binary_sha256"], packaging.sha256_file(root / "me.exe"))
        html = next(item for item in data["viewer_plugins"] if item["id"] == "midnight.viewer.html")
        self.assertEqual(html["package_sha256"], signing.fingerprint(html["files"]))
        self.assertEqual((root / "viewers/midnight-viewer-html/libcef.dll").read_bytes(), before)
        inner = json.loads((root / "viewers/midnight-viewer-html/runtime-files.json").read_text())
        self.assertEqual(inner["archive_sha256"], "a" * 64)
        self.assertTrue(archive.is_file())
        self.assertEqual(archive.with_suffix(".zip.sha256").read_text().split()[0],
                         packaging.sha256_file(archive))
        with zipfile.ZipFile(archive) as bundle:
            self.assertEqual(bundle.read(self.name + "/me.exe"), (root / "me.exe").read_bytes())

    def test_finalize_requires_native_verification_receipt(self):
        self.prepare()
        with self.assertRaises((ValueError, FileNotFoundError)):
            signing.finalize(self.work)
        self.assertFalse((self.work / "output").exists())

    def test_finalize_rejects_unrelated_file_mutation_and_stale_receipt(self):
        for name in ("viewers/midnight-viewer-html/libcef.dll", "me.exe"):
            with self.subTest(path=name):
                if self.work.exists():
                    shutil.rmtree(self.work)
                plan = self.prepare()
                self.simulate_signing_receipt(plan)
                with (Path(plan["root"]) / name).open("ab") as stream:
                    stream.write(b"tamper")
                with self.assertRaises(ValueError):
                    signing.finalize(self.work)
                self.assertFalse((self.work / "output").exists())

    def test_prepare_rejects_wrong_revision_or_initial_hash(self):
        for field in ("revision", "binary_sha256"):
            with self.subTest(field=field):
                self.info[field] = "0" * (40 if field == "revision" else 64)
                self.bundle()
                with self.assertRaises(ValueError):
                    signing.prepare(self.archive, self.work, self.revision, self.version)
                self.info[field] = (self.revision if field == "revision" else
                                    packaging.sha256_file(self.stage / "me.exe"))
                if self.work.exists():
                    shutil.rmtree(self.work)

    def test_prepare_rejects_unknown_owned_binary(self):
        write_file(self.stage, "thumbnailers/midnight-extra.exe")
        self.bundle()
        with self.assertRaises(ValueError):
            signing.prepare(self.archive, self.work, self.revision, self.version)

    def test_prepare_rejects_unsigned_redirected_plugin_entry_even_with_valid_inventory(self):
        directory = self.stage / "viewers/midnight-viewer-basic"
        (directory / "other.dll").write_bytes(b"unsigned replacement entry")
        manifest = directory / "plugin.toml"
        manifest.write_text(manifest.read_text().replace("midnight_viewer_basic.dll", "other.dll"))
        group = next(item for item in self.info["viewer_plugins"] if item["id"] == "midnight.viewer.basic")
        group["files"].append(self.record("viewers/midnight-viewer-basic/other.dll"))
        for record in group["files"]:
            record.update(self.record(record["path"]))
        group["package_sha256"] = signing.fingerprint(group["files"])
        self.bundle()
        with self.assertRaises(ValueError):
            signing.prepare(self.archive, self.work, self.revision, self.version)

    def test_prepare_rejects_unsafe_duplicate_and_case_colliding_zip_paths(self):
        names = ("../outside", self.name + "/../outside", self.name + "/NUL.txt",
                 self.name + "/bad:stream", self.name + "/bad\\path",
                 self.name + "/ME.exe", self.name + "/me.exe", self.name + "/trailing. ")
        for name in names:
            with self.subTest(name=name):
                self.bundle([(name, b"bad")])
                with self.assertRaises(ValueError):
                    signing.prepare(self.archive, self.work, self.revision, self.version)
                if self.work.exists():
                    shutil.rmtree(self.work)

    def test_prepare_rejects_zip_symlinks_before_extracting(self):
        self.bundle()
        with zipfile.ZipFile(self.archive, "a") as bundle:
            entry = zipfile.ZipInfo(self.name + "/link")
            entry.create_system = 3
            entry.external_attr = (stat.S_IFLNK | 0o777) << 16
            bundle.writestr(entry, "../../outside")
        with self.assertRaises(ValueError):
            signing.prepare(self.archive, self.work, self.revision, self.version)

    @unittest.skipUnless(os.name == "nt" and shutil.which("pwsh"), "Windows Authenticode boundary")
    def test_native_verifier_accepts_unsigned_preflight_but_refuses_release(self):
        plan = self.prepare()
        assembly = self.base / "unsigned.dll"
        environment = dict(os.environ, SIGNING_TEST_ASSEMBLY=str(assembly))
        result = subprocess.run([shutil.which("pwsh"), "-NoLogo", "-NoProfile", "-Command",
                                 "Add-Type -TypeDefinition 'public class SigningFixture {}' "
                                 "-OutputAssembly $env:SIGNING_TEST_ASSEMBLY"],
                                env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in plan["files"]:
            shutil.copyfile(assembly, Path(plan["root"]) / name)
        command = [shutil.which("pwsh"), "-NoLogo", "-NoProfile", "-File",
                   str(ROOT / "scripts/verify-windows-release.ps1"),
                   "-Plan", str(self.work / "signing-plan.json"), "-Publisher", "CN=Fixture"]
        result = subprocess.run([*command, "-BeforeSigning"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.work / "verification.json").exists())
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected publisher and timestamp", result.stderr)
        self.assertFalse((self.work / "verification.json").exists())


if __name__ == "__main__":
    unittest.main()
