"""The Windows installer: payload checks, database plan and, on Windows, a built MSI."""
import hashlib
import importlib.util
import io
import json
import os
import shutil
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

    def test_dual_purpose_per_user_by_default_or_for_all_users(self):
        payload, tables = self.plan()
        directories = {row[0]: row for row in tables["Directory"]}
        # Per machine: Program Files. Per user: the earlier per-user MSI's folder, set first.
        self.assertEqual(directories["INSTALLDIR"][1:], ("ProgramFiles64Folder", "MIDNIG~1|Midnight Explorer (MSI)"))
        self.assertNotIn("LocalAppDataFolder", directories)
        actions = {row[0]: row for row in tables["CustomAction"]}
        self.assertEqual(actions["SetUserInstallFolder"][2:],
                         ("INSTALLDIR", "[LocalAppDataFolder]Programs\\Midnight Explorer (MSI)\\"))
        execute = {row[0]: row for row in tables["InstallExecuteSequence"]}
        self.assertEqual(execute["SetUserInstallFolder"][1:],
                         (f"NOT INSTALLDIR AND ({package_msi.PER_USER})", execute["CostFinalize"][2] - 10))
        self.assertEqual(execute["SetShortcutWorkingFolder"][1], package_msi.PER_USER)
        # All users: the Windows drive's root. Windows Installer expands a profile variable
        # when it creates the shortcut, which would start every account in the installer's.
        self.assertEqual(execute["SetMachineShortcutWorkingFolder"][1], f"NOT ({package_msi.PER_USER})")
        self.assertEqual(actions["SetShortcutWorkingFolder"][2:], ("MESTARTFOLDER", "[%USERPROFILE]"))
        self.assertEqual(actions["SetMachineShortcutWorkingFolder"][2:], ("MESTARTFOLDER", "[WindowsVolume]"))
        properties = dict(tables["Property"])
        self.assertEqual((properties["ALLUSERS"], properties["MSIINSTALLPERUSER"]), ("2", "1"))
        self.assertNotIn("MESTARTFOLDER", properties)
        self.assertEqual(properties["Manufacturer"], "Tomas Jelinek")
        self.assertEqual(properties["ProductVersion"], VERSION)
        # The 0.6.1 per-user MSI's upgrade code, so that a per-user installation upgrades it.
        self.assertEqual(properties["UpgradeCode"], "{DDF2A2BA-6901-484C-97B2-4EF662BF5E72}")
        self.assertEqual(properties["ARPURLINFOABOUT"], f"https://github.com/TomasJelinek/fm/tree/v{VERSION}")
        (shortcut,) = tables["Shortcut"]
        self.assertEqual(shortcut[1:5], ("ProgramMenuFolder", "MIDNIG~1|Midnight Explorer", "StartMenuShortcut",
                                         "[#MainExecutable]"))
        self.assertEqual(shortcut[11], "MESTARTFOLDER")
        self.assertNotIn("NOT ALLUSERS", dict(tables["LaunchCondition"]))
        package_msi.check_scope_rules(tables)

    def test_the_scope_dialog_sets_the_scope_and_folder_of_its_choice(self):
        _, tables = self.plan()
        events = {}
        for dialog, control, event, argument, condition, order in tables["ControlEvent"]:
            events.setdefault((dialog, control), []).append((order, event, argument, condition))
        for button in ("InstallUser", "InstallMachine"):
            self.assertEqual([row[1:] for row in sorted(events[("ScopeDlg", button)])], [
                ("[ALLUSERS]", "2", "1"),
                ("[MSIINSTALLPERUSER]", "1", 'ME_SCOPE = "user"'),
                ("[MSIINSTALLPERUSER]", "{}", 'ME_SCOPE = "machine"'),
                ("[INSTALLDIR]", "[LocalAppDataFolder]Programs\\Midnight Explorer (MSI)\\", 'ME_SCOPE = "user"'),
                ("[INSTALLDIR]", "[%ProgramW6432]\\Midnight Explorer (MSI)\\", 'ME_SCOPE = "machine"'),
                ("EndDialog", "Return", "1")])
        controls = {(row[0], row[1]): row for row in tables["Control"]}
        self.assertTrue(controls[("ScopeDlg", "InstallMachine")][7] & package_msi.ELEVATION_SHIELD)
        self.assertEqual(controls[("ScopeDlg", "InstallMachine")][3:7], controls[("ScopeDlg", "InstallUser")][3:7])
        self.assertEqual(controls[("ScopeDlg", "Scope")][8], "ME_SCOPE")
        radio = {(row[0], row[2]) for row in tables["RadioButton"]}
        self.assertEqual(radio, {("ME_SCOPE", "user"), ("ME_SCOPE", "machine"),
                                 ("ME_MAINTENANCE", "Repair"), ("ME_MAINTENANCE", "Remove")})
        ui = {row[0]: row for row in tables["InstallUISequence"]}
        self.assertEqual(ui["ScopeDlg"][1], "NOT Installed")
        self.assertEqual([ui[name][2] for name in ("ExitDlg", "UserExitDlg", "FatalErrorDlg")], [-1, -2, -3])
        self.assertLess(ui["AppSearch"][2], ui["SetMachineScope"][2])
        self.assertLess(ui["SetMachineScope"][2], ui["ScopeDlg"][2])
        dialogs = {row[0] for row in tables["Dialog"]}
        self.assertTrue({row[0] for row in tables["Control"]} <= dialogs)
        # Every dialog is shown by the sequence, except the one the progress dialog's Cancel spawns.
        self.assertEqual(dialogs - set(ui), {"CancelDlg"})

    def test_one_scope_at_a_time(self):
        # Copies in both scopes would share component codes, so the other scope's copy
        # (found in HKCU or HKLM, also that of the 0.6.1 per-user MSI) refuses a scope.
        _, tables = self.plan()
        locators = {row[0]: row[1:] for row in tables["RegLocator"]}
        searches = dict(tables["AppSearch"])
        key = "Software\\Midnight Explorer Installer"
        self.assertEqual(locators[searches["ME_USER_COPY"]], (package_msi.HKCU, key, "RootFolder", 18))
        self.assertEqual(locators[searches["ME_MACHINE_COPY"]], (package_msi.HKLM, key, "RootFolder", 18))
        properties = dict(tables["Property"])
        self.assertEqual(properties["SecureCustomProperties"].split(";"),
                         ["ME_DOWNGRADE_DETECTED", "ME_UPGRADE_DETECTED", "ME_USER_COPY", "ME_MACHINE_COPY"])
        # Silent installations: launch conditions, applied only where SetScopeRule runs, in
        # the execute sequence after AppSearch. The UI sequence evaluates launch conditions
        # before ScopeDlg, with the default scope, so there the rule must not apply.
        conditions = dict(tables["LaunchCondition"])
        self.assertEqual(conditions[package_msi.OTHER_SCOPE_CONDITIONS["user"]],
                         "Midnight Explorer is already installed for all users. Remove it in Settings > Apps > "
                         "Installed apps before installing it just for you.")
        self.assertEqual(conditions[package_msi.OTHER_SCOPE_CONDITIONS["machine"]],
                         "Midnight Explorer is already installed just for you. Remove it in Settings > Apps > "
                         "Installed apps before installing it for all users.")
        for condition in package_msi.OTHER_SCOPE_CONDITIONS.values():
            self.assertTrue(condition.startswith("Installed OR NOT ME_SCOPE_RULE OR "), condition)
            self.assertLessEqual(len(condition), 255)
        execute = {row[0]: row for row in tables["InstallExecuteSequence"]}
        ui = {row[0]: row for row in tables["InstallUISequence"]}
        self.assertLess(execute["AppSearch"][2], execute["SetScopeRule"][2])
        self.assertLess(execute["SetScopeRule"][2], execute["LaunchConditions"][2])
        self.assertIsNone(execute["SetScopeRule"][1])
        self.assertNotIn("SetScopeRule", ui)
        self.assertEqual({row[0]: row for row in tables["CustomAction"]}["SetScopeRule"],
                         ("SetScopeRule", 51, "ME_SCOPE_RULE", "1"))
        # The dialog: the conflicting choice's Install button is disabled, and its note says why.
        rules = {(row[1], row[2]): row[3] for row in tables["ControlCondition"] if row[0] == "ScopeDlg"}
        self.assertEqual(rules[("InstallUser", "Disable")], "ME_MACHINE_COPY")
        self.assertEqual(rules[("InstallUser", "Enable")], "NOT ME_MACHINE_COPY")
        self.assertEqual(rules[("InstallMachine", "Disable")], "ME_USER_COPY")
        self.assertEqual(rules[("InstallMachine", "Enable")], "NOT ME_USER_COPY")
        notes = {row[1]: row[9] for row in tables["Control"] if row[0] == "ScopeDlg"}
        self.assertEqual(notes["MachineCopyNote"], conditions[package_msi.OTHER_SCOPE_CONDITIONS["user"]])
        self.assertEqual(rules[("MachineCopyNote", "Show")], 'ME_SCOPE = "user" AND ME_MACHINE_COPY')
        self.assertEqual(notes["UserCopyNote"], conditions[package_msi.OTHER_SCOPE_CONDITIONS["machine"]])
        self.assertEqual(rules[("UserCopyNote", "Show")], 'ME_SCOPE = "machine" AND ME_USER_COPY')
        # Every case the build rehearses with Windows Installer: refused only in the execute
        # sequence of a new installation while the other scope has a copy.
        cases = package_msi.scope_rule_cases()
        self.assertEqual(len(cases), 4 * 4 * 2 * 2)
        refused = {(scope, copies) for scope, _, copies, execute, maintenance, refusal in cases if refusal}
        self.assertEqual(refused, {("user", ("machine",)), ("user", ("user", "machine")),
                                   ("machine", ("user",)), ("machine", ("user", "machine"))})
        self.assertTrue(all(execute and not maintenance for _, _, _, execute, maintenance, refusal in cases if refusal))

    def test_rules_refuse_a_package_of_one_scope_only(self):
        _, tables = self.plan()
        cases = {
            "per user only": ("Property", lambda rows: [row for row in rows if row[0] != "ALLUSERS"]),
            "per machine only": ("Property", lambda rows: [row for row in rows if row[0] != "MSIINSTALLPERUSER"]),
            "HKCU key path": ("Registry", lambda rows: [(row[0], package_msi.HKCU, *row[2:]) for row in rows]),
            "other upgrade code": ("Property", lambda rows: [
                (name, "{00000000-0000-0000-0000-000000000000}" if name == "UpgradeCode" else value)
                for name, value in rows]),
            "code-running action": ("CustomAction", lambda rows: rows + [("Run", 34, "INSTALLDIR", "x.exe")]),
            "profile folder": ("Directory", lambda rows: rows + [("LocalAppDataFolder", "TARGETDIR", ".")]),
            "no machine folder": ("ControlEvent", lambda rows: [row for row in rows if "ProgramW6432" not in row[3]]),
            # The application's updater finds an installed copy by its InstallLocation.
            "no install location": ("CustomAction", lambda rows: [
                row for row in rows if row[0] != "SetInstallLocation"]),
            "install location not run": ("InstallExecuteSequence", lambda rows: [
                row for row in rows if row[0] != "SetInstallLocation"]),
            # One scope at a time.
            "other scope not refused": ("LaunchCondition", lambda rows: [
                row for row in rows if row[0] != package_msi.OTHER_SCOPE_CONDITIONS["machine"]]),
            "scope rule before the dialog": ("InstallUISequence", lambda rows: rows + [("SetScopeRule", None, 60)]),
            "scope rule never set": ("InstallExecuteSequence", lambda rows: [
                row for row in rows if row[0] != "SetScopeRule"]),
            "no search before the rule": ("InstallExecuteSequence", lambda rows: [
                row for row in rows if row[0] != "AppSearch"]),
            "other key searched": ("RegLocator", lambda rows: [
                (row[0], row[1], "Software\\Other", *row[3:]) for row in rows]),
            "dialog installs beside a copy": ("ControlCondition", lambda rows: [
                row for row in rows if row[2] != "Disable"]),
        }
        for label, (table, change) in cases.items():
            with self.subTest(label):
                changed = dict(tables, **{table: change(list(tables[table]))})
                with self.assertRaises(ValueError):
                    package_msi.check_scope_rules(changed)

    def test_every_file_of_the_payload_once_in_sequence(self):
        payload, tables = self.plan()
        files = tables["File"]
        self.assertEqual([row[7] for row in files], list(range(1, len(files) + 1)))
        self.assertEqual(sorted(row[8] for row in files), [relative for relative, _ in payload.files])
        self.assertEqual(tables["Media"][0][1], len(files))
        self.assertEqual(len({row[0] for row in files}), len(files))
        self.assertEqual(package_msi.installed_paths(tables["Directory"], tables["Component"], files),
                         {row[0]: row[8] for row in files})

    def test_components_use_scoped_registry_key_paths_and_remove_their_folders(self):
        _, tables = self.plan()
        registry = {row[0]: row for row in tables["Registry"]}
        for component in tables["Component"]:
            self.assertTrue(component[3] & package_msi.COMPONENT_REGISTRY_KEY_PATH)
            # HKCU per user, HKLM per machine.
            self.assertEqual(registry[component[5]][1], -1)
            self.assertRegex(component[1], r"^\{[0-9A-F-]{36}\}$")
        removed = {row[3] for row in tables["RemoveFile"]}
        created = {row[0] for row in tables["Directory"]} - {"TARGETDIR", "ProgramFiles64Folder", "ProgramMenuFolder"}
        self.assertEqual(created, removed)
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

    def test_verify_runs_the_actions_before_the_first_dialog(self):
        # 0.6.2's MSI lacked the Signature table that AppSearch reads: Windows Installer
        # stopped with error 2228 before its first dialog.
        archive = write_release(self.folder, release_files())
        msi = package_msi.build(archive, out=io.StringIO())
        import msi_database
        with msi_database.Database(msi, update=True) as database:
            database.execute("DROP TABLE `Signature`")
            database.commit()
        with self.assertRaisesRegex(ValueError, "cannot start an installation: Action AppSearch"):
            package_msi.verify(msi, archive, out=io.StringIO())

    def test_windows_installer_refuses_only_the_other_scope(self):
        # The build rehearses the launch conditions with Windows Installer's own evaluator;
        # a package whose conditions refuse too little or too much fails it.
        archive = write_release(self.folder, release_files())
        msi = package_msi.build(archive, out=io.StringIO())
        import msi_database
        user = package_msi.OTHER_SCOPE_CONDITIONS["user"]
        for label, condition, error in (
                ("other scope installs", "VersionNT64", "is not refused"),
                ("refused before the dialog", user.replace("NOT ME_SCOPE_RULE OR ", ""), "UI sequence is refused"),
                ("upgrade refused", user.replace("ME_MACHINE_COPY", "ME_USER_COPY"), "is refused")):
            with self.subTest(label):
                changed = self.folder / f"{label}.msi"
                shutil.copyfile(msi, changed)
                with msi_database.Database(changed, update=True) as database:
                    database.execute(f"DELETE FROM `LaunchCondition` WHERE `Condition` = '{user}'")
                    if condition != "VersionNT64":
                        database.insert("LaunchCondition", [(condition, package_msi.OTHER_SCOPE_REFUSALS["user"])])
                    database.commit()
                with self.assertRaisesRegex(ValueError, error):
                    package_msi.rehearse_scope_rule(changed)
        package_msi.rehearse_scope_rule(msi)

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
