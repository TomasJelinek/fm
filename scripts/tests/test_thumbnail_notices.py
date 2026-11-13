"""Thumbnail distributions retain pinned dependency terms and reject incomplete inputs."""

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/thumbnail-notices.py"


class ThumbnailNoticeTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), "Thumbnail notice generator is missing")
        spec = importlib.util.spec_from_file_location("thumbnail_notices", SCRIPT)
        self.notices = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.notices)
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / "_build/notices"
        self.plugins = ("midnight-thumbnail-image", "midnight-thumbnail-video")
        self.metadata = {}
        for name in self.plugins:
            package = self.package(name, source=None, license=None,
                                   directory=self.root / "plugins" / name)
            Path(package["manifest_path"]).with_name("README.md").write_text(
                f"Documentation for {name}\n", encoding="utf-8")
            self.metadata[name] = {"packages": [package], "resolve": {
                "root": package["id"], "nodes": [{"id": package["id"], "features": []}]}}
        shared = self.root / "plugins/image-decoder/notices"
        (shared / "Noto-Sans").mkdir(parents=True)
        (shared / "Noto-Sans/OFL.txt").write_text("Font license\n", encoding="utf-8")
        (shared / "rust").mkdir()
        (shared / "rust/COPYRIGHT-library.html").write_text("Rust notices\n", encoding="utf-8")

    def package(self, name, *, version="1.2.3", source="registry+https://github.com/rust-lang/crates.io-index",
                license="MIT", directory=None, files=None):
        directory = directory or self.root / "sources" / f"{name}-{version}"
        directory.mkdir(parents=True, exist_ok=True)
        manifest = directory / "Cargo.toml"
        manifest.write_text(f'[package]\nname="{name}"\nversion="{version}"\n', encoding="utf-8")
        for relative, content in (files or {}).items():
            path = directory / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        return {"id": f"{name}@{version}", "name": name, "version": version,
                "source": source, "license": license, "license_file": None,
                "manifest_path": str(manifest), "repository": "https://example.invalid/source",
                "metadata": None}

    def dependency(self, package, *, plugin="midnight-thumbnail-image", features=()):
        self.metadata[plugin]["packages"].append(package)
        self.metadata[plugin]["resolve"]["nodes"].append(
            {"id": package["id"], "features": list(features)})

    def generate(self):
        self.notices.generate_notices(self.metadata, self.output, repository=self.root)

    def test_both_workers_keep_transitive_and_vendored_terms_features_and_existing_shared_notices(self):
        self.dependency(self.package("codec", files={"LICENSE-MIT": "Codec terms\n",
                                                     "NOTICE": "Codec attribution\n"}),
                        features=("png", "jpeg"))
        vendor = self.package("vendored", source=None, files={"LICENSE": "Vendored terms\n"})
        vendor["metadata"] = {"midnight": {"vendored-third-party": True}}
        self.dependency(vendor)
        helper = self.package("midnight-image-decoder", source=None, license="MIT",
                              directory=self.root / "plugins/image-decoder",
                              files={"LICENSE": "Shared decoder terms\n"})
        self.dependency(helper)
        self.generate()
        image = self.output / "midnight-thumbnail-image"
        self.assertEqual((image / "licenses/codec-1.2.3/LICENSE-MIT").read_text(), "Codec terms\n")
        self.assertEqual((image / "licenses/codec-1.2.3/NOTICE").read_text(), "Codec attribution\n")
        self.assertEqual((image / "licenses/vendored-1.2.3/LICENSE").read_text(), "Vendored terms\n")
        self.assertEqual((image / "licenses/midnight-image-decoder-1.2.3/LICENSE").read_text(),
                         "Shared decoder terms\n")
        self.assertEqual((image / "licenses/Noto-Sans/OFL.txt").read_text(), "Font license\n")
        inventory = json.loads((image / "THIRD-PARTY.json").read_text())
        codec = next(package for package in inventory if package["name"] == "codec")
        self.assertEqual(codec["features"], ["jpeg", "png"])
        self.assertEqual(codec["license"], "MIT")
        for plugin in self.plugins:
            directory = self.output / plugin
            self.assertEqual((directory / "README.md").read_text(), f"Documentation for {plugin}\n")
            self.assertEqual((directory / "licenses/rust/COPYRIGHT-library.html").read_text(),
                             "Rust notices\n")

    def test_missing_required_dependency_terms_fail_before_replacing_existing_notices(self):
        self.generate()
        previous = (self.output / "midnight-thumbnail-image/THIRD-PARTY.json").read_bytes()
        self.dependency(self.package("missing-terms"))
        with self.assertRaisesRegex(ValueError, "missing-terms-1.2.3"):
            self.generate()
        self.assertEqual((self.output / "midnight-thumbnail-image/THIRD-PARTY.json").read_bytes(), previous)

    def test_unknown_third_party_license_is_rejected_even_when_a_license_file_exists(self):
        self.dependency(self.package("unreviewed", license="UNKNOWN", files={"LICENSE": "Unreviewed\n"}))
        with self.assertRaisesRegex(ValueError, "[Uu]nknown.*unreviewed"):
            self.generate()
        self.assertFalse(self.output.exists())

    def test_ffmpeg_sys_missing_license_uses_only_the_exact_reviewed_version_and_declaration(self):
        package = self.package("ffmpeg-sys-next", version="9.0.1", license="WTFPL")
        self.dependency(package, plugin="midnight-thumbnail-video")
        overrides = self.root / "packaging/licenses/ffmpeg-sys-next-9.0.1"
        overrides.mkdir(parents=True)
        (overrides / "LICENSE.WTFPL").write_text("Canonical WTFPL text\n", encoding="utf-8")
        (overrides / "SOURCE.md").write_text("Pinned published declaration provenance\n", encoding="utf-8")
        self.generate()
        generated = self.output / "midnight-thumbnail-video/licenses/ffmpeg-sys-next-9.0.1"
        self.assertEqual((generated / "LICENSE.WTFPL").read_text(), "Canonical WTFPL text\n")
        self.assertEqual((generated / "SOURCE.md").read_text(), "Pinned published declaration provenance\n")
        self.assertEqual((generated / "Cargo.toml").read_bytes(), Path(package["manifest_path"]).read_bytes())
        package["license"] = "LicenseRef-Unreviewed"
        with self.assertRaisesRegex(ValueError, "ffmpeg-sys-next-9.0.1"):
            self.generate()

    def test_override_without_its_required_license_text_is_rejected(self):
        package = self.package("ffmpeg-sys-next", version="9.0.1", license="WTFPL")
        self.dependency(package, plugin="midnight-thumbnail-video")
        overrides = self.root / "packaging/licenses/ffmpeg-sys-next-9.0.1"
        overrides.mkdir(parents=True)
        (overrides / "SOURCE.md").write_text("Provenance without license text\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "ffmpeg-sys-next-9.0.1"):
            self.generate()

    def test_license_symlink_cannot_copy_unrelated_content(self):
        package = self.package("redirect")
        outside = self.root / "outside"
        outside.write_text("Unrelated private content\n", encoding="utf-8")
        try:
            Path(package["manifest_path"]).with_name("LICENSE").symlink_to(outside)
        except OSError as error:
            self.skipTest(f"OS does not permit fixture symlinks: {error}")
        self.dependency(package)
        with self.assertRaises(ValueError):
            self.generate()
        self.assertFalse(self.output.exists())

    def test_output_must_be_in_build_and_must_not_traverse_symlinks(self):
        with self.assertRaisesRegex(ValueError, "_build"):
            self.notices.generate_notices(self.metadata, self.root / "outside", repository=self.root)
        outside = self.root / "outside"
        outside.mkdir()
        (self.root / "_build").mkdir()
        try:
            (self.root / "_build/redirect").symlink_to(outside, target_is_directory=True)
        except OSError as error:
            self.skipTest(f"OS does not permit fixture symlinks: {error}")
        with self.assertRaises(ValueError):
            self.notices.generate_notices(self.metadata, self.root / "_build/redirect/notices",
                                          repository=self.root)
        self.assertEqual(list(outside.iterdir()), [])

    def test_cli_uses_locked_offline_metadata_and_preserves_cargo_failure(self):
        config = self.root / "sdk.toml"
        config.write_text("# Fixture SDK config\n", encoding="utf-8")
        responses = [subprocess.CompletedProcess([], 0, stdout=json.dumps(self.metadata[plugin]))
                     for plugin in self.plugins]
        with mock.patch.object(self.notices.subprocess, "run", side_effect=responses) as run:
            self.notices.main(["--config", str(config), "--output", str(self.output)], repository=self.root)
        self.assertTrue((self.output / "midnight-thumbnail-video/THIRD-PARTY.json").is_file())
        self.assertEqual(run.call_count, 2)
        for call, plugin in zip(run.call_args_list, self.plugins):
            arguments = call.args[0]
            self.assertEqual(arguments[0:2], ["cargo", "metadata"])
            for argument in ("--locked", "--offline", str(config),
                             str(self.root / "plugins" / plugin / "Cargo.toml")):
                self.assertIn(argument, arguments)
        with mock.patch.object(self.notices.subprocess, "run",
                               side_effect=subprocess.CalledProcessError(47, ["cargo", "metadata"])):
            with self.assertRaises(subprocess.CalledProcessError) as failure:
                self.notices.main(["--config", str(config), "--output", str(self.output)], repository=self.root)
        self.assertEqual(failure.exception.returncode, 47)


if __name__ == "__main__":
    unittest.main()
