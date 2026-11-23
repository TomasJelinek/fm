"""Regression checks for the release dependency inventory."""
import importlib.util
import hashlib
import json
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


def directory_link(test, kind, link, target):
    """A directory symlink, skipped where the OS forbids it, or a Windows junction."""
    target.mkdir(parents=True, exist_ok=True)
    link.parent.mkdir(parents=True, exist_ok=True)
    if kind == "junction":
        import _winapi  # A junction needs no symlink privilege.
        _winapi.CreateJunction(str(target), str(link))
    else:
        try:
            link.symlink_to(target, target_is_directory=True)
        except OSError as error:
            test.skipTest(f"OS does not permit fixture symlinks: {error}")
    return link


def remove_link(link):
    if os.name == "nt":
        os.rmdir(link)  # Removes the junction or directory symlink, never its target.
    else:
        link.unlink()


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

    def test_vendored_renderer_keeps_distribution_notices(self):
        self.metadata["packages"][0]["metadata"] = None
        vendor = package("renderer", None)
        vendor["metadata"] = {"midnight": {"vendored-third-party": True}}
        self.metadata["packages"].append(vendor)
        result = packaging.dependency_packages(
            self.metadata, "midnight-explorer v0.1.0\nrenderer v0.1.0\n")
        self.assertEqual([entry["name"] for entry in result], ["renderer"])

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

    def test_inventory_records_package_authors_for_the_about_dialog(self):
        temporary = Path(__file__).resolve().parents[2] / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary) as directory:
            stage = Path(directory) / "release"
            stage.mkdir()
            rust_docs = Path(directory) / "rust"
            rust_docs.mkdir()
            (rust_docs / "COPYRIGHT-library.html").write_text("Rust fixture", encoding="utf-8")
            crate = Path(directory) / "demo"
            crate.mkdir()
            (crate / "LICENSE-MIT").write_text("Copyright (c) 2020 Ada", encoding="utf-8")
            demo = dict(package("demo"), manifest_path=str(crate / "Cargo.toml"),
                        license="MIT", authors=["Ada <ada@example.org>", "Grace"])
            packaging.add_notices(stage, {"packages": [package("midnight-explorer", None), demo]},
                                  "midnight-explorer v0.1.0\ndemo v0.1.0\n", rust_docs, None)
            inventory = json.loads((stage / "THIRD-PARTY.json").read_text(encoding="utf-8"))
            self.assertEqual([(entry["name"], entry["authors"]) for entry in inventory],
                             [("demo", ["Ada <ada@example.org>", "Grace"])])
            self.assertTrue((stage / "licenses/demo-0.1.0/LICENSE-MIT").is_file())

    def test_archive_modes_do_not_inherit_world_writable_staging_permissions(self):
        temporary = Path(__file__).resolve().parents[2] / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary) as directory:
            folder = Path(directory)
            stage = folder / "release"
            stage.mkdir()
            for name in ("midnight-explorer", "README.txt"):
                path = stage / name
                path.write_text("fixture", encoding="utf-8")
                path.chmod(0o777)
            for archive_format in ("tar.gz", "zip"):
                with self.subTest(format=archive_format):
                    archive = folder / f"release.{archive_format}"
                    packaging.write_archive(stage, archive, "midnight-explorer", archive_format)
                    if archive_format == "tar.gz":
                        with tarfile.open(archive) as bundle:
                            self.assertEqual(bundle.getmember("release/midnight-explorer").mode, 0o755)
                            self.assertEqual(bundle.getmember("release/README.txt").mode, 0o644)
                            self.assertEqual(bundle.getmember("release").mode, 0o755)
                            self.assertEqual(bundle.getmember("release/midnight-explorer").uid, 0)
                    else:
                        with zipfile.ZipFile(archive) as bundle:
                            self.assertEqual(bundle.getinfo("release/midnight-explorer").external_attr >> 16 & 0o777, 0o755)
                            self.assertEqual(bundle.getinfo("release/README.txt").external_attr >> 16 & 0o777, 0o644)

    def test_linux_packages_carry_the_html_sandbox_installer_as_an_executable(self):
        root = Path(__file__).resolve().parents[2]
        temporary = root / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary) as directory:
            folder = Path(directory)
            stage = folder / "release"
            stage.mkdir()
            (stage / "midnight-explorer").write_text("fixture", encoding="utf-8")
            self.assertIsNone(packaging.copy_html_sandbox_installer(stage, root, "x86_64-pc-windows-msvc"))
            self.assertFalse((stage / packaging.HTML_SANDBOX_INSTALLER).exists())
            installer = packaging.copy_html_sandbox_installer(stage, root, "x86_64-unknown-linux-gnu")
            self.assertEqual(installer, "install-html-viewer-apparmor.sh")
            self.assertEqual((stage / installer).read_bytes(),
                             (root / "scripts" / installer).read_bytes())
            archive = folder / "release.tar.gz"
            packaging.write_archive(stage, archive, "midnight-explorer", "tar.gz", [installer])
            with tarfile.open(archive) as bundle:
                self.assertEqual(bundle.getmember(f"release/{installer}").mode, 0o755)


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
        packaging.write_archive(self.stage, archive, "midnight-explorer.exe", "zip")
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
            packaging.write_archive(self.stage, self.root / "unsafe.zip", "midnight-explorer.exe", "zip")

    def assert_packages_below_a_linked_folder_are_accepted(self, kind):
        # A checkout reached through a link (/home -> /var/home, a linked parent folder):
        # only the package roots and what lies below them must be real.
        self.package()
        linked = directory_link(self, kind, self.root / "linked", self.root)
        inventory = packaging.copy_viewer_packages(self.stage, linked / "input-viewers", self.target)
        self.assertEqual(inventory[0]["id"], "midnight.viewer.basic")
        self.assertEqual((self.stage / "viewers/basic/notices/upstream-LICENSE").read_text(), "Upstream terms")
        # The staging check compares resolved paths, whichever side the link spells.
        with self.assertRaisesRegex(ValueError, "staging"):
            packaging.copy_viewer_packages(self.viewers / "basic", linked / "input-viewers", self.target)

    def test_packages_below_a_symlinked_folder_are_accepted(self):
        self.assert_packages_below_a_linked_folder_are_accepted("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_packages_below_a_junction_are_accepted(self):
        self.assert_packages_below_a_linked_folder_are_accepted("junction")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_junction_inside_a_package_or_as_its_root_is_refused(self):
        package = self.package()
        outside = self.root / "outside"
        (outside / "nested").mkdir(parents=True)
        (outside / "nested/private").write_text("not part of this package")
        redirect = directory_link(self, "junction", package / "notices/redirect", outside)
        with self.assertRaisesRegex(ValueError, "reparse"):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        remove_link(redirect)
        moved = self.root / "moved-viewers"
        self.viewers.rename(moved)
        directory_link(self, "junction", self.viewers, moved)
        self.addCleanup(remove_link, self.viewers)
        with self.assertRaisesRegex(ValueError, "reparse"):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        self.assertEqual(list(self.stage.iterdir()), [])

    @unittest.skipIf(os.name == "nt", "Windows cannot create a literal backslash filename")
    def test_unix_filename_cannot_become_a_windows_archive_path_escape(self):
        package = self.package()
        (package / "notices" / "..\\..\\..\\escape.txt").write_text("ambiguous archive path")
        with self.assertRaises(ValueError):
            packaging.copy_viewer_packages(self.stage, self.viewers, self.target)
        (self.stage / "..\\escape.txt").write_text("ambiguous archive path")
        with self.assertRaises(ValueError):
            packaging.write_archive(self.stage, self.root / "unsafe-name.zip", "midnight-explorer.exe", "zip")


class ReleaseLinkRuleTests(unittest.TestCase):
    """Folders above a package root or the checkout may be links; nothing below them."""

    def setUp(self):
        temporary = Path(__file__).resolve().parents[2] / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(owner.cleanup)
        self.owner = Path(owner.name)
        self.checkout = self.owner / "checkout"
        self.checkout.mkdir()
        self.rust_docs = self.owner / "rust"
        self.rust_docs.mkdir()
        (self.rust_docs / "COPYRIGHT-library.html").write_text("Rust fixture", encoding="utf-8")
        self.outside = self.owner / "outside"
        self.outside.mkdir()
        (self.outside / "MIT.txt").write_text("Unrelated private content", encoding="utf-8")
        self.stages = []

    def staged_names(self):
        return {path.name for stage in self.stages for path in stage.rglob("*")}

    def crate(self, directory, name="codec", files=None):
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n', encoding="utf-8")
        for relative, content in (files or {}).items():
            (directory / relative).parent.mkdir(parents=True, exist_ok=True)
            (directory / relative).write_text(content, encoding="utf-8")
        return {**package(name), "license": "MIT", "manifest_path": str(directory / "Cargo.toml")}

    def notices(self, *crates, overrides=None):
        stage = Path(tempfile.mkdtemp(prefix="stage ", dir=self.owner))
        self.stages.append(stage)
        tree = "midnight-explorer v0.1.0\n" + "".join(f'{crate["name"]} v0.1.0\n' for crate in crates)
        packaging.add_notices(stage, {"packages": [package("midnight-explorer", None), *crates]}, tree,
                              self.rust_docs, overrides)
        return stage / "licenses"

    def assert_linked_registry_and_crate_root_are_accepted(self, kind):
        # A linked cargo home (~/.cargo -> /data/cargo, /home -> /var/home) and a registry
        # crate folder that is itself a link substitute nothing from outside the crate.
        home = directory_link(self, kind, self.owner / "home/.cargo", self.owner / "disk/cargo")
        codec = self.crate(home / "registry/src/index-0000/codec-0.1.0", files={
            "LICENSE-MIT": "Codec terms", "LICENSES/Apache-2.0.txt": "Codec Apache terms"})
        crunchy = directory_link(self, kind, self.owner / "cargo/registry/src/index-0000/crunchy-0.1.0",
                                 self.owner / "disk/crunchy")
        licenses = self.notices(codec, self.crate(crunchy, "crunchy", {"LICENSE": "Crunchy terms"}))
        self.assertEqual((licenses / "codec-0.1.0/LICENSE-MIT").read_text(), "Codec terms")
        self.assertEqual((licenses / "codec-0.1.0/LICENSES/Apache-2.0.txt").read_text(), "Codec Apache terms")
        self.assertEqual((licenses / "crunchy-0.1.0/LICENSE").read_text(), "Crunchy terms")
        self.assertEqual((licenses / "rust/COPYRIGHT-library.html").read_text(), "Rust fixture")

    def test_symlinked_registry_and_crate_root_are_accepted(self):
        self.assert_linked_registry_and_crate_root_are_accepted("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_junction_registry_and_crate_root_are_accepted(self):
        self.assert_linked_registry_and_crate_root_are_accepted("junction")

    def assert_links_inside_a_crate_or_override_are_refused(self, kind):
        crate = self.owner / "registry/redirect-0.1.0"
        redirect = self.crate(crate, "redirect", {"LICENSE": "Redirect terms"})
        directory_link(self, kind, crate / "LICENSES", self.outside)
        with self.assertRaisesRegex(ValueError, "link.*LICENSES"):
            self.notices(redirect)
        # A crate without notices takes them from the reviewed override tree, which is a
        # package input too.
        overrides = self.owner / "overrides"
        bare = self.crate(self.owner / "registry/bare-0.1.0", "bare")
        (overrides / "bare-0.1.0").mkdir(parents=True)
        (overrides / "bare-0.1.0/LICENSE").write_text("Reviewed terms", encoding="utf-8")
        self.assertEqual((self.notices(bare, overrides=overrides) / "bare-0.1.0/LICENSE").read_text(),
                         "Reviewed terms")
        directory_link(self, kind, overrides / "bare-0.1.0/upstream", self.outside)
        with self.assertRaisesRegex(ValueError, "link.*upstream"):
            self.notices(bare, overrides=overrides)
        self.assertNotIn("MIT.txt", self.staged_names())

    def test_symlinks_inside_a_crate_or_override_are_refused(self):
        self.assert_links_inside_a_crate_or_override_are_refused("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_junctions_inside_a_crate_or_override_are_refused(self):
        self.assert_links_inside_a_crate_or_override_are_refused("junction")

    def test_symlinked_license_file_is_refused(self):
        crate = self.owner / "registry/redirect-0.1.0"
        redirect = self.crate(crate, "redirect")
        try:
            (crate / "LICENSE").symlink_to(self.outside / "MIT.txt")
        except OSError as error:
            self.skipTest(f"OS does not permit fixture symlinks: {error}")
        with self.assertRaisesRegex(ValueError, "link.*LICENSE"):
            self.notices(redirect)

    def test_declared_license_file_cannot_escape_its_crate(self):
        crate = self.owner / "registry/escape-0.1.0"
        escape = self.crate(crate, "escape", {"LICENSE": "Escape terms"})
        (crate.parent / "LICENSE").write_text("Unrelated private content", encoding="utf-8")
        for license_file in ("../LICENSE", str(self.outside / "MIT.txt")):
            with self.subTest(license_file=license_file):
                escape["license_file"] = license_file
                with self.assertRaisesRegex(ValueError, "escapes its package"):
                    self.notices(escape)

    def assert_build_output_rule(self, kind):
        build = self.checkout / "_build"
        resolved = self.checkout.resolve()
        # The checkout named through a link, and a _build that does not exist yet.
        linked = directory_link(self, kind, self.owner / "linked checkout", self.checkout)
        output = linked / "_build/releases/v1"
        self.assertEqual(packaging.build_output(output, resolved), output)
        self.assertEqual(packaging.build_output(build / "releases", linked), build / "releases")
        for refused in (self.checkout / "releases", build, build / "x/../releases", self.owner / "_build/x"):
            with self.subTest(output=refused), self.assertRaisesRegex(
                    ValueError, "must be below the repository's _build|must not contain '\\.\\.'"):
                packaging.build_output(refused, resolved)
        # A link at or below _build, including one that leads back into the checkout.
        directory_link(self, kind, build / "redirect", self.outside)
        directory_link(self, kind, build / "loop", self.checkout)
        for refused in (build / "redirect/releases", build / "loop/_build/releases"):
            with self.subTest(output=refused), self.assertRaisesRegex(ValueError, "link"):
                packaging.build_output(refused, resolved)
        for link in (build / "redirect", build / "loop"):
            remove_link(link)
        build.rmdir()
        directory_link(self, kind, build, self.outside)
        with self.assertRaisesRegex(ValueError, "link"):
            packaging.build_output(build / "releases", resolved)

    def test_build_output_allows_a_symlinked_checkout_but_no_link_from_build_down(self):
        self.assert_build_output_rule("symlink")

    @unittest.skipUnless(os.name == "nt", "Junctions are Windows directory reparse points")
    def test_build_output_allows_a_junction_checkout_but_no_link_from_build_down(self):
        self.assert_build_output_rule("junction")


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


class AuthorizationHelperPackageTests(unittest.TestCase):
    def test_exact_sibling_helper_is_required_copied_and_hashed(self):
        root = Path(__file__).resolve().parents[2] / "_build" / "tmp"
        root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=root) as temporary:
            source = Path(temporary) / "source"
            stage = Path(temporary) / "stage"
            source.mkdir()
            stage.mkdir()
            with self.assertRaises(FileNotFoundError):
                packaging.copy_authorization_helper(stage, source / "midnight-explorer.exe", "x86_64-pc-windows-msvc")
            helper = source / "midnight-elevated-listing.exe"
            helper.write_bytes(b"owned test helper")
            record = packaging.copy_authorization_helper(stage, source / "midnight-explorer.exe", "x86_64-pc-windows-msvc")
            self.assertEqual((stage / helper.name).read_bytes(), helper.read_bytes())
            self.assertEqual(record["sha256"], hashlib.sha256(helper.read_bytes()).hexdigest())
            self.assertTrue(record["executable"])


class ProjectLicenseTests(unittest.TestCase):
    """Every archive carries the program's GPL text, the project README and the source."""

    def setUp(self):
        self.root = Path(__file__).resolve().parents[2]
        temporary = self.root / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.version = tomllib.loads((self.root / "Cargo.toml").read_text())["workspace"]["package"]["version"]

    def test_the_repository_license_is_the_complete_gpl_version_3(self):
        text = (self.root / "LICENSE").read_text(encoding="utf-8")
        self.assertTrue(text.lstrip().startswith("GNU GENERAL PUBLIC LICENSE"))
        self.assertIn("Version 3, 29 June 2007", text)
        self.assertIn("END OF TERMS AND CONDITIONS", text)
        self.assertEqual(len(text.splitlines()), 674)
        manifest = tomllib.loads((self.root / "Cargo.toml").read_text())
        self.assertEqual(manifest["workspace"]["package"]["license"], "GPL-3.0-or-later")

    def test_plugins_and_api_workspaces_carry_the_same_license(self):
        licence = (self.root / "LICENSE").read_bytes()
        plugins = sorted(path.parent for path in (self.root / "plugins").glob("*/Cargo.toml"))
        self.assertGreaterEqual(len(plugins), 15)
        for plugin in plugins:
            with self.subTest(plugin=plugin.name):
                self.assertEqual((plugin / "LICENSE").read_bytes(), licence)
                package = tomllib.loads((plugin / "Cargo.toml").read_text(encoding="utf-8"))["package"]
                self.assertEqual(package.get("license"), "GPL-3.0-or-later")
        for workspace in ("api", "filesystem-api", "image-viewer-api", "thumbnail-api",
                          "web-viewer-api", "worker-api"):
            with self.subTest(workspace=workspace):
                self.assertEqual((self.root / workspace / "LICENSE").read_bytes(), licence)

    def test_windows_names_the_license_txt_and_linux_keeps_license(self):
        for target, name, absent in (("x86_64-pc-windows-msvc", "LICENSE.txt", "LICENSE"),
                                     ("x86_64-unknown-linux-gnu", "LICENSE", "LICENSE.txt")):
            with self.subTest(target=target):
                stage = Path(self.temporary.name) / target
                stage.mkdir()
                packaging.copy_project_documents(stage, self.root, target)
                self.assertEqual(packaging.project_license_name(target), name)
                self.assertEqual((stage / name).read_bytes(), (self.root / "LICENSE").read_bytes())
                self.assertFalse((stage / absent).exists())
                self.assertEqual((stage / "README.md").read_bytes(), (self.root / "README.md").read_bytes())
                for relative in packaging.PROJECT_DOCUMENTS:
                    self.assertTrue((stage / Path(relative).name).is_file(), relative)
                archive_format = "zip" if name == "LICENSE.txt" else "tar.gz"
                (stage / "midnight-explorer").write_text("fixture", encoding="utf-8")
                archive = Path(self.temporary.name) / f"{target}.{archive_format}"
                packaging.write_archive(stage, archive, "midnight-explorer", archive_format)
                if archive_format == "zip":
                    with zipfile.ZipFile(archive) as bundle:
                        names = set(bundle.namelist())
                else:
                    with tarfile.open(archive) as bundle:
                        names = set(bundle.getnames())
                self.assertIn(f"{target}/{name}", names)
                self.assertIn(f"{target}/README.md", names)
                self.assertNotIn(f"{target}/{absent}", names)

    def test_release_readmes_name_the_license_and_the_tagged_source(self):
        source = f"https://github.com/TomasJelinek/fm/tree/v{self.version}"
        for platform, license_name in (("windows", "LICENSE.txt"), ("linux", "LICENSE")):
            with self.subTest(platform=platform):
                readme = (self.root / "packaging" / platform / "README.txt").read_text(encoding="utf-8")
                packaging.check_release_readme(readme, self.version)
                self.assertIn(source, readme)
                self.assertIn(f"see {license_name}.", readme)
                self.assertIn("The plugins are licensed under the same terms.", readme)
                self.assertNotIn("MIT", readme)
                self.assertIn("GNU General Public License, version 3", readme)
        with self.assertRaises(ValueError):
            packaging.check_release_readme(f"Midnight Explorer {self.version} - fixture\n", self.version)
        with self.assertRaises(ValueError):
            packaging.check_release_readme(f"Midnight Explorer 0.0.0 - fixture\n{source}\n", self.version)

    def test_third_party_notice_states_the_program_and_slint_licenses(self):
        stage = Path(self.temporary.name) / "release"
        stage.mkdir()
        rust_docs = Path(self.temporary.name) / "rust"
        rust_docs.mkdir()
        (rust_docs / "COPYRIGHT-library.html").write_text("Rust fixture", encoding="utf-8")
        packaging.add_notices(stage, {"packages": [package("midnight-explorer", None)]},
                              "midnight-explorer v0.1.0\n", rust_docs, None)
        notice = (stage / "THIRD-PARTY.txt").read_text(encoding="utf-8")
        self.assertIn("Midnight Explorer and its plugins are licensed under GPL-3.0-or-later; "
                      "see the LICENSE file", notice)
        self.assertIn("Slint's GPL-3.0-only option", notice)
        self.assertNotIn("do not assign", notice)


if __name__ == "__main__":
    unittest.main()
