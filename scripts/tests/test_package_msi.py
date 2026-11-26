"""The Windows installer: payload checks, database plan and, on Windows, a built MSI."""
import hashlib
import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
spec = importlib.util.spec_from_file_location("package_msi", ROOT / "scripts/package-msi.py")
package_msi = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package_msi)

VERSION = package_msi.workspace_version()
ROOT_NAME = f"midnight-explorer-{VERSION}-x86_64-pc-windows-msvc"
EXECUTABLE = b"MZ stand-in executable"


def release_files(version=VERSION, **changes):
    files = {
        "midnight-explorer.exe": EXECUTABLE,
        "midnight-elevated-listing.exe": b"MZ helper",
        "README.txt": (f"Midnight Explorer {version} for Windows x64\n"
                       f"Source: https://github.com/TomasJelinek/fm/tree/v{version}\n").encode(),
        "LICENSE.txt": (ROOT / "LICENSE").read_bytes(),
        "THIRD-PARTY.txt": b"notices\n",
        "BUILD-INFO.json": json.dumps({
            "version": version, "target": "x86_64-pc-windows-msvc", "binary": "midnight-explorer.exe",
            "binary_sha256": hashlib.sha256(EXECUTABLE).hexdigest()}).encode(),
        "viewers/basic/viewer.json": b"{}",
        "viewers/basic/notices/licenses/a-very-long-crate-name-1.2.3/LICENSE-APACHE": b"apache",
        "viewers/basic/notices/licenses/a-very-long-crate-name-1.2.4/LICENSE-APACHE": b"apache 2",
        "thumbnailers/README.md": b"readme",
        "thumbnailers/readme.MD.txt": b"other",
    }
    files.update(changes)
    return {name: content for name, content in files.items() if content is not None}


def write_release(folder, files, version=VERSION, root=None):
    root = root or f"midnight-explorer-{version}-x86_64-pc-windows-msvc"
    archive = Path(folder) / f"{root}.zip"
    with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED) as bundle:
        for name, content in sorted(files.items()):
            info = zipfile.ZipInfo(f"{root}/{name}")
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            bundle.writestr(info, content)
    return archive


