"""Native MSI authoring contract; never installs a product on the test host."""
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("windows_msi", ROOT / "scripts/windows-msi.py")
msi = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(msi)


class WindowsMsiTests(unittest.TestCase):
    def setUp(self):
        (ROOT / "_build/tmp").mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=ROOT / "_build/tmp")
        self.addCleanup(self.temporary.cleanup)
        self.work = Path(self.temporary.name)
        self.source = self.work / "source café"
        self.source.mkdir()
        (self.source / "me.exe").write_bytes(b"fixture main")
        (self.source / "BUILD-INFO.json").write_text(json.dumps({
            "version": "0.3.0", "target": "x86_64-pc-windows-msvc"}), encoding="utf-8")
        (self.source / "sub folder").mkdir()
        (self.source / "sub folder/café.txt").write_bytes(b"retained bytes")

    def author(self, version="0.3.0"):
        return msi.author(self.source, version, self.work / ("author-" + version))

    def test_payload_is_complete_and_uses_stable_component_identity(self):
        first = self.author()
        for entry in first["payload"]:
            self.assertEqual((Path(first["work"]) / "cab-input" / entry["id"]).read_bytes(),
                             (self.source / entry["path"]).read_bytes())
        (self.source / "BUILD-INFO.json").write_text(json.dumps({
            "version": "0.4.0", "target": "x86_64-pc-windows-msvc"}), encoding="utf-8")
        second = self.author("0.4.0")
        self.assertEqual({p["path"] for p in first["payload"]},
                         {"me.exe", "BUILD-INFO.json", "sub folder/café.txt"})
        self.assertNotEqual(first["product_code"], second["product_code"])
        self.assertEqual(first["upgrade_code"], second["upgrade_code"])
        self.assertEqual(first["tables"]["Component"]["rows"],
                         second["tables"]["Component"]["rows"])

    def test_per_user_install_and_only_folder_choice(self):
        plan = self.author()
        tables = plan["tables"]
        properties = dict(tables["Property"]["rows"])
        self.assertNotIn("ALLUSERS", properties)
        self.assertEqual(properties["MSIINSTALLPERUSER"], "1")
        self.assertNotIn("ARPINSTALLLOCATION", properties)
        self.assertEqual(tables["Signature"]["rows"], [])
        self.assertIn(["INSTALLFOLDER", "ProgramsFolder", "MIDNIG~1|Midnight Explorer"],
                      tables["Directory"]["rows"])
        controls = tables["Control"]["rows"]
        self.assertEqual(sum(row[2] == "PathEdit" for row in controls), 1)
        self.assertEqual(len(tables["Feature"]["rows"]), 1)
        self.assertNotIn("CustomAction", tables)
        self.assertNotIn("RemoveFile", tables)
        self.assertTrue(any("NOT ALLUSERS" in row[0] for row in tables["LaunchCondition"]["rows"]))

    def test_version_and_inventory_mismatch_fail(self):
        for version in ("0.3.0-beta", "256.0.0", "0.3.65536", "0.4.0"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                self.author(version)

    def test_ambiguous_or_unrepresentable_payload_paths_fail(self):
        (self.source / "ME.EXE").write_bytes(b"collision")
        if len(list(self.source.glob("*.exe"))) > 1:
            with self.assertRaises(ValueError):
                self.author()
            (self.source / "ME.EXE").unlink()
        (self.source / "日本語.txt").write_bytes(b"unsupported MSI codepage")
        with self.assertRaises(ValueError):
            self.author()

    def test_symlink_or_reparse_payload_rejected(self):
        target = self.work / "outside"
        target.write_bytes(b"outside")
        link = self.source / "linked.txt"
        try:
            link.symlink_to(target)
        except OSError as error:
            self.skipTest(str(error))
        with self.assertRaises(ValueError):
            self.author()

    @unittest.skipUnless(os.name == "nt", "Windows junction boundary")
    def test_directory_junction_rejected_without_symlink_privilege(self):
        import _winapi
        target = self.work / "external"
        target.mkdir()
        (target / "unexpected.dll").write_bytes(b"external")
        link = self.source / "redirected"
        _winapi.CreateJunction(str(target), str(link))
        self.addCleanup(os.rmdir, link)
        with self.assertRaises(ValueError):
            self.author()

    def test_non_build_output_rejected(self):
        with self.assertRaises(ValueError):
            msi.author(self.source, "0.3.0", ROOT / "forbidden-msi-output")


if __name__ == "__main__":
    unittest.main()
