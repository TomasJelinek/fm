"""Installed worker manifests and executable modes survive release packaging."""
import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location(
    "package_release", Path(__file__).resolve().parents[1] / "package-release.py")
packaging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packaging)


class WorkerPackages(unittest.TestCase):
    def setUp(self):
        temporary = Path(__file__).resolve().parents[2] / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.stage = self.root / "release"
        self.stage.mkdir()
        self.target = "x86_64-unknown-linux-gnu"

    def filesystem(self):
        source = self.root / "filesystems"
        package = source / "archive"
        package.mkdir(parents=True)
        (package / "plugin.toml").write_text(
            'schema_version=1\nid="archive"\nkind="filesystem-worker"\nversion="0.1.0"\n'
            f'target="{self.target}"\nprotocol=1\nentry_executable="archive-worker"\n'
            'formats=["zip","gzip","tar","tar-gzip","7z"]\n')
        (package / "archive-worker").write_bytes(b"archive fixture")
        self.notices(package)
        return source

    def thumbnails(self):
        source = self.root / "thumbnailers"
        source.mkdir()
        (source / "image.json").write_text(json.dumps({
            "protocol": 1, "id": "image", "version": "0.1.0", "executable": "image-worker",
            "extensions": ["png", "jpg"], "video": False,
        }))
        (source / "image-worker").write_bytes(b"image fixture")
        (source / "libavcodec.so.62").write_bytes(b"shared library fixture")
        self.notices(source)
        (source / "notices/image-worker").mkdir(parents=True)
        (source / "notices/image-worker/LICENSE").write_text("Actual upstream licence")
        return source

    def notices(self, directory):
        (directory / "THIRD-PARTY.md").write_text("Pinned attribution")
        (directory / "licenses").mkdir()
        (directory / "licenses/upstream-MIT").write_text("Actual upstream terms")

    def test_both_worker_layouts_preserve_notices_hashes_and_executable_modes(self):
        fs = packaging.copy_filesystem_packages(self.stage, self.filesystem(), self.target)
        thumbs = packaging.copy_thumbnail_packages(self.stage, self.thumbnails(), self.target)
        self.assertEqual(fs[0]["kind"], "filesystem-worker")
        self.assertEqual(thumbs["providers"][0]["id"], "image")
        self.assertEqual(len(fs[0]["package_sha256"]), 64)
        self.assertTrue((self.stage / "thumbnailers/licenses/upstream-MIT").is_file())
        executable = ["filesystems/archive/archive-worker", "thumbnailers/image-worker"]
        for archive_format in ("tar.gz", "zip"):
            output = self.root / f"workers.{archive_format}"
            packaging.write_archive(self.stage, output, "me", archive_format, executable)
            expected = {"release/" + name: 0o755 for name in executable}
            expected["release/thumbnailers/libavcodec.so.62"] = 0o644
            expected["release/filesystems/archive/plugin.toml"] = 0o644
            if archive_format == "zip":
                with zipfile.ZipFile(output) as bundle:
                    for name, mode in expected.items():
                        self.assertEqual(bundle.getinfo(name).external_attr >> 16 & 0o777, mode)
            else:
                with tarfile.open(output) as bundle:
                    for name, mode in expected.items():
                        self.assertEqual(bundle.getmember(name).mode, mode)

    def test_filesystem_schema_target_protocol_and_entry_escape_fail_before_copy(self):
        source = self.filesystem()
        manifest = source / "archive/plugin.toml"
        original = manifest.read_text()
        for old, new in [('kind="filesystem-worker"', 'kind="viewer"'),
                         (self.target, "wrong-target"), ('protocol=1', 'protocol=true'), ('protocol=1', 'protocol=4'),
                         ('entry_executable="archive-worker"', 'entry_executable="../worker"'),
                         ('formats=["zip","gzip","tar","tar-gzip","7z"]', 'formats=["rar"]')]:
            with self.subTest(new=new):
                manifest.write_text(original.replace(old, new))
                with self.assertRaises(ValueError):
                    packaging.copy_filesystem_packages(self.stage, source, self.target)
                self.assertEqual(list(self.stage.iterdir()), [])

    def test_streaming_filesystem_protocol_two_preserves_manifest_and_executable(self):
        source = self.filesystem()
        manifest = source / "archive/plugin.toml"
        manifest.write_text(manifest.read_text().replace("protocol=1", "protocol=2"))
        inventory = packaging.copy_filesystem_packages(self.stage, source, self.target)
        self.assertEqual(inventory[0]["kind"], "filesystem-worker")
        self.assertIn("protocol=2", (self.stage / "filesystems/archive/plugin.toml").read_text())
        self.assertEqual((self.stage / "filesystems/archive/archive-worker").read_bytes(), b"archive fixture")

    def test_cooperative_filesystem_protocol_three_preserves_manifest_and_executable(self):
        source = self.filesystem()
        manifest = source / "archive/plugin.toml"
        manifest.write_text(manifest.read_text().replace("protocol=1", "protocol=3"))
        inventory = packaging.copy_filesystem_packages(self.stage, source, self.target)
        self.assertEqual(inventory[0]["kind"], "filesystem-worker")
        self.assertIn("protocol=3", (self.stage / "filesystems/archive/plugin.toml").read_text())
        self.assertEqual((self.stage / "filesystems/archive/archive-worker").read_bytes(), b"archive fixture")

    def test_optional_stable_member_ids_are_preserved_with_boolean_type(self):
        source = self.filesystem()
        manifest = source / "archive/plugin.toml"
        original = manifest.read_text()
        for value in (True, False):
            with self.subTest(value=value):
                stage = self.root / f"release-{value}"
                stage.mkdir()
                manifest.write_text(original + f"stable_entry_ids={str(value).lower()}\n")
                inventory = packaging.copy_filesystem_packages(stage, source, self.target)
                self.assertIs(inventory[0]["stable_entry_ids"], value)
                self.assertEqual((stage / "filesystems/archive/plugin.toml").read_bytes(),
                                 manifest.read_bytes())

    def test_non_boolean_stable_member_capability_fails_before_copy(self):
        source = self.filesystem()
        manifest = source / "archive/plugin.toml"
        original = manifest.read_text()
        for value in ("1", "0", '"true"', "[]"):
            with self.subTest(value=value):
                manifest.write_text(original + f"stable_entry_ids={value}\n")
                with self.assertRaises(ValueError):
                    packaging.copy_filesystem_packages(self.stage, source, self.target)
                self.assertEqual(list(self.stage.iterdir()), [])

    def test_thumbnail_protocol_duplicates_and_manifest_path_escape_fail_before_copy(self):
        source = self.thumbnails()
        manifest = source / "image.json"
        original = json.loads(manifest.read_text())
        for key, value in [("protocol", True), ("executable", "../worker"), ("version", "."),
                           ("video", "false"), ("extensions", [".png"])]:
            with self.subTest(key=key):
                manifest.write_text(json.dumps({**original, key: value}))
                with self.assertRaises(ValueError):
                    packaging.copy_thumbnail_packages(self.stage, source, self.target)
                self.assertEqual(list(self.stage.iterdir()), [])
        manifest.write_text(json.dumps(original))
        (source / "duplicate.json").write_text(json.dumps(original))
        with self.assertRaises(ValueError):
            packaging.copy_thumbnail_packages(self.stage, source, self.target)

    def test_ffmpeg_runtime_requires_complete_libraries_corresponding_source_and_build_notices(self):
        source = self.thumbnails()
        (source / "libavcodec.so.63").write_bytes(b"runtime")
        with self.assertRaises(ValueError):
            packaging.copy_thumbnail_packages(self.stage, source, self.target)
        self.assertEqual(list(self.stage.iterdir()), [])
        for name in ("libavformat.so.63", "libavutil.so.61", "libswscale.so.10"):
            (source / name).write_bytes(b"runtime")
        notices = source / "notices/ffmpeg-9.0.2"
        notices.mkdir()
        for name in ("COPYING.LGPLv2.1", "config.h", "config.mak"):
            (notices / name).write_text("Fixture attribution/configuration")
        build_options = (b"FFmpeg 9.0.2\n--enable-shared\n--disable-static\n--disable-gpl\n"
                         b"--disable-nonfree\n--disable-version3\n--disable-network\n"
                         b"--enable-decoder=vp8,vp9,theora,ffv1,rawvideo\n")
        (notices / "build-options.txt").write_bytes(build_options)
        with self.assertRaises(ValueError):
            packaging.copy_thumbnail_packages(self.stage, source, self.target)
        (notices / "ffmpeg-9.0.2.tar.xz").write_bytes(b"Matching source archive fixture")
        (notices / "build-options.txt").unlink()
        with self.assertRaises(ValueError):
            packaging.copy_thumbnail_packages(self.stage, source, self.target)
        self.assertEqual(list(self.stage.iterdir()), [])
        (notices / "build-options.txt").write_bytes(build_options)
        result = packaging.copy_thumbnail_packages(self.stage, source, self.target)
        self.assertTrue(any(file["path"].endswith("ffmpeg-9.0.2.tar.xz") for file in result["files"]))
        self.assertEqual(
            (self.stage / "thumbnailers/notices/ffmpeg-9.0.2/build-options.txt").read_bytes(),
            build_options)

    def test_worker_symlinks_and_missing_notices_are_rejected(self):
        source = self.filesystem()
        (source / "archive/THIRD-PARTY.md").unlink()
        with self.assertRaises(ValueError):
            packaging.copy_filesystem_packages(self.stage, source, self.target)
        thumbs = self.thumbnails()
        link = thumbs / "injected.so"
        try:
            link.symlink_to(self.root / "outside")
        except OSError as error:
            self.skipTest(f"Cannot create symlink fixture: {error}")
        with self.assertRaises(ValueError):
            packaging.copy_thumbnail_packages(self.stage, thumbs, self.target)


if __name__ == "__main__":
    unittest.main()
