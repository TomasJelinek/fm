"""Process image viewers keep isolation, attribution and executable permissions."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location(
    "package_release", Path(__file__).resolve().parents[1] / "package-release.py")
packaging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packaging)


class ImageViewerPackages(unittest.TestCase):
    def setUp(self):
        temporary = Path(__file__).resolve().parents[2] / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.viewers = self.root / "viewers"
        self.viewers.mkdir()
        self.stage = self.root / "release"
        self.stage.mkdir()
        self.target = "x86_64-unknown-linux-gnu"

    def package(self, name="image", identity="midnight.viewer.image"):
        package = self.viewers / name
        package.mkdir()
        manifest = {"schema": 1, "protocol": 1, "id": identity,
                    "name": "Images", "version": "0.1.0", "target": self.target,
                    "executable": "image-worker", "extensions": ["png", "jpg"]}
        (package / "image-viewer.json").write_text(json.dumps(manifest), encoding="utf-8")
        (package / "image-worker").write_bytes(b"independent sandbox worker fixture")
        (package / "LICENSE").write_text("Plugin license", encoding="utf-8")
        (package / "THIRD-PARTY.md").write_text("Pinned dependency attribution", encoding="utf-8")
        (package / "notices").mkdir()
        (package / "notices/image-LICENSE").write_text("Codec license", encoding="utf-8")
        return package, manifest

    def test_process_viewer_preserves_manifest_notices_and_executable_mode(self):
        self.package()
        inventory = packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        self.assertEqual(inventory[0]["kind"], "image-viewer-worker")
        self.assertEqual(inventory[0]["entry_executable"], "image-worker")
        executable = [entry["path"] for entry in inventory[0]["files"]
                      if entry.get("executable")]
        self.assertEqual(executable, ["viewers/image/image-worker"])
        self.assertEqual(len(inventory[0]["package_sha256"]), 64)
        output = self.root / "release.zip"
        packaging.write_archive(self.stage, output, "me", "zip", executable)
        with zipfile.ZipFile(output) as archive:
            self.assertEqual(archive.getinfo("release/viewers/image/image-worker").external_attr
                             >> 16 & 0o777, 0o755)
            self.assertEqual(archive.read("release/viewers/image/notices/image-LICENSE"),
                             b"Codec license")

    def test_malformed_worker_packages_fail_before_staging(self):
        package, original = self.package()
        path = package / "image-viewer.json"
        cases = [("schema", True), ("protocol", 2), ("target", "wrong-target"),
                 ("executable", "../worker"), ("executable", "image-worker.exe"),
                 ("extensions", [".png"]), ("extensions", ["png", "png"]),
                 ("unknown", "ignored?"), ("name", "")]
        for key, value in cases:
            with self.subTest(key=key, value=value):
                path.write_text(json.dumps({**original, key: value}), encoding="utf-8")
                with self.assertRaises(ValueError):
                    packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
                self.assertEqual(list(self.stage.iterdir()), [])

    def test_duplicate_ids_and_ambiguous_native_worker_package_are_refused(self):
        package, _ = self.package()
        (package / "plugin.toml").write_text('schema_version=1', encoding="utf-8")
        with self.assertRaises(ValueError):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        (package / "plugin.toml").unlink()
        self.package("other")
        with self.assertRaises(ValueError):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        self.assertEqual(list(self.stage.iterdir()), [])

    def test_missing_license_is_refused(self):
        package, _ = self.package()
        (package / "LICENSE").unlink()
        with self.assertRaises(ValueError):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        self.assertEqual(list(self.stage.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
