"""The MSI installation test (scripts/test-msi-install.py): its plan, arguments and checks.

The installations themselves run only on CI's disposable Windows runner; these tests cover
the logic that decides what to run and whether a result passes, and, on Windows, the
release-shaped stand-in ZIP, the two MSIs it prepares and the dry-run plan.
"""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import struct
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
spec = importlib.util.spec_from_file_location("msi_install_test", ROOT / "scripts/test-msi-install.py")
msi_test = importlib.util.module_from_spec(spec)
spec.loader.exec_module(msi_test)

ENVIRONMENT = {"LOCALAPPDATA": r"C:\Users\me\AppData\Local", "APPDATA": r"C:\Users\me\AppData\Roaming",
               "ProgramW6432": r"C:\Program Files", "ProgramData": r"C:\ProgramData",
               "USERPROFILE": r"C:\Users\me"}


def shell_link(target=None, working=None, arguments=None, environment_target=None):
    """A minimal [MS-SHLLINK] file: LinkInfo with a Unicode local path, Unicode strings."""
    flags = 0x80
    body = b""
    if target:
        flags |= 0x2
        base = target.encode("utf-16-le") + b"\0\0"
        header = 0x24
        ansi_base = header
        ansi_suffix = ansi_base + 1
        unicode_base = ansi_suffix + 1
        unicode_suffix = unicode_base + len(base)
        info = struct.pack("<9I", 0, header, 0x1, 0, ansi_base, 0, ansi_suffix, unicode_base, unicode_suffix)
        info += b"\0" + b"\0" + base + b"\0\0"
        body += struct.pack("<I", len(info)) + info[4:]
    for flag, text in ((0x10, working), (0x20, arguments)):
        if text is not None:
            flags |= flag
            body += struct.pack("<H", len(text)) + text.encode("utf-16-le")
    if environment_target:
        wide = environment_target.encode("utf-16-le").ljust(520, b"\0")
        body += struct.pack("<2I", 0x314, 0xA0000001) + b"\0" * 260 + wide
    body += struct.pack("<I", 0)
    header = struct.pack("<I16sI", 0x4C, b"\0" * 16, flags) + b"\0" * (76 - 24)
    return header + body


