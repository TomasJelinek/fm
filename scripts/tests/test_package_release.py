"""Regression checks for the release dependency inventory."""
import importlib.util
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "package_release", Path(__file__).resolve().parents[1] / "package-release.py")
packaging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packaging)


def package(name, source="registry+https://github.com/rust-lang/crates.io-index"):
    return {"name": name, "version": "0.1.0", "source": source}


class DependencyInventoryTests(unittest.TestCase):
    def setUp(self):
        self.metadata = {"packages": [package("midnight-explorer", None),
                                      package("runtime"), package("derive"),
                                      package("unused-workspace-renderer")]}

    def test_uses_application_selection_and_preserves_proc_macro_notices(self):
        tree = ("midnight-explorer v0.1.0 (/checkout/app)\n"
                "runtime v0.1.0\n"
                "derive v0.1.0 (proc-macro)\n"
                "runtime v0.1.0 (*)\n")
        result = packaging.dependency_packages(self.metadata, tree)
        self.assertEqual([entry["name"] for entry in result], ["derive", "runtime"])

    def test_indented_tree_cannot_silently_omit_dependency_notices(self):
        tree = "midnight-explorer v0.1.0\n\u2514\u2500\u2500 runtime v0.1.0\n"
        with self.assertRaises(ValueError):
            packaging.dependency_packages(self.metadata, tree)

    def test_missing_or_ambiguous_metadata_is_rejected(self):
        tree = "midnight-explorer v0.1.0\nruntime v0.1.0\n"
        with self.assertRaises(ValueError):
            packaging.dependency_packages(self.metadata, tree + "missing v0.1.0\n")
        self.metadata["packages"].append(package("runtime", "git+https://example.invalid/fork"))
        with self.assertRaises(ValueError):
            packaging.dependency_packages(self.metadata, tree)


class ArchivePermissionsTests(unittest.TestCase):
    def test_archive_modes_do_not_inherit_world_writable_staging_permissions(self):
        temporary = Path(__file__).resolve().parents[2] / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary) as directory:
            folder = Path(directory)
            stage = folder / "release"
            stage.mkdir()
            for name in ("me", "README.txt"):
                path = stage / name
                path.write_text("fixture", encoding="utf-8")
                path.chmod(0o777)
            for archive_format in ("tar.gz", "zip"):
                with self.subTest(format=archive_format):
                    archive = folder / f"release.{archive_format}"
                    packaging.write_archive(stage, archive, "me", archive_format)
                    if archive_format == "tar.gz":
                        with tarfile.open(archive) as bundle:
                            self.assertEqual(bundle.getmember("release/me").mode, 0o755)
                            self.assertEqual(bundle.getmember("release/README.txt").mode, 0o644)
                            self.assertEqual(bundle.getmember("release").mode, 0o755)
                            self.assertEqual(bundle.getmember("release/me").uid, 0)
                    else:
                        with zipfile.ZipFile(archive) as bundle:
                            self.assertEqual(bundle.getinfo("release/me").external_attr >> 16 & 0o777, 0o755)
                            self.assertEqual(bundle.getinfo("release/README.txt").external_attr >> 16 & 0o777, 0o644)


if __name__ == "__main__":
    unittest.main()
