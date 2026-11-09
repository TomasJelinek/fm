"""Regression checks for the release dependency inventory."""
import importlib.util
import hashlib
import os
import tarfile
import tempfile
import tomllib
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
    def test_distribution_keeps_palette_licenses_and_attribution(self):
        root = Path(__file__).resolve().parents[2]
        temporary = root / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary) as directory:
            stage = Path(directory) / "release"
            stage.mkdir()
            rust_docs = Path(directory) / "rust"
            rust_docs.mkdir()
            (rust_docs / "COPYRIGHT-library.html").write_text("Rust fixture", encoding="utf-8")
            packaging.add_notices(stage, {"packages": [package("midnight-explorer", None)]},
                                  "midnight-explorer v0.1.0\n", rust_docs, None)
            for author in ("folke", "joshdick", "rose-pine", "catppuccin"):
                relative = Path("licenses/themes") / author / "LICENSE"
                self.assertTrue((stage / relative).is_file(), f"Missing palette notice: {author}")
                self.assertEqual((stage / relative).read_bytes(), (root / "docs" / relative).read_bytes())
            self.assertEqual((stage / "THEME_PROVENANCE.md").read_bytes(),
                             (root / "docs/THEME_PROVENANCE.md").read_bytes())

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


class ViewerPackageTests(unittest.TestCase):
    def setUp(self):
        temporary = Path(__file__).resolve().parents[2] / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.viewers = self.root / "input-viewers"
        self.viewers.mkdir()
        self.stage = self.root / "release"
        self.stage.mkdir()
        self.target = "x86_64-pc-windows-msvc"

    def package(self, name="basic", identity="midnight.viewer.basic", entry="viewer.dll"):
        package = self.viewers / name
        package.mkdir()
        manifest = (f'schema_version=1\nid="{identity}"\nkind="viewer"\nversion="0.1.0"\n'
                    f'entry_library="{entry}"\ntarget="{self.target}"\nabi_major=1\n'
                    'minimum_host_minor=0\nmodels=["text-page.v1"]\nmodes=["text"]\n')
        (package / "plugin.toml").write_text(manifest, encoding="utf-8")
        (package / "viewer.dll").write_bytes(b"independent native fixture")
        (package / "LICENSE").write_text("Plugin license", encoding="utf-8")
        (package / "THIRD-PARTY.md").write_text("Dependency notices", encoding="utf-8")
        notices = package / "notices"
        notices.mkdir()
        (notices / "upstream-LICENSE").write_text("Upstream terms", encoding="utf-8")
        return package

    def test_copies_plugin_manifest_library_and_nested_notices_with_inventory_hashes(self):
        self.package()
        inventory = packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        self.assertEqual(len(inventory), 1)
        self.assertEqual(inventory[0]["id"], "midnight.viewer.basic")
        files = {item["path"]: item for item in inventory[0]["files"]}
        library = files["viewers/basic/viewer.dll"]
        self.assertEqual(library["sha256"], hashlib.sha256(b"independent native fixture").hexdigest())
        self.assertEqual(len(inventory[0]["package_sha256"]), 64)
        self.assertEqual((self.stage / "viewers/basic/notices/upstream-LICENSE").read_text(), "Upstream terms")
        archive = self.root / "release.zip"
        packaging.write_archive(self.stage, archive, "me.exe", "zip")
        with zipfile.ZipFile(archive) as bundle:
            self.assertEqual(bundle.read("release/viewers/basic/viewer.dll"), b"independent native fixture")

    def test_duplicate_id_target_mismatch_and_entry_escape_are_refused(self):
        for change in ("duplicate", "target", "escape"):
            with self.subTest(change=change):
                first = self.package(change, identity=f"viewer.{change}")
                if change == "duplicate":
                    self.package("duplicate-other", identity="viewer.duplicate")
                elif change == "target":
                    manifest = first / "plugin.toml"
                    manifest.write_text(manifest.read_text().replace(self.target, "wrong-target"))
                else:
                    manifest = first / "plugin.toml"
                    manifest.write_text(manifest.read_text().replace('entry_library="viewer.dll"', 'entry_library="../outside.dll"'))
                with self.assertRaises(ValueError):
                    packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
                for child in tuple(self.viewers.iterdir()):
                    import shutil
                    shutil.rmtree(child)

    def test_release_requires_all_selected_plugins_and_license_notices(self):
        package = self.package()
        with self.assertRaises(ValueError):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target,
                                          required_ids=("midnight.viewer.basic", "midnight.viewer.source"))
        (package / "LICENSE").unlink()
        with self.assertRaises(ValueError):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)

    def test_symlinked_package_content_and_zip_stage_are_rejected(self):
        package = self.package()
        outside = self.root / "outside.txt"
        outside.write_text("not part of this package")
        link = package / "notices" / "redirect"
        try:
            link.symlink_to(outside)
        except OSError as error:
            self.skipTest(f"OS does not permit fixture symlink creation: {error}")
        with self.assertRaises(ValueError):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        (self.stage / "redirect").symlink_to(outside)
        with self.assertRaises(ValueError):
            packaging.write_archive(self.stage, self.root / "unsafe.zip", "me.exe", "zip")

    @unittest.skipIf(os.name == "nt", "Windows cannot create a literal backslash filename")
    def test_unix_filename_cannot_become_a_windows_archive_path_escape(self):
        package = self.package()
        (package / "notices" / "..\\..\\..\\escape.txt").write_text("ambiguous archive path")
        with self.assertRaises(ValueError):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        (self.stage / "..\\escape.txt").write_text("ambiguous archive path")
        with self.assertRaises(ValueError):
            packaging.write_archive(self.stage, self.root / "unsafe-name.zip", "me.exe", "zip")


class IndependentPluginNoticeTests(unittest.TestCase):
    def test_basic_carries_notices_for_every_pinned_third_party_crate(self):
        root = Path(__file__).resolve().parents[2] / "plugins/midnight-viewer-basic"
        lock = tomllib.loads((root / "Cargo.lock").read_text())
        for package in lock["package"]:
            if package.get("source", "").startswith("registry+https://github.com/rust-lang/crates.io-index"):
                directory = root / "notices" / f'{package["name"]}-{package["version"]}'
                with self.subTest(dependency=directory.name):
                    self.assertTrue(directory.is_dir(), f"Missing independent package notices: {directory.name}")
                    self.assertTrue(any(path.is_file() and path.stat().st_size > 0 for path in directory.iterdir()))


if __name__ == "__main__":
    unittest.main()