class Logic(unittest.TestCase):
    def test_refuses_to_change_a_machine_that_is_not_declared_disposable(self):
        with self.assertRaisesRegex(msi_test.Failure, "--disposable-machine"):
            msi_test.refuse_unless_disposable(False, dry_run=False)
        msi_test.refuse_unless_disposable(True, dry_run=False)
        msi_test.refuse_unless_disposable(False, dry_run=True)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
            msi_test.main(["run", "--work", str(ROOT / "_build" / "never-made")], out=io.StringIO())
        self.assertIn("--disposable-machine", str(raised.exception))

    def test_upgrades_from_the_published_msis_older_than_the_workspace(self):
        self.assertEqual(msi_test.previous_versions("0.6.2"), ["0.6.1"])
        self.assertEqual(msi_test.previous_versions("0.7.0"), ["0.6.1", "0.6.2"])
        self.assertEqual(msi_test.previous_versions("0.6.1"), [])
        self.assertLess(msi_test.parse_version(msi_test.OLDER_VERSION), (0, 6, 1))

    def test_silent_msiexec_arguments_choose_the_scope_and_keep_a_verbose_log(self):
        self.assertEqual(msi_test.msiexec_arguments("install", "a.msi", "a.log", "user"),
                         ["/i", "a.msi", "/qn", "/norestart", "/l*v", "a.log", "ALLUSERS=2", "MSIINSTALLPERUSER=1"])
        self.assertEqual(msi_test.msiexec_arguments("install", "a.msi", "a.log", "machine")[-1], "ALLUSERS=1")
        self.assertEqual(msi_test.msiexec_arguments("install", "a.msi", "a.log")[-1], "a.log")
        self.assertEqual(msi_test.msiexec_arguments("repair", "a.msi", "a.log")[:3], ["/fomus", "a.msi", "/qn"])
        self.assertEqual(msi_test.msiexec_arguments("uninstall", "a.msi", "a.log")[0], "/x")
        with self.assertRaises(ValueError):
            msi_test.msiexec_arguments("uninstall", "a.msi", "a.log", "user")
        with self.assertRaises(ValueError):
            msi_test.msiexec_arguments("install", "a.msi", "a.log", "everyone")

    def test_a_downgrade_is_refused_by_the_launch_condition(self):
        text = f"MSI (c) ... {msi_test.DOWNGRADE_TEXT}. Uninstall it first to install this one."
        for data in (b"\xff\xfe" + text.encode("utf-16-le"), text.encode("cp1252"), b"\xef\xbb\xbf" + text.encode()):
            self.assertTrue(msi_test.downgrade_refused(1603, msi_test.decode_log(data)))
        self.assertFalse(msi_test.downgrade_refused(0, text))
        self.assertFalse(msi_test.downgrade_refused(1603, "Error 2228"))

    def test_installed_files_must_equal_the_payload_exactly(self):
        expected = {"a.exe": "1", "sub/b.txt": "2"}
        self.assertEqual(msi_test.compare_files(expected, dict(expected)), [])
        self.assertEqual(msi_test.compare_files(expected, {"a.exe": "1", "sub/b.txt": "3", "BUILD-INFO.json": "4"}),
                         ["not in the release payload: BUILD-INFO.json", "differs from the release payload: sub/b.txt"])
        self.assertEqual(msi_test.compare_files(expected, {"a.exe": "1"}), ["missing sub/b.txt"])

    def test_installed_apps_entry_names_the_product_version_publisher_folder_and_icon(self):
        folder = Path(r"C:\Program Files\Midnight Explorer (MSI)")
        values = {"DisplayName": "Midnight Explorer", "DisplayVersion": "0.6.2", "Publisher": "Tomas Jelinek",
                  "WindowsInstaller": 1, "NoModify": 1, "InstallLocation": str(folder) + "\\",
                  "DisplayIcon": r"C:\Windows\Installer\{X}\ProductIcon.ico"}
        self.assertEqual(msi_test.check_uninstall_entry(values, "0.6.2", folder, lambda path: True), [])
        problems = msi_test.check_uninstall_entry(dict(values, DisplayVersion="0.0.1", NoModify=None), "0.6.2",
                                                  folder, lambda path: False)
        self.assertEqual(len(problems), 3, problems)
        reduced = {name: values[name] for name in ("DisplayName", "DisplayVersion", "Publisher",
                                                   "InstallLocation", "DisplayIcon")}
        self.assertEqual(msi_test.check_uninstall_entry(reduced, "0.6.2", folder, lambda path: True,
                                                        registry=False), [])

    def test_shortcut_target_and_start_folder_follow_the_scope(self):
        target = r"C:\Users\me\AppData\Local\Programs\Midnight Explorer (MSI)\midnight-explorer.exe"
        info = msi_test.parse_shortcut(shell_link(target, working=r"C:\Users\me"))
        self.assertEqual(info, {"target": target, "working_folder": r"C:\Users\me", "arguments": None})
        user = msi_test.expected_working_folder("user", ENVIRONMENT)
        self.assertEqual(msi_test.check_shortcut(info, Path(target), user), [])
        machine = msi_test.expected_working_folder("machine", ENVIRONMENT)
        self.assertEqual(machine, "%USERPROFILE%")
        self.assertEqual(len(msi_test.check_shortcut(info, Path(target), machine)), 1)
        literal = msi_test.parse_shortcut(shell_link(target, working="%USERPROFILE%", arguments="--x"))
        self.assertEqual(len(msi_test.check_shortcut(literal, Path(target), machine)), 1)
        variable = msi_test.parse_shortcut(shell_link(environment_target=r"%ProgramFiles%\a.exe"))
        self.assertEqual(variable["target"], r"%ProgramFiles%\a.exe")
        with self.assertRaises(ValueError):
            msi_test.parse_shortcut(b"not a link")

    def test_scope_folders_and_shortcuts(self):
        self.assertEqual(msi_test.install_folder("user", ENVIRONMENT),
                         Path(ENVIRONMENT["LOCALAPPDATA"]) / "Programs" / "Midnight Explorer (MSI)")
        self.assertEqual(msi_test.install_folder("machine", ENVIRONMENT).name, "Midnight Explorer (MSI)")
        self.assertIn("ProgramData", str(msi_test.shortcut_path("machine", ENVIRONMENT)))
        self.assertEqual(msi_test.shortcut_path("user", ENVIRONMENT).name, "Midnight Explorer.lnk")

    def test_only_administrators_may_change_the_all_users_folder(self):
        users, everyone = "S-1-5-32-545", "S-1-1-0"
        baseline = [{"Path": "C:\\Program Files", "Rules": [
            {"Sid": users, "Rights": -1610612736, "Allow": True, "InheritOnly": True},
            {"Sid": "S-1-5-32-544", "Rights": 268435456, "Allow": True, "InheritOnly": True}]}]
        installed = [{"Path": "C:\\Program Files\\Midnight Explorer (MSI)", "Rules": [
            {"Sid": users, "Rights": 1179817, "Allow": True, "InheritOnly": False},       # read and execute
            {"Sid": "S-1-5-18", "Rights": 2032127, "Allow": True, "InheritOnly": False},  # SYSTEM full control
            {"Sid": "S-1-3-0", "Rights": 268435456, "Allow": True, "InheritOnly": True},  # CREATOR OWNER
            {"Sid": users, "Rights": -1610612736, "Allow": True, "InheritOnly": True}]}]
        self.assertEqual(msi_test.write_grants(installed, baseline), [])
        changed = [{"Path": "x", "Rules": [{"Sid": users, "Rights": 1245631, "Allow": True, "InheritOnly": False},
                                           {"Sid": everyone, "Rights": 0x10000000, "Allow": True, "InheritOnly": True},
                                           {"Sid": everyone, "Rights": 2032127, "Allow": False, "InheritOnly": False}]}]
        self.assertEqual(len(msi_test.write_grants(changed, baseline)), 2)
        # What Program Files itself grants (here to Users) is not an addition.
        users_write = [{"Path": "x", "Rules": changed[0]["Rules"][:1]}]
        self.assertEqual(msi_test.write_grants(users_write, [{"Path": "p", "Rules": changed[0]["Rules"][:1]}]), [])

    def test_ice_errors_beyond_the_recorded_findings_fail(self):
        messages = [
            "ICE03\t1\tTable: File Column: File Missing specifications in _Validation Table (or Old Database)\tice03.html",
            "ICE57\t1\tComponent 'StartMenuShortcut' has both per-user data and a keypath that can be either",
            "ICE38\t1\tComponent X installs to user profile.\tice38.html",
            "ICE61\t2\tThis product should remove only older versions of itself.",
            "ICE99\t0\tICE99 failed to run",
            "ICE01\t3\tCreated 05/21/98.",
            "Action start 12:00:00: ICE01.",
        ]
        unexpected, recorded, warnings = msi_test.ice_findings(messages)
        self.assertEqual([item[0] for item in unexpected], ["ICE38", "ICE99"])
        self.assertEqual([item[0] for item in recorded], ["ICE03", "ICE57"])
        self.assertEqual([item[0] for item in warnings], ["ICE61"])
        self.assertEqual(msi_test.known_counts(recorded),
                         {("ICE03", "Missing specifications in _Validation Table"): 1,
                          ("ICE57", "Component 'StartMenuShortcut' has both per-user data and a keypath"): 1})
        self.assertIsNone(msi_test.parse_ice_message("ICE01\t7\tunknown type"))

    def test_older_readme_names_its_own_version_and_source(self):
        text = "Midnight Explorer 0.6.2 - Windows x64\nhttps://github.com/TomasJelinek/fm/tree/v0.6.2\n"
        self.assertEqual(msi_test.rewrite_version(text, "0.6.2", "0.0.1"),
                         "Midnight Explorer 0.0.1 - Windows x64\nhttps://github.com/TomasJelinek/fm/tree/v0.0.1\n")