class Workspace(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(dir=temporary)
        self.folder = Path(self.owner.name)

    def tearDown(self):
        self.owner.cleanup()


class Payload(Workspace):
    def test_reads_the_release_payload_without_build_info(self):
        payload = package_msi.read_payload(write_release(self.folder, release_files()),
                                           expected_version=VERSION)
        names = [relative for relative, _ in payload.files]
        self.assertEqual(names, sorted(names))
        self.assertNotIn("BUILD-INFO.json", names)
        self.assertEqual(set(names), set(release_files()) - {"BUILD-INFO.json"})
        self.assertEqual(payload.version, VERSION)
        self.assertEqual(payload.root, ROOT_NAME)

    def test_refuses_a_release_that_does_not_match(self):
        other = "0.0.1" if VERSION != "0.0.1" else "0.0.2"
        cases = {
            "workspace version": (release_files(other), other, None),
            "executable hash": (release_files(**{"midnight-explorer.exe": b"MZ changed"}), VERSION, None),
            "README version": (release_files(**{"README.txt": b"Midnight Explorer 9.9.9 x\n"}), VERSION, None),
            "license": (release_files(**{"LICENSE.txt": b"MIT"}), VERSION, None),
            "missing license": (release_files(**{"LICENSE.txt": None}), VERSION, None),
            "missing README": (release_files(**{"README.txt": None}), VERSION, None),
            "missing helper": (release_files(**{"midnight-elevated-listing.exe": None}), VERSION, None),
            "missing build info": (release_files(**{"BUILD-INFO.json": None}), VERSION, None),
            "entry outside the root": (release_files(), VERSION, "other-root"),
            "non-ASCII name": (release_files(**{"licenses/café.txt": b"x"}), VERSION, None),
            "reserved name": (release_files(**{"viewers/NUL.txt": b"x"}), VERSION, None),
            "case duplicate": (release_files(**{"readme.txt": b"x"}), VERSION, None),
            "path too long": (release_files(**{"licenses/" + "x" * 160: b"x"}), VERSION, None),
        }
        for label, (files, version, root) in cases.items():
            with self.subTest(label), tempfile.TemporaryDirectory(dir=self.folder) as folder:
                archive = write_release(folder, files, version)
                if root:
                    archive = Path(folder) / f"midnight-explorer-{version}-x86_64-pc-windows-msvc.zip"
                    archive.unlink()
                    with zipfile.ZipFile(archive, "x") as bundle:
                        for name, content in files.items():
                            bundle.writestr(f"{root}/{name}", content)
                with self.assertRaises(ValueError):
                    package_msi.read_payload(archive, expected_version=VERSION)

    def test_refuses_names_and_versions_windows_installer_cannot_hold(self):
        for name in ("midnight-explorer-1.2.3-x86_64-unknown-linux-gnu.zip",
                     "midnight-explorer-1.2.3-beta.1-x86_64-pc-windows-msvc.zip",
                     "midnight-explorer-256.0.0-x86_64-pc-windows-msvc.zip",
                     "midnight-explorer-0.1.65536-x86_64-pc-windows-msvc.zip"):
            with self.subTest(name), self.assertRaises(ValueError):
                package_msi.read_payload(self.folder / name)

    def test_refuses_folder_entries_and_links(self):
        for attribute in (0o040755 << 16, 0o120777 << 16):
            with self.subTest(attribute=oct(attribute)), tempfile.TemporaryDirectory(dir=self.folder) as folder:
                archive = write_release(folder, release_files())
                with zipfile.ZipFile(archive, "a") as bundle:
                    info = zipfile.ZipInfo(f"{ROOT_NAME}/viewers/link")
                    info.external_attr = attribute
                    bundle.writestr(info, b"target")
                with self.assertRaises(ValueError):
                    package_msi.read_payload(archive)


class Plan(Workspace):
    def plan(self, files=None):
        payload = package_msi.read_payload(write_release(self.folder, files or release_files()))
        return payload, package_msi.plan(payload)

    def test_installs_per_user_below_the_profile_with_a_start_menu_shortcut(self):
        payload, tables = self.plan()
        directories = {row[0]: row for row in tables["Directory"]}
        self.assertEqual(directories["INSTALLDIR"][1:], ("UserProgramsFolder", "MIDNIG~1|Midnight Explorer (MSI)"))
        self.assertEqual(directories["UserProgramsFolder"][1], "LocalAppDataFolder")
        properties = dict(tables["Property"])
        self.assertNotIn("ALLUSERS", properties)
        self.assertEqual(properties["Manufacturer"], "Tomas Jelinek")
        self.assertEqual(properties["ProductVersion"], VERSION)
        self.assertEqual(properties["UpgradeCode"], package_msi.UPGRADE_CODE)
        self.assertEqual(properties["ARPURLINFOABOUT"], f"https://github.com/TomasJelinek/fm/tree/v{VERSION}")
        (shortcut,) = tables["Shortcut"]
        self.assertEqual(shortcut[1:5], ("ProgramMenuFolder", "MIDNIG~1|Midnight Explorer", "StartMenuShortcut",
                                         "[#MainExecutable]"))
        self.assertEqual(shortcut[11], "MEUSERPROFILE")
        self.assertIn("NOT ALLUSERS", dict(tables["LaunchCondition"]))

    def test_every_file_of_the_payload_once_in_sequence(self):
        payload, tables = self.plan()
        files = tables["File"]
        self.assertEqual([row[7] for row in files], list(range(1, len(files) + 1)))
        self.assertEqual(sorted(row[8] for row in files), [relative for relative, _ in payload.files])
        self.assertEqual(tables["Media"][0][1], len(files))
        self.assertEqual(len({row[0] for row in files}), len(files))
        self.assertEqual(package_msi.installed_paths(tables["Directory"], tables["Component"], files),
                         {row[0]: row[8] for row in files})

    def test_profile_components_use_registry_key_paths_and_remove_their_folders(self):
        _, tables = self.plan()
        registry = {row[0]: row for row in tables["Registry"]}
        for component in tables["Component"]:
            self.assertTrue(component[3] & package_msi.COMPONENT_REGISTRY_KEY_PATH)
            self.assertEqual(registry[component[5]][1], package_msi.HKCU)
            self.assertRegex(component[1], r"^\{[0-9A-F-]{36}\}$")
        removed = {row[3] for row in tables["RemoveFile"]}
        created = {row[0] for row in tables["Directory"]} - {"TARGETDIR", "LocalAppDataFolder", "ProgramMenuFolder"}
        self.assertEqual(created - removed, set())
        self.assertTrue(all(row[2] is None and row[4] == 2 for row in tables["RemoveFile"]))

    def test_short_names_are_unique_per_folder(self):
        _, tables = self.plan()
        for name in ("Directory", "File"):
            seen = {}
            for row in tables[name]:
                short = row[2].split("|")[0]
                if short in (".", "SourceDir"):
                    continue
                self.assertRegex(short, r"^[A-Z0-9_~!#$%&'()@^{}-]{1,8}(?:\.[A-Z0-9_-]{1,3})?$")
                self.assertNotIn(short, seen.setdefault(row[1], set()))
                seen[row[1]].add(short)
        taken = set()
        self.assertEqual(package_msi.short_name("README.txt", taken), "README.TXT")
        self.assertEqual(package_msi.short_name("readme.txt", taken), "README~1.TXT")
        self.assertEqual(package_msi.short_name("THIRD-PARTY.json", taken), "THIRD-~1.JSO")
        self.assertEqual(package_msi.short_name(".hidden", taken), "HIDDEN~1")

    def test_codes_are_stable_for_a_release_and_change_with_it(self):
        self.assertEqual(package_msi.product_code("1.2.3"), package_msi.product_code("1.2.3"))
        self.assertNotEqual(package_msi.product_code("1.2.3"), package_msi.product_code("1.2.4"))
        self.assertNotEqual(package_msi.package_code("1.2.3", "a"), package_msi.package_code("1.2.3", "b"))
        _, first = self.plan()
        self.owner.cleanup()
        self.setUp()
        _, second = self.plan()
        self.assertEqual(first, second)

    def test_upgrades_replace_older_versions_and_refuse_downgrades(self):
        _, tables = self.plan()
        upgrades = {row[6]: row for row in tables["Upgrade"]}
        self.assertEqual(upgrades["ME_DOWNGRADE_DETECTED"][1:5], (VERSION, None, None, package_msi.UPGRADE_ONLY_DETECT))
        self.assertEqual(upgrades["ME_UPGRADE_DETECTED"][1:5], (None, VERSION, None, package_msi.UPGRADE_MIGRATE_FEATURES))
        conditions = dict(tables["LaunchCondition"])
        self.assertIn("NOT ME_DOWNGRADE_DETECTED", conditions)
        execute = dict((row[0], row[2]) for row in tables["InstallExecuteSequence"])
        self.assertEqual(execute["RemoveExistingProducts"], execute["InstallValidate"] + 1)

    def test_custom_actions_only_set_properties(self):
        _, tables = self.plan()
        self.assertTrue(tables["CustomAction"])
        self.assertTrue(all(row[1] == 51 for row in tables["CustomAction"]))
        self.assertNotIn("Binary", package_msi.TABLES)


@unittest.skipUnless(os.name == "nt", "the MSI is built with the Windows Installer API")
class WindowsInstaller(Workspace):
    def test_builds_and_verifies_an_msi_from_a_release_zip(self):
        archive = write_release(self.folder, release_files())
        out = io.StringIO()
        msi = package_msi.build(archive, out=out)
        self.assertEqual(msi.name, f"{ROOT_NAME}.msi")
        self.assertEqual(out.getvalue().strip(), str(msi))
        self.assertEqual(sorted(entry.name for entry in self.folder.iterdir()), sorted([archive.name, msi.name]))
        package_msi.verify(msi, archive, out=out)
        import msi_database
        with msi_database.Database(msi) as database:
            summary = database.summary()
            self.assertEqual(summary[msi_database.PID_WORDCOUNT], 10)
            self.assertEqual(summary[msi_database.PID_TEMPLATE], "x64;1033")
            properties = dict(database.rows("SELECT `Property`, `Value` FROM `Property`"))
            self.assertEqual(properties["ProductCode"], package_msi.product_code(VERSION))
            names = {row[0] for row in database.rows("SELECT `FileName` FROM `File`")}
            self.assertIn("LICENSE.TXT|LICENSE.txt", names)
        with self.assertRaisesRegex(ValueError, "exists"):
            package_msi.build(archive)

    def test_verify_refuses_an_msi_of_another_payload(self):
        archive = write_release(self.folder, release_files())
        msi = package_msi.build(archive, out=io.StringIO())
        other = self.folder / "other"
        other.mkdir()
        changed = write_release(other, release_files(**{"thumbnailers/README.md": b"changed"}))
        renamed = other / msi.name
        os.replace(msi, renamed)
        with self.assertRaises(ValueError):
            package_msi.verify(renamed, changed, out=io.StringIO())


if __name__ == "__main__":
    unittest.main()
