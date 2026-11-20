"""Thumbnail distributions retain pinned dependency terms and reject incomplete inputs."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/thumbnail-notices.py"
# Runs the command line in a child Python. Only Cargo is replaced: a process replays the
# fixture metadata's raw UTF-8 bytes, so the script's own decoding of the output is real.
REPLAYING_CARGO = """import importlib.util, pathlib, subprocess, sys
script, repository, *arguments = sys.argv[1:]
spec = importlib.util.spec_from_file_location("thumbnail_notices", script)
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)
run = subprocess.run
def cargo(command, **options):
    plugin = pathlib.Path(command[command.index("--manifest-path") + 1]).parent.name
    replay = "import sys; sys.stdout.buffer.write(open(sys.argv[1], 'rb').read())"
    return run([sys.executable, "-c", replay, str(pathlib.Path(repository) / (plugin + ".json"))], **options)
subprocess.run = cargo
notices.main(arguments, repository=pathlib.Path(repository))
"""


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
        # The fixture checkout; its folder leaves room for links that lead to it.
        self.owner = Path(self.temporary.name)
        self.root = self.owner / "checkout"
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

    def directory_link(self, kind, link, target):
        """A directory symlink, skipped where the OS forbids it, or a Windows junction."""
        target.mkdir(parents=True, exist_ok=True)
        link.parent.mkdir(parents=True, exist_ok=True)
        if kind == "junction":
            import _winapi  # A junction needs no symlink privilege.
            _winapi.CreateJunction(str(target), str(link))
            return link
        try:
            link.symlink_to(target, target_is_directory=True)
        except OSError as error:
            self.skipTest(f"OS does not permit fixture symlinks: {error}")
        return link

    def assert_links_above_and_at_crate_roots_are_accepted(self, kind):
        # Cargo metadata names crates as the build reaches them: below a linked cargo or home
        # directory (~/.cargo -> /data/cargo, /home -> /var/home), or in a registry crate
        # folder that is itself a link. Neither substitutes content from outside the crate.
        home = self.directory_link(kind, self.root / "home/.cargo", self.root / "disk/cargo")
        registry = Path("registry/src/index.crates.io-0000")
        self.dependency(self.package("codec", directory=home / registry / "codec-1.2.3", files={
            "LICENSE-MIT": "Codec terms\n", "LICENSES/Apache-2.0.txt": "Codec Apache terms\n"}))
        crate = self.directory_link(kind, self.root / "cargo" / registry / "crunchy-0.2.4",
                                    self.root / "disk/crunchy-0.2.4")
        self.dependency(self.package("crunchy", version="0.2.4", directory=crate,
                                     files={"LICENSE": "Crunchy terms\n"}))
        self.generate()
        licenses = self.output / "midnight-thumbnail-image/licenses"
        self.assertEqual((licenses / "codec-1.2.3/LICENSE-MIT").read_text(), "Codec terms\n")
        self.assertEqual((licenses / "codec-1.2.3/LICENSES/Apache-2.0.txt").read_text(),
                         "Codec Apache terms\n")
        self.assertEqual((licenses / "crunchy-0.2.4/LICENSE").read_text(), "Crunchy terms\n")

    def assert_linked_directory_inside_a_crate_is_rejected(self, kind):
        # The crate is reached through an accepted linked cargo home, but that tolerance ends
        # at the crate's own root: a link below it cannot copy unrelated content.
        home = self.directory_link(kind, self.root / "home/.cargo", self.root / "disk/cargo")
        crate = home / "registry/src/index.crates.io-0000/redirect-1.2.3"
        package = self.package("redirect", directory=crate, files={"LICENSE": "Redirect terms\n"})
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "MIT.txt").write_text("Unrelated private content\n", encoding="utf-8")
        self.directory_link(kind, crate / "LICENSES", outside)
        self.dependency(package)
        # Discovery refuses the link itself; it never lists what lies behind it.
        with self.assertRaisesRegex(ValueError, "link.*: .*LICENSES$"):
            self.generate()
        self.assertFalse(self.output.exists())

    def assert_checkout_reached_through_a_link_is_accepted(self, kind):
        # Fedora Silverblue's /home -> /var/home, a linked parent folder or a subst drive:
        # the build names the output through the link, main() the checkout by its resolved
        # path, and the output must still be recognised as below the checkout's _build.
        linked = self.directory_link(kind, self.owner / "linked checkout", self.root)
        self.notices.generate_notices(self.metadata, linked / "_build/notices", repository=self.root)
        readme = self.output / "midnight-thumbnail-image/README.md"
        self.assertEqual(readme.read_text(), "Documentation for midnight-thumbnail-image\n")
        # Everything named through the link, as when Cargo reports the linked spelling.
        for metadata in self.metadata.values():
            for package in metadata["packages"]:
                manifest = Path(package["manifest_path"])
                package["manifest_path"] = str(linked / manifest.relative_to(self.root))
        readme.unlink()
        self.notices.generate_notices(self.metadata, linked / "_build/notices", repository=linked)
        self.assertEqual(readme.read_text(), "Documentation for midnight-thumbnail-image\n")

    def assert_links_at_or_below_build_are_refused(self, kind):
        # _build and every folder below it must be real: a link there could redirect the
        # replacement of generated notices to unrelated content, even back into the checkout.
        outside = self.owner / "outside"
        outside.mkdir()
        (outside / "keep").write_text("user data\n", encoding="utf-8")
        build = self.root / "_build"
        self.directory_link(kind, build / "redirect", outside)
        self.directory_link(kind, build / "loop", self.root)
        for output in (build / "redirect/notices", build / "loop/_build/notices"):
            with self.subTest(output=output), self.assertRaisesRegex(ValueError, "link"):
                self.notices.generate_notices(self.metadata, output, repository=self.root)
        self.assertEqual([path.name for path in outside.iterdir()], ["keep"])
        for link in (build / "redirect", build / "loop"):
            if os.name == "nt":
                os.rmdir(link)  # Removes the junction or directory symlink, never its target.
            else:
                link.unlink()
        build.rmdir()
        self.directory_link(kind, build, outside)
        with self.assertRaisesRegex(ValueError, "link"):
            self.notices.generate_notices(self.metadata, self.output, repository=self.root)
        self.assertEqual([path.name for path in outside.iterdir()], ["keep"])

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

    def test_symlinks_above_or_at_a_crate_root_are_accepted(self):
        self.assert_links_above_and_at_crate_roots_are_accepted("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_junctions_above_or_at_a_crate_root_are_accepted(self):
        self.assert_links_above_and_at_crate_roots_are_accepted("junction")

    def test_symlinked_directory_inside_a_crate_is_rejected(self):
        self.assert_linked_directory_inside_a_crate_is_rejected("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_junction_inside_a_crate_is_rejected(self):
        self.assert_linked_directory_inside_a_crate_is_rejected("junction")

    def test_license_file_cannot_escape_its_crate(self):
        package = self.package("escape")
        unrelated = Path(package["manifest_path"]).parent.with_name("LICENSE")
        unrelated.write_text("Unrelated private content\n", encoding="utf-8")
        self.dependency(package)
        for license_file in ("../LICENSE", str(unrelated)):
            with self.subTest(license_file=license_file):
                package["license_file"] = license_file
                with self.assertRaisesRegex(ValueError, "escapes its package"):
                    self.generate()
                self.assertFalse(self.output.exists())

    def test_add_file_requires_the_package_root(self):
        # Only the caller knows where a package starts. A guessed root, such as the file's
        # own directory, would let a link inside the package pass as one above it.
        readme = self.root / "plugins/midnight-thumbnail-image/README.md"
        with self.assertRaises(TypeError):
            self.notices.add_file({}, readme, Path("README.md"))

    def test_nonportable_names_below_a_package_root_are_rejected_before_access(self):
        # add_file walks these names itself, whatever destination its caller chose. On
        # Windows 'Z:outside' would re-anchor the walk on drive Z: and 'LICENSE:notes' would
        # open a data stream of LICENSE.
        package = self.package("names", files={"LICENSE": "Names terms\n"})
        crate = Path(package["manifest_path"]).parent
        (crate / "x").mkdir()
        for member in (("x", "Z:outside", "LICENSE"), ("LICENSE:notes",)):
            source = Path(os.sep.join((str(crate), *member)))
            with self.subTest(member=member):
                with self.assertRaisesRegex(ValueError, "Nonportable.*'(Z|LICENSE):"):
                    self.notices.add_file({}, source, Path("README.md"), root=crate)

    def test_output_must_be_below_the_checkouts_own_build(self):
        (self.owner / "_build").mkdir()
        for output in (self.root / "outside", self.root / "_build", self.root / "_build/x/../notices",
                       self.owner / "_build/notices"):
            with self.subTest(output=output), self.assertRaisesRegex(
                    ValueError, "must be below the repository's _build|must not contain '\\.\\.'"):
                self.notices.generate_notices(self.metadata, output, repository=self.root)
        self.assertFalse((self.root / "outside").exists())
        self.assertEqual(list((self.owner / "_build").iterdir()), [])

    def test_checkout_reached_through_a_symlink_is_accepted(self):
        self.assert_checkout_reached_through_a_link_is_accepted("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_checkout_reached_through_a_junction_is_accepted(self):
        self.assert_checkout_reached_through_a_link_is_accepted("junction")

    def test_symlinks_at_or_below_build_are_refused(self):
        self.assert_links_at_or_below_build_are_refused("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_junctions_at_or_below_build_are_refused(self):
        self.assert_links_at_or_below_build_are_refused("junction")

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

    @unittest.skipUnless(os.name == "nt", "Python before 3.15 decodes text output in the Windows ANSI code page")
    def test_cli_decodes_cargo_metadata_below_a_non_ascii_path_as_utf8(self):
        # A crates.io dependency below a non-ASCII CARGO_HOME, such as a user profile. Cargo
        # writes UTF-8 JSON, which Python's pre-3.15 default (-X utf8=0) decoded as ANSI.
        home = self.root / "Jiří 文档/.cargo/registry/src/index.crates.io-0000"
        self.dependency(self.package("codec", directory=home / "codec-1.2.3",
                                     files={"LICENSE-MIT": "Codec terms\n"}))
        for plugin in self.plugins:
            (self.root / f"{plugin}.json").write_bytes(
                json.dumps(self.metadata[plugin], ensure_ascii=False).encode("utf-8"))
        config = self.root / "sdk.toml"
        config.write_text("# Fixture SDK config\n", encoding="utf-8")
        result = subprocess.run(
            [sys.executable, "-X", "utf8=0", "-B", "-c", REPLAYING_CARGO, str(SCRIPT), str(self.root),
             "--config", str(config), "--output", str(self.output)],
            capture_output=True, encoding="utf-8", errors="replace", timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        licenses = self.output / "midnight-thumbnail-image/licenses/codec-1.2.3"
        self.assertEqual((licenses / "LICENSE-MIT").read_text(encoding="utf-8"), "Codec terms\n")


if __name__ == "__main__":
    unittest.main()