class Plan(unittest.TestCase):
    def simulate(self, steps):
        """Installed (package, scope) pairs after each step, as Windows Installer would leave them."""
        installed = set()
        for operation, details in steps:
            package, scope = details.get("package"), details.get("scope")
            if operation in ("install", "ui-install"):
                scope = scope or "user"  # the published MSIs are per user
                replaced = {item for item in installed if item[1] == scope}
                self.assertTrue(all(item[0] != "new" for item in replaced) or package == "new",
                                f"{package} would be a downgrade, not an install")
                installed = (installed - replaced) | {(package, scope)}
            elif operation in ("repair", "uninstall"):
                scopes = {item[1] for item in installed if item[0] == package}
                self.assertEqual(len(scopes), 1, f"{operation} of {package} is ambiguous: {installed}")
                if operation == "uninstall":
                    installed = {item for item in installed if item[0] != package}
            elif operation == "uninstall-one":
                self.assertTrue(installed)
                installed.discard(sorted(item for item in installed if item[0] == package)[0])
            elif operation in ("check-installed", "check-side-by-side"):
                wanted = [(package, scope)] if operation == "check-installed" else [("new", "user"), ("new", "machine")]
                for item in wanted:
                    self.assertIn(item, installed, f"{operation} {item} after {installed}")
            elif operation == "check-removed":
                self.assertFalse({item for item in installed if item[1] == scope})
            elif operation == "check-nothing-installed":
                self.assertEqual(installed, set())
        return installed

    def test_every_scenario_ends_with_nothing_installed(self):
        for scenarios in (["scopes"], ["previous"], ["ui"], ["scopes", "previous", "ui"]):
            with self.subTest(scenarios=scenarios):
                steps = msi_test.plan(scenarios, "0.7.0", ["0.6.1", "0.6.2"])
                self.assertEqual(steps[:2], [("preflight", {}), ("marker", {})])
                self.assertEqual(steps[-1], ("check-nothing-installed", {}))
                self.assertEqual(self.simulate(steps), set())

    def test_scopes_cover_upgrade_downgrade_repair_permissions_and_side_by_side(self):
        steps = msi_test.plan(["scopes"], "0.6.2", [], ui_smoke=True)
        operations = [operation for operation, _ in steps]
        for scope in msi_test.SCOPES:
            for operation in ("install", "install-refused", "repair", "smoke"):
                self.assertIn((operation, scope), [(o, d.get("scope")) for o, d in steps])
        self.assertIn(("smoke", {"scope": "machine", "ui": True}), steps)
        self.assertEqual(operations.count("check-permissions"), 1)
        self.assertEqual(operations.count("uninstall-one"), 2)
        self.assertLess(operations.index("check-side-by-side"), operations.index("uninstall-one"))
        self.assertIn("check-marker", operations[operations.index("uninstall-one"):])
        # Every installation and repair is checked before the next step changes anything.
        for index, (operation, details) in enumerate(steps):
            if operation in ("install", "repair") and details.get("package") != "older":
                self.assertTrue(steps[index + 1][0].startswith("check-"), steps[index:index + 2])

    def test_published_msis_are_upgraded_in_the_per_user_folder(self):
        steps = msi_test.plan(["previous"], "0.7.0", ["0.6.1", "0.6.2"])
        installs = [details["package"] for operation, details in steps if operation == "install"]
        self.assertEqual(installs, ["previous-0.6.1", "new", "previous-0.6.2", "new"])
        self.assertNotIn("ui-install", [operation for operation, _ in steps])


