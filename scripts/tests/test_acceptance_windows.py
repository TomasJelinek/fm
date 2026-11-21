"""The Windows acceptance runner (scripts/acceptance.ps1): plan, arguments and the
private installation it runs the suite from.

The run tests give the runner a stand-in test executable, the `py.exe` launcher, whose
first argument (passed as a scenario name) is a script that records what it sees: the
environment, the installation's folders and its access control list. No Cargo build
runs.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
PWSH = shutil.which("pwsh")
LAUNCHER = shutil.which("py")
RECORDER = r'''
import json, os, subprocess, sys
from pathlib import Path
packages = Path(os.environ["MIDNIGHT_ACCEPTANCE_PLUGINS"])
root = Path(os.environ["MIDNIGHT_ACCEPTANCE_STAGE_ROOT"])
acl = subprocess.run(["icacls", str(root)], capture_output=True, text=True).stdout
record = {
    "packages": str(packages),
    "root": str(root),
    "run_dir": os.environ.get("MIDNIGHT_ACCEPTANCE_RUN_DIR"),
    "temp": os.environ.get("TEMP"),
    "bin": sorted(p.name for p in (root / "bin").iterdir()),
    "arguments": sys.argv[1:],
    "installed": {kind: sorted(p.name for p in (packages / kind).iterdir())
                  for kind in ("viewers", "filesystems", "thumbnailers")},
    "acl": acl,
}
Path(os.environ["RECORD"]).write_text(json.dumps(record), encoding="utf-8")
sys.exit(int(os.environ.get("RECORD_EXIT", "0")))
'''


@unittest.skipUnless(os.name == "nt" and PWSH, "Windows runner with PowerShell 7")
class AcceptanceRunnerTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="acceptance runner ", dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name)

    def run_script(self, *arguments, environment=None):
        return subprocess.run([PWSH, "-NoLogo", "-NoProfile", "-File",
                               str(ROOT / "scripts/acceptance.ps1"), *arguments],
                              capture_output=True, text=True, timeout=120,
                              env=dict(os.environ, **(environment or {})))

    def test_dry_run_prints_the_plan_without_creating_anything(self):
        before = set(Path(os.environ["USERPROFILE"]).glob("MidnightAcceptance-*"))
        result = self.run_script("--dry-run", "--validation-packages", "--scale", "smoke",
                                 "--only", "startup,files_links", "--run-dir", str(self.root))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        self.assertIn("Build: [cargo] [test] [--workspace] [--locked] [--features] "
                      "[midnight-acceptance/acceptance] [--test] [acceptance] [--no-run] "
                      "[--message-format] [json-render-diagnostics]", lines)
        validation = ROOT / "_build" / "validate"
        for kind, folder in (("viewers", "viewer-packages"), ("filesystems", "filesystem-packages"),
                             ("filesystems", "archive-ci\\package")):
            self.assertIn(f"Plugin packages: {kind} <- {validation / folder}", lines)
        self.assertIn(f"Run folders: {self.root}", lines)
        self.assertIn("Command: [<installation>\\bin\\acceptance.exe] [--scale] [smoke] "
                      "[startup] [files_links]", lines)
        self.assertTrue(any(line.startswith("Private installation: " + os.environ["USERPROFILE"]
                                            + "\\MidnightAcceptance-") for line in lines), lines)
        self.assertEqual(set(Path(os.environ["USERPROFILE"]).glob("MidnightAcceptance-*")), before)

    def test_invalid_arguments_stop_with_usage(self):
        for arguments in (("--bogus",), ("debug", "release"), ("--scale", "huge"), ("--scale",),
                          ("--only", "-x"), ("--only", "a,,b"), ("--run-dir", "relative"),
                          ("--plugins", "x", "--validation-packages")):
            with self.subTest(arguments=arguments):
                result = self.run_script("--dry-run", *arguments)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("Usage: scripts\\acceptance.ps1", result.stderr)

    def test_help_prints_usage(self):
        result = self.run_script("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Usage: scripts\\acceptance.ps1", result.stdout)

    @unittest.skipUnless(LAUNCHER, "the py launcher stands in for the test executable")
    def test_run_uses_an_owner_only_installation_and_removes_it(self):
        plugins = self.root / "plugins"
        for package in ("viewers/basic", "filesystems/native.windows",
                        "filesystems/network.windows", "thumbnailers/image"):
            (plugins / package).mkdir(parents=True)
            (plugins / package / "plugin.toml").write_text("name = 'stub'\n", encoding="utf-8")
        recorder = self.root / "recorder.py"
        recorder.write_text(RECORDER, encoding="utf-8")
        record = self.root / "record.json"
        runs = self.root / "runs"
        runs.mkdir()
        result = self.run_script("--test-binary", LAUNCHER, "--plugins", str(plugins),
                                 "--run-dir", str(runs), "--only", str(recorder),
                                 environment={"RECORD": str(record), "RECORD_EXIT": "1"})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        seen = json.loads(record.read_text(encoding="utf-8"))
        installation = Path(seen["root"])
        self.assertTrue(installation.name.startswith("MidnightAcceptance-"), seen)
        self.assertEqual(installation.parent, Path(os.environ["USERPROFILE"]))
        self.assertFalse(installation.exists(), "the private installation was not removed")
        self.assertEqual(Path(seen["packages"]), installation / "packages")
        # The test executable runs as a copy in the installation's bin folder.
        self.assertEqual(seen["bin"], [Path(LAUNCHER).name])
        self.assertEqual(seen["arguments"], [])
        self.assertEqual(seen["run_dir"], str(runs))
        self.assertEqual(Path(seen["temp"]), ROOT / "_build" / "tmp")
        # Network providers are never staged.
        self.assertEqual(seen["installed"], {"viewers": ["basic"], "filesystems": ["native.windows"],
                                             "thumbnailers": ["image"]})
        # Three explicit, not inherited entries: this account, SYSTEM and Administrators.
        entries = [line.strip() for line in seen["acl"].splitlines()[:-2] if line.strip()]
        self.assertEqual(len(entries), 3, seen["acl"])
        self.assertFalse(any("(I)" in entry for entry in entries), seen["acl"])
        self.assertTrue(any("NT AUTHORITY\\SYSTEM" in entry for entry in entries), seen["acl"])
        self.assertTrue(any("BUILTIN\\Administrators" in entry for entry in entries), seen["acl"])
        self.assertIn("Plugin packages (filesystems): native.windows", result.stdout)


if __name__ == "__main__":
    unittest.main()
