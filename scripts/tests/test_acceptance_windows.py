"""The Windows acceptance runner (scripts/acceptance.ps1): plan, arguments and the
private installation it runs the suite from.

The run tests give the runner a stand-in test executable, the `py.exe` launcher, whose
first argument (passed as a scenario name) is a script that records what it sees: the
environment, the installation's folders and its access control list. No Cargo build
runs. They give the runner a private `USERPROFILE` below the test's temporary folder,
so the installation it creates and removes never touches the real profile.
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
# The video worker's FFmpeg libraries and playback helper, which the restricted workers'
# AppContainer must be granted, as scripts/deploy-private.ps1 grants them.
VIDEO_FILES = ["avcodec-63.dll", "avformat-63.dll", "avutil-61.dll", "swscale-10.dll",
               "swresample-7.dll", "midnight-video-playback.exe"]
# The SID of the top-level AppContainer `MidnightExplorer.RestrictedWorker.v1`
# (scripts/tests/test_restricted_worker_identity.py checks it against Windows).
WORKER_SID = "S-1-15-2-210492624-3469538162-618285203-1868907-2672626540-1166629363-1118814998"
# What the private installation's packages\thumbnailers holds of the stub plugins.
THUMBNAILERS = sorted(VIDEO_FILES + ["midnight-thumbnail-image.exe", "midnight-thumbnail-image.json",
                                     "midnight-thumbnail-video.exe", "midnight-thumbnail-video.json",
                                     "notices"])
RECORDER = r'''
import json, os, subprocess, sys
from pathlib import Path
packages = Path(os.environ["MIDNIGHT_ACCEPTANCE_PLUGINS"])
root = Path(os.environ["MIDNIGHT_ACCEPTANCE_STAGE_ROOT"])
acl = subprocess.run(["icacls", str(root)], capture_output=True, text=True).stdout
thumbnailers = packages / "thumbnailers"
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
    "file_acls": {p.name: subprocess.run(["icacls", str(p)], capture_output=True, text=True).stdout
                  for p in thumbnailers.iterdir() if p.is_file()},
}
Path(os.environ["RECORD"]).write_text(json.dumps(record), encoding="utf-8")
sys.exit(int(os.environ.get("RECORD_EXIT", "0")))
'''
# One PowerShell session that runs the runner in-process twice and then as a dry run,
# recording its working directory and the variables the runner sets after each.
IN_PROCESS = r'''
$ErrorActionPreference = 'Stop'
$names = @('TEMP', 'TMP', 'CARGO_HOME', 'RUSTUP_HOME', 'MIDNIGHT_ACCEPTANCE_PLUGINS',
    'MIDNIGHT_ACCEPTANCE_STAGE_ROOT', 'MIDNIGHT_ACCEPTANCE_RUN_DIR')
function Get-SessionState {
    $variables = [ordered]@{}
    foreach ($name in $names) { $variables[$name] = [Environment]::GetEnvironmentVariable($name) }
    [ordered]@{ location = (Get-Location).Path; variables = $variables }
}
Set-Location -LiteralPath $env:START
$steps = @([ordered]@{ step = 'before'; code = 0; state = Get-SessionState })
foreach ($run in 1, 2) {
    $env:RECORD = Join-Path $env:START "record-$run.json"
    & $env:RUNNER --test-binary $env:LAUNCHER --run-dir $env:RUNS --only $env:RECORDER
    $steps += [ordered]@{ step = "run $run"; code = $LASTEXITCODE; state = Get-SessionState }
}
& $env:RUNNER --dry-run
$steps += [ordered]@{ step = 'dry run'; code = $LASTEXITCODE; state = Get-SessionState }
ConvertTo-Json -InputObject $steps -Depth 5 | Set-Content -LiteralPath $env:REPORT -Encoding utf8
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

    def stub_plugins(self):
        plugins = self.root / "plugins"
        for package in ("viewers/basic", "filesystems/native.windows",
                        "filesystems/network.windows"):
            (plugins / package).mkdir(parents=True)
            (plugins / package / "plugin.toml").write_text("name = 'stub'\n", encoding="utf-8")
        # Thumbnail providers are installed flat, as build.bat leaves them: manifests and
        # executables, the FFmpeg libraries they share and notices.
        thumbnailers = plugins / "thumbnailers"
        (thumbnailers / "notices" / "midnight-thumbnail-image").mkdir(parents=True)
        (thumbnailers / "notices" / "midnight-thumbnail-image" / "LICENSE").write_text(
            "stub\n", encoding="utf-8")
        for name in THUMBNAILERS:
            if name != "notices":
                (thumbnailers / name).write_text("stub\n", encoding="utf-8")
        return plugins

    def assert_worker_grants(self, file_acls):
        """The video files grant the workers' AppContainer read and execute, an ACE of
        their own (not inherited); no other file names it."""
        for name, acl in file_acls.items():
            with self.subTest(file=name):
                grants = [line for line in acl.splitlines() if WORKER_SID in line]
                if name in VIDEO_FILES:
                    self.assertEqual(len(grants), 1, acl)
                    self.assertTrue(grants[0].rstrip().endswith(f"{WORKER_SID}:(RX)"), acl)
                else:
                    self.assertEqual(grants, [], acl)

    def private_profile(self):
        profile = self.root / "profile"
        profile.mkdir()
        return profile

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

    def test_the_worker_sid_is_never_asked_of_windows_in_this_process(self):
        # In a packaged PowerShell that API returns a child AppContainer of the package.
        text = (ROOT / "scripts/acceptance.ps1").read_text(encoding="utf-8")
        self.assertNotIn("DeriveAppContainerSidFromAppContainerName(", text)
        self.assertIn("function Get-RestrictedWorkerSid", text)

    def test_invalid_arguments_stop_with_usage(self):
        for arguments in (("--bogus",), ("debug", "release"), ("--scale", "huge"), ("--scale",),
                          ("--only", "-x"), ("--only", "a,,b"), ("--run-dir", "relative"),
                          ("--plugins", "x", "--validation-packages")):
            with self.subTest(arguments=arguments):
                result = self.run_script("--dry-run", *arguments)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("Usage: pwsh -File scripts\\acceptance.ps1", result.stderr)

    def test_help_prints_usage(self):
        result = self.run_script("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Usage: pwsh -File scripts\\acceptance.ps1", result.stdout)

    @unittest.skipUnless(LAUNCHER, "the py launcher stands in for the test executable")
    def test_run_uses_an_owner_only_installation_and_removes_it(self):
        plugins = self.stub_plugins()
        profile = self.private_profile()
        recorder = self.root / "recorder.py"
        recorder.write_text(RECORDER, encoding="utf-8")
        record = self.root / "record.json"
        runs = self.root / "runs"
        runs.mkdir()
        result = self.run_script("--test-binary", LAUNCHER, "--plugins", str(plugins),
                                 "--run-dir", str(runs), "--only", str(recorder),
                                 environment={"RECORD": str(record), "RECORD_EXIT": "1",
                                              "USERPROFILE": str(profile)})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        seen = json.loads(record.read_text(encoding="utf-8"))
        installation = Path(seen["root"])
        self.assertTrue(installation.name.startswith("MidnightAcceptance-"), seen)
        self.assertEqual(installation.parent, profile)
        self.assertFalse(installation.exists(), "the private installation was not removed")
        self.assertEqual(Path(seen["packages"]), installation / "packages")
        # The test executable runs as a copy in the installation's bin folder.
        self.assertEqual(seen["bin"], [Path(LAUNCHER).name])
        self.assertEqual(seen["arguments"], [])
        self.assertEqual(seen["run_dir"], str(runs))
        self.assertEqual(Path(seen["temp"]), ROOT / "_build" / "tmp")
        # Network providers are never staged; the flat thumbnailers folder is copied whole.
        self.assertEqual(seen["installed"], {"viewers": ["basic"], "filesystems": ["native.windows"],
                                             "thumbnailers": THUMBNAILERS})
        # Three explicit, not inherited entries: this account, SYSTEM and Administrators.
        entries = [line.strip() for line in seen["acl"].splitlines()[:-2] if line.strip()]
        self.assertEqual(len(entries), 3, seen["acl"])
        self.assertFalse(any("(I)" in entry for entry in entries), seen["acl"])
        self.assertTrue(any("NT AUTHORITY\\SYSTEM" in entry for entry in entries), seen["acl"])
        self.assertTrue(any("BUILTIN\\Administrators" in entry for entry in entries), seen["acl"])
        self.assertIn("Plugin packages (filesystems): native.windows", result.stdout)
        # Each manifest names a provider; libraries and notices are not packages.
        self.assertIn("Plugin packages (thumbnailers): midnight-thumbnail-image, midnight-thumbnail-video",
                      result.stdout.splitlines())
        # A copy loses explicit entries: the runner grants the workers' AppContainer again.
        self.assert_worker_grants(seen["file_acls"])

    @unittest.skipUnless(LAUNCHER, "the py launcher stands in for the test executable")
    def test_in_process_runs_restore_the_session_and_stage_the_packages_again(self):
        # Run in-process, the runner used to leave MIDNIGHT_ACCEPTANCE_PLUGINS naming the
        # removed installation's packages (the next run staged none), and TEMP, TMP,
        # MIDNIGHT_ACCEPTANCE_RUN_DIR and the working directory changed.
        plugins = self.stub_plugins()
        profile = self.private_profile()
        recorder = self.root / "recorder.py"
        recorder.write_text(RECORDER, encoding="utf-8")
        runs = self.root / "runs"
        runs.mkdir()
        start = self.root / "start"
        start.mkdir()
        temporary = self.root / "temp before"
        temporary.mkdir()
        report = self.root / "report.json"
        environment = {name: value for name, value in os.environ.items() if name.upper() not in
                       ("MIDNIGHT_ACCEPTANCE_STAGE_ROOT", "MIDNIGHT_ACCEPTANCE_RUN_DIR")}
        environment.update({
            "TEMP": str(temporary), "TMP": str(temporary),
            "CARGO_HOME": "cargo home before", "RUSTUP_HOME": "rustup home before",
            # Without --plugins the runner stages the packages this variable names.
            "MIDNIGHT_ACCEPTANCE_PLUGINS": str(plugins),
            "USERPROFILE": str(profile), "RECORD_EXIT": "0",
            "START": str(start), "RUNNER": str(ROOT / "scripts/acceptance.ps1"),
            "LAUNCHER": LAUNCHER, "RUNS": str(runs), "RECORDER": str(recorder),
            "REPORT": str(report),
        })
        result = subprocess.run([PWSH, "-NoLogo", "-NoProfile", "-Command", IN_PROCESS],
                                capture_output=True, text=True, timeout=240, env=environment)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        steps = json.loads(report.read_text(encoding="utf-8-sig"))
        self.assertEqual([step["step"] for step in steps], ["before", "run 1", "run 2", "dry run"])
        before = steps[0]["state"]
        self.assertEqual(Path(before["location"]), start)
        self.assertEqual(before["variables"]["MIDNIGHT_ACCEPTANCE_PLUGINS"], str(plugins))
        self.assertIsNone(before["variables"]["MIDNIGHT_ACCEPTANCE_STAGE_ROOT"])
        for step in steps[1:]:
            with self.subTest(step=step["step"]):
                self.assertEqual(step["code"], 0, result.stdout + result.stderr)
                self.assertEqual(step["state"], before)
        for run in (1, 2):
            with self.subTest(run=run):
                seen = json.loads((start / f"record-{run}.json").read_text(encoding="utf-8"))
                self.assertEqual(seen["installed"], {"viewers": ["basic"], "filesystems": ["native.windows"],
                                                     "thumbnailers": THUMBNAILERS})
                self.assert_worker_grants(seen["file_acls"])
                self.assertEqual(Path(seen["root"]).parent, profile)
                self.assertFalse(Path(seen["root"]).exists(), "the private installation was not removed")


if __name__ == "__main__":
    unittest.main()