@unittest.skipUnless(os.name == "nt", "The MSI is built with the Windows Installer API")
class Prepared(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build" / "tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(dir=temporary)
        self.folder = Path(self.owner.name)

    def tearDown(self):
        self.owner.cleanup()

    def test_stand_in_prepare_and_dry_run(self):
        inputs = self.folder / "in"
        (inputs / "viewers/midnight-viewer-basic").mkdir(parents=True)
        (inputs / "filesystems/native").mkdir(parents=True)
        (inputs / "me.exe").write_bytes(b"MZ explorer")
        (inputs / "helper.exe").write_bytes(b"MZ helper")
        (inputs / "viewers/midnight-viewer-basic/viewer.json").write_text("{}")
        (inputs / "filesystems/native/provider.json").write_text("{}")
        out = io.StringIO()
        archive = msi_test.stand_in(inputs / "me.exe", inputs / "helper.exe", inputs / "viewers",
                                    inputs / "filesystems", self.folder / "zip", out=out)
        payload, hashes = msi_test.payload_hashes(archive)
        self.assertEqual(payload.version, msi_test.package_msi.workspace_version())
        self.assertEqual(hashes["midnight-explorer.exe"], hashlib.sha256(b"MZ explorer").hexdigest())
        self.assertIn("viewers/midnight-viewer-basic/viewer.json", hashes)
        self.assertIn("filesystems/native/provider.json", hashes)
        with zipfile.ZipFile(archive) as bundle:
            info = json.loads(bundle.read(f"{payload.root}/BUILD-INFO.json"))
        self.assertEqual(info["binary_sha256"], hashlib.sha256(b"MZ explorer").hexdigest())
        with self.assertRaisesRegex(msi_test.Failure, "exists"):
            msi_test.stand_in(inputs / "me.exe", inputs / "helper.exe", inputs / "viewers",
                              inputs / "filesystems", self.folder / "zip", out=out)

        manifest = msi_test.prepare(archive, self.folder / "work", out=out)
        older_payload, older_hashes = msi_test.payload_hashes(manifest["older"]["zip"])
        self.assertEqual(older_payload.version, msi_test.OLDER_VERSION)
        self.assertEqual(set(older_hashes), set(hashes))
        self.assertNotEqual(older_hashes["README.txt"], hashes["README.txt"])
        self.assertEqual(older_hashes["midnight-explorer.exe"], hashes["midnight-explorer.exe"])
        self.assertEqual(msi_test.package_property(manifest["older"]["msi"], "ProductVersion"), msi_test.OLDER_VERSION)
        self.assertEqual(msi_test.package_property(manifest["new"]["msi"], "UpgradeCode"),
                         msi_test.package_property(manifest["older"]["msi"], "UpgradeCode"))

        plan_text = io.StringIO()
        msi_test.main(["run", "--work", str(self.folder / "work"), "--dry-run", "--ui-smoke"], out=plan_text)
        text = plan_text.getvalue()
        self.assertIn("msiexec.exe /i", text)
        self.assertIn("ALLUSERS=1", text)
        self.assertIn("--ui-smoke-test --software", text)
        self.assertRegex(text, r"Planned: \d+ steps of scopes")
        self.assertEqual(sorted(path.name for path in (self.folder / "work").iterdir()),
                         ["manifest.json", "new", "older"], "a dry run writes nothing")


if __name__ == "__main__":
    unittest.main()
