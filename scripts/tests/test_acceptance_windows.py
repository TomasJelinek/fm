"""The Windows acceptance runner (scripts/acceptance.ps1): plan, arguments and the
private installation it runs the suite from.

The run tests give the runner a stand-in test executable, the `py.exe` launcher, whose
first argument (passed as a scenario name) is a script that records what it sees: the
environment, the installation's folders and its access control list. No Cargo build
runs. They give the runner a private `USERPROFILE` below the test's temporary folder,
so the installation it creates and removes never touches the real profile. A run is
refused when its run folders' volume is not qualified for Copy and Move (8.3 short names);
the run tests stand in for that probe with `MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME`, and
the probe itself is checked against the volume's state read here.
"""
import ctypes
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
PWSH = shutil.which("pwsh")
LAUNCHER = shutil.which("py")
# The video worker's FFmpeg libraries and playback helpers (bundled and Media Foundation),
# which the application grants the restricted workers' AppContainer.
VIDEO_FILES = ["avcodec-63.dll", "avformat-63.dll", "avutil-61.dll", "swscale-10.dll",
               "swresample-7.dll", "midnight-video-playback.exe", "midnight-video-playback-system.exe"]
# The SID of the top-level AppContainer `MidnightExplorer.RestrictedWorker.v1`
# (scripts/tests/test_restricted_worker_identity.py checks it against Windows).
WORKER_SID = "S-1-15-2-210492624-3469538162-618285203-1868907-2672626540-1166629363-1118814998"
# The runner's stand-in for its run-folder volume probe, for the tests whose run folders
# are wherever the test's temporary folder is.
QUALIFIED = {"MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME": "qualified"}


def copy_volume(folder):
    """The root of the volume `folder` is on, and the runner's reason for it: NTFS whose
    persistent volume state disables 8.3 short names (FSCTL_QUERY_PERSISTENT_VOLUME_STATE
    on the root folder, as the platform reads it) is qualified for Copy and Move."""
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                     wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD,
                                     wintypes.HANDLE]
    kernel32.DeviceIoControl.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPVOID,
                                         wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD,
                                         ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    root = ctypes.create_unicode_buffer(1024)
    if not kernel32.GetVolumePathNameW(str(folder), root, len(root)):
        raise ctypes.WinError(ctypes.get_last_error())
    filesystem = ctypes.create_unicode_buffer(64)
    if not kernel32.GetVolumeInformationW(root.value, None, 0, None, None, None, filesystem,
                                          len(filesystem)):
        raise ctypes.WinError(ctypes.get_last_error())
    if filesystem.value != "NTFS":
        return root.value, f"{filesystem.value}, not NTFS"
    # FILE_READ_ATTRIBUTES, every sharing mode, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS.
    handle = kernel32.CreateFileW(root.value, 0x80, 7, None, 3, 0x02000000, None)
    if handle in (None, ctypes.c_void_p(-1).value):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        # FILE_FS_PERSISTENT_VOLUME_INFORMATION: VolumeFlags, FlagMask, Version, Reserved.
        request = (ctypes.c_uint32 * 4)(0, 1, 1, 0)
        response = (ctypes.c_uint32 * 4)()
        returned = wintypes.DWORD()
        if not kernel32.DeviceIoControl(handle, 0x9023C, request, 16, response, 16,
                                        ctypes.byref(returned), None):
            raise ctypes.WinError(ctypes.get_last_error())
    finally:
        kernel32.CloseHandle(handle)
    if returned.value >= 16 and response[0] & 1:
        return root.value, "NTFS, 8.3 short-name creation disabled"
    return root.value, "NTFS whose volume state does not disable 8.3 short names"
# What the private installation's packages\thumbnailers holds of the stub plugins.
THUMBNAILERS = sorted(VIDEO_FILES + ["midnight-thumbnail-image.exe", "midnight-thumbnail-image.json",
                                     "midnight-thumbnail-video.exe", "midnight-thumbnail-video.json",
                                     "midnight-thumbnail-video-system.exe", "notices"])
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
    "stress": os.environ.get("MIDNIGHT_ACCEPTANCE_STRESS"),
    "fat_dir": os.environ.get("MIDNIGHT_ACCEPTANCE_FAT_DIR"),
    "temp": os.environ.get("TEMP"),
    "bin": sorted(p.name for p in (root / "bin").iterdir()),
    # The run's marker in its installation (None: there is none).
    "marker": (root / ".acceptance-installation").read_text(encoding="utf-8")
              if (root / ".acceptance-installation").is_file() else None,
    "arguments": sys.argv[1:],
    # None: the kind's folder does not exist.
    "installed": {kind: sorted(p.name for p in (packages / kind).iterdir())
                  if (packages / kind).is_dir() else None
                  for kind in ("viewers", "filesystems", "thumbnailers")},
    "acl": acl,
    "file_acls": {p.name: subprocess.run(["icacls", str(p)], capture_output=True, text=True).stdout
                  for p in (thumbnailers.iterdir() if thumbnailers.is_dir() else []) if p.is_file()},
}
Path(os.environ["RECORD"]).write_text(json.dumps(record), encoding="utf-8")
sys.exit(int(os.environ.get("RECORD_EXIT", "0")))
'''
# One PowerShell session that runs the runner in-process twice and then as a dry run,
# recording its working directory and the variables the runner sets after each.
IN_PROCESS = r'''
$ErrorActionPreference = 'Stop'
$names = @('TEMP', 'TMP', 'CARGO_HOME', 'RUSTUP_HOME', 'MIDNIGHT_ACCEPTANCE_PLUGINS',
    'MIDNIGHT_ACCEPTANCE_STAGE_ROOT', 'MIDNIGHT_ACCEPTANCE_RUN_DIR', 'MIDNIGHT_ACCEPTANCE_STRESS')
function Get-SessionState {
    $variables = [ordered]@{}
    foreach ($name in $names) { $variables[$name] = [Environment]::GetEnvironmentVariable($name) }
    [ordered]@{ location = (Get-Location).Path; variables = $variables }
}
Set-Location -LiteralPath $env:START
$steps = @([ordered]@{ step = 'before'; code = 0; state = Get-SessionState })
foreach ($run in 1, 2) {
    $env:RECORD = Join-Path $env:START "record-$run.json"
    & $env:RUNNER --test-binary $env:LAUNCHER --run-dir $env:RUNS --only $env:RECORDER --stress
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

    def assert_no_worker_grants(self, file_acls):
        """No staged file names the workers' AppContainer: like files extracted from a
        release ZIP, the video helpers' libraries get its grant from the application."""
        self.assertTrue(file_acls)
        for name, acl in file_acls.items():
            with self.subTest(file=name):
                self.assertNotIn(WORKER_SID, acl)

    def private_profile(self):
        profile = self.root / "profile"
        profile.mkdir()
        return profile

    def test_dry_run_prints_the_plan_without_creating_anything(self):
        before = set(Path(os.environ["USERPROFILE"]).glob("MidnightAcceptance-*"))
        fat = self.root / "fat volume"
        result = self.run_script("--dry-run", "--validation-packages", "--scale", "smoke",
                                 "--only", "startup,files_links", "--run-dir", str(self.root),
                                 "--stress", "--jobs", "3", environment={"MIDNIGHT_ACCEPTANCE_FAT_DIR": str(fat)})
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
        self.assertIn("Stress sizes: on (MIDNIGHT_ACCEPTANCE_STRESS=1)", lines)
        self.assertIn(f"Gated folder: MIDNIGHT_ACCEPTANCE_FAT_DIR = {fat}", lines)
        self.assertIn("Command: [<installation>\\bin\\acceptance.exe] [--scale] [smoke] "
                      "[startup] [files_links] [--stress] [--jobs] [3]", lines)
        self.assertTrue(any(line.startswith("Private installation: " + os.environ["USERPROFILE"]
                                            + "\\MidnightAcceptance-") for line in lines), lines)
        self.assertEqual(set(Path(os.environ["USERPROFILE"]).glob("MidnightAcceptance-*")), before)

    def test_the_dry_run_names_the_run_folders_volume_as_the_platform_reads_it(self):
        # The probe reads the volume's persistent state, as the platform does, not the
        # registry or whether a file got a short name.
        root, reason = copy_volume(self.root)
        verdict = "" if reason == "NTFS, 8.3 short-name creation disabled" else "; a run there stops"
        result = self.run_script("--dry-run", "--run-dir", str(self.root / "runs"))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"Run folder volume: {root} ({reason}){verdict}", result.stdout.splitlines())
        # A dry run only says that a run would stop.
        result = self.run_script("--dry-run", "--run-dir", str(self.root / "runs"),
                                 environment={"MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME": "unqualified"})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"Run folder volume: {self.root} (unqualified by "
                      "MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME); a run there stops",
                      result.stdout.splitlines())

    @unittest.skipUnless(LAUNCHER, "the py launcher stands in for the test executable")
    def test_a_run_on_a_volume_that_keeps_8dot3_names_stops_before_anything(self):
        # There Copy and Move take the engine's opaque lane and their jobs wait for each
        # other, so side-by-side checks would fail for no fault of the product.
        plugins = self.stub_plugins()
        profile = self.private_profile()
        recorder = self.root / "recorder.py"
        recorder.write_text(RECORDER, encoding="utf-8")
        record = self.root / "record.json"
        runs = self.root / "runs"
        result = self.run_script("--test-binary", LAUNCHER, "--plugins", str(plugins),
                                 "--run-dir", str(runs), "--only", str(recorder),
                                 environment={"RECORD": str(record), "USERPROFILE": str(profile),
                                              "MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME":
                                              "unqualified"})
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn(f"the run folders {runs} are on {self.root} (unqualified by "
                      "MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME)", result.stderr)
        self.assertIn("opaque lane", result.stderr)
        self.assertIn('docs/developer/TEST_EXECUTION.md, "Windows Copy/Move qualification volume"',
                      result.stderr)
        # Nothing ran and nothing was made: no installation, no run folder, no record.
        self.assertFalse(record.exists())
        self.assertFalse(runs.exists())
        self.assertEqual(list(profile.glob("MidnightAcceptance-*")), [])

    @unittest.skipUnless(LAUNCHER, "the py launcher stands in for the test executable")
    def test_a_run_that_requires_a_lease_runs_only_under_it(self):
        # A wrapper names its lease's lock in MIDNIGHT_ACCEPTANCE_REQUIRE_LEASE: the lock must
        # name the runner or a process that started it, or the run stops before anything runs.
        lock = self.root / "desktop.lock"
        profile = self.private_profile()
        recorder = self.root / "recorder.py"
        recorder.write_text(RECORDER, encoding="utf-8")
        record = self.root / "record.json"

        def run(holder):
            if holder is None:
                lock.unlink(missing_ok=True)
            else:
                lock.write_text(holder, encoding="utf-8")
            record.unlink(missing_ok=True)
            result = self.run_script("--test-binary", LAUNCHER, "--run-dir", str(self.root / "runs"),
                                     "--only", str(recorder),
                                     environment={"RECORD": str(record), "USERPROFILE": str(profile),
                                                  "MIDNIGHT_ACCEPTANCE_REQUIRE_LEASE": str(lock),
                                                  **QUALIFIED})
            return result, record.exists()

        # This test process started the runner, so its lease counts.
        result, ran = run(f"test {os.getpid()} 2026-11-22T10:00:00")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(ran)
        for holder, reason in (("other 4 1 2026-11-22T10:00:00",
                                'it is held by "other 4 1 2026-11-22T10:00:00", not by this run'),
                               (None, "it is not held")):
            with self.subTest(holder=holder):
                result, ran = run(holder)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn(f"the lease {lock} (MIDNIGHT_ACCEPTANCE_REQUIRE_LEASE) was not taken "
                              f"for this run: {reason}", result.stderr)
                self.assertFalse(ran)
        self.assertEqual(list(profile.glob("MidnightAcceptance-*")), [])

    @unittest.skipUnless(LAUNCHER, "the py launcher stands in for the test executable")
    def test_packages_newer_than_the_checkout_stop_the_run(self):
        # The native filesystem host refuses a package whose minimum_host_minor exceeds its
        # ABI_MINOR: every staged scenario would then miss its filesystem plugin.
        abi = (ROOT / "filesystem-api/midnight-filesystem-plugin-api/src/abi.rs").read_text(
            encoding="utf-8")
        minor = int(re.search(r"^pub const ABI_MINOR: u16 = (\d+);", abi, re.M).group(1))
        plugins = self.root / "plugins"
        package = plugins / "filesystems" / "native.windows"
        package.mkdir(parents=True)
        profile = self.private_profile()
        for wanted, refused in ((minor + 1, True), (minor, False)):
            with self.subTest(minimum_host_minor=wanted):
                (package / "plugin.toml").write_text(
                    f"abi_major = 1\nminimum_host_minor = {wanted}\n", encoding="utf-8")
                problem = (f"packages newer than this build: {package} needs filesystem host "
                           f"ABI minor {wanted} and this checkout has {minor}")
                plan = self.run_script("--dry-run", "--plugins", str(plugins))
                self.assertEqual(plan.returncode, 0, plan.stdout + plan.stderr)
                self.assertEqual(any(line.startswith(f"A run stops: {problem}")
                                     for line in plan.stdout.splitlines()), refused, plan.stdout)
                recorder = self.root / "recorder.py"
                recorder.write_text(RECORDER, encoding="utf-8")
                record = self.root / f"record-{wanted}.json"
                result = self.run_script("--test-binary", LAUNCHER, "--plugins", str(plugins),
                                         "--run-dir", str(self.root / "runs"), "--only", str(recorder),
                                         environment={"RECORD": str(record), "USERPROFILE": str(profile),
                                                      **QUALIFIED})
                self.assertEqual(result.returncode, 2 if refused else 0, result.stdout + result.stderr)
                self.assertEqual(problem in result.stderr, refused, result.stderr)
                self.assertEqual(record.exists(), not refused)
                self.assertEqual(list(profile.glob("MidnightAcceptance-*")), [])

    def test_a_relative_gated_folder_stops_with_usage(self):
        for variable in ("MIDNIGHT_ACCEPTANCE_FAT_DIR", "MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS"):
            for value in ("relative", "C:relative"):
                with self.subTest(variable=variable, value=value):
                    result = self.run_script("--dry-run", environment={variable: value})
                    self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                    self.assertIn(f"{variable} {value} is not an absolute folder", result.stderr)
                    self.assertIn("Usage: pwsh -File scripts\\acceptance.ps1", result.stderr)

    def test_the_runner_grants_the_workers_nothing(self):
        # The application grants the video helpers' files in a private installation; a
        # grant here would hide a release ZIP's missing one from viewers_video.
        text = (ROOT / "scripts/acceptance.ps1").read_text(encoding="utf-8")
        for grant in ("Get-RestrictedWorkerSid", "AddAccessRule", "Set-Acl -LiteralPath $path"):
            self.assertNotIn(grant, text)

    def test_invalid_arguments_stop_with_usage(self):
        for arguments in (("--bogus",), ("debug", "release"), ("--scale", "huge"), ("--scale",),
                          ("--jobs",), ("--jobs", "0"), ("--jobs", "5"), ("--jobs", "many"),
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
        fat = self.root / "fat volume"
        result = self.run_script("--test-binary", LAUNCHER, "--plugins", str(plugins),
                                 "--run-dir", str(runs), "--only", str(recorder),
                                 environment={"RECORD": str(record), "RECORD_EXIT": "1",
                                              "USERPROFILE": str(profile),
                                              "MIDNIGHT_ACCEPTANCE_FAT_DIR": str(fat), **QUALIFIED})
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
        # A gated volume's folder reaches the suite (and its staged children) as it is.
        self.assertEqual(seen["fat_dir"], str(fat))
        self.assertIn(f"Gated folder: MIDNIGHT_ACCEPTANCE_FAT_DIR = {fat}", result.stdout.splitlines())
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
        # A copy loses explicit entries, and the runner adds none.
        self.assert_no_worker_grants(seen["file_acls"])
        # The installation names the process that ran it: "<process id> <start ticks> <time>".
        self.assertRegex(seen["marker"] or "", r"^\d+ \d+ \S+$")

    @unittest.skipUnless(LAUNCHER, "the py launcher stands in for the test executable")
    def test_the_next_run_removes_installations_whose_run_has_ended(self):
        # A run that is killed leaves its private installation behind. Its marker names the
        # run's process; the next run removes it once that process has ended or its id
        # belongs to another process. One without a marker is named and left alone however
        # old it is, as is one whose run still runs; nothing else in the profile is touched.
        plugins = self.stub_plugins()
        profile = self.private_profile()
        ended = int(subprocess.run([LAUNCHER, "-c", "import os; print(os.getpid())"],
                                   capture_output=True, text=True, check=True).stdout)
        alive = os.getpid()
        ticks = int(subprocess.run(
            [PWSH, "-NoLogo", "-NoProfile", "-Command",
             f"(Get-Process -Id {alive}).StartTime.ToUniversalTime().Ticks"],
            capture_output=True, text=True, check=True).stdout)
        markers = {"ended": f"{ended} 638990000000000000 2026-11-22T00:00:00.0000000Z",
                   "reused": f"{alive} {ticks - 10_000_000} 2026-11-22T00:00:00.0000000Z",
                   "running": f"{alive} {ticks} 2026-11-22T00:00:00.0000000Z",
                   "unmarked": None}
        folders = {}
        for name, marker in markers.items():
            folder = profile / f"MidnightAcceptance-{name}"
            (folder / "bin").mkdir(parents=True)
            (folder / "bin" / "acceptance.exe").write_text("stub\n", encoding="utf-8")
            if marker is not None:
                (folder / ".acceptance-installation").write_text(marker, encoding="utf-8")
            folders[name] = folder
        # Twenty years old: age alone never removes an installation.
        os.utime(folders["unmarked"], (946684800, 946684800))
        other = profile / "Other folder"
        other.mkdir()
        environment = {"USERPROFILE": str(profile), **QUALIFIED}

        plan = self.run_script("--dry-run", "--plugins", str(plugins), environment=environment)
        self.assertEqual(plan.returncode, 0, plan.stdout + plan.stderr)
        lines = plan.stdout.splitlines()
        for name in ("ended", "reused"):
            self.assertIn(f"Left by a run that has ended: {folders[name]} (a run removes it)", lines)
        self.assertIn(f"Left without a run marker: {folders['unmarked']} (a run leaves it alone)", lines)
        self.assertFalse(any(str(folders["running"]) in line for line in lines), plan.stdout)
        self.assertTrue(all(folder.is_dir() for folder in folders.values()), "the dry run removed something")

        recorder = self.root / "recorder.py"
        recorder.write_text(RECORDER, encoding="utf-8")
        record = self.root / "record.json"
        runs = self.root / "runs"
        runs.mkdir()
        result = self.run_script("--test-binary", LAUNCHER, "--plugins", str(plugins),
                                 "--run-dir", str(runs), "--only", str(recorder),
                                 environment={"RECORD": str(record), **environment})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        for name in ("ended", "reused"):
            self.assertIn(f"Removed the private installation of a run that has ended: {folders[name]}", lines)
            self.assertFalse(folders[name].exists(), name)
        self.assertIn(f"Left alone: {folders['unmarked']} names no run (no .acceptance-installation); "
                      "remove it by hand once no run uses it", lines)
        for name in ("running", "unmarked"):
            self.assertTrue((folders[name] / "bin" / "acceptance.exe").is_file(), name)
        self.assertTrue(other.is_dir())
        # The run's own installation went as usual: only the two kept ones are left.
        seen = json.loads(record.read_text(encoding="utf-8"))
        self.assertFalse(Path(seen["root"]).exists())
        self.assertEqual(sorted(p.name for p in profile.glob("MidnightAcceptance-*")),
                         ["MidnightAcceptance-running", "MidnightAcceptance-unmarked"])

    @unittest.skipUnless(LAUNCHER, "the py launcher stands in for the test executable")
    def test_without_packages_no_package_folder_is_made(self):
        # The staged scenarios report SKIP when viewers\ or filesystems\ is missing, as on
        # Linux; empty folders made them fail instead ("Native filesystem plugin is
        # unavailable"). Only network providers, which are never staged, count as none.
        plugins = self.root / "plugins"
        (plugins / "filesystems" / "network.windows").mkdir(parents=True)
        (plugins / "filesystems" / "network.windows" / "plugin.toml").write_text(
            "name = 'stub'\n", encoding="utf-8")
        (plugins / "viewers").mkdir()
        profile = self.private_profile()
        recorder = self.root / "recorder.py"
        recorder.write_text(RECORDER, encoding="utf-8")
        record = self.root / "record.json"
        runs = self.root / "runs"
        runs.mkdir()
        result = self.run_script("--test-binary", LAUNCHER, "--plugins", str(plugins),
                                 "--run-dir", str(runs), "--only", str(recorder),
                                 environment={"RECORD": str(record), "RECORD_EXIT": "0",
                                              "USERPROFILE": str(profile), **QUALIFIED})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        seen = json.loads(record.read_text(encoding="utf-8"))
        self.assertEqual(seen["installed"], {"viewers": None, "filesystems": None,
                                             "thumbnailers": None})
        for kind in ("viewers", "filesystems", "thumbnailers"):
            self.assertIn(f"Plugin packages ({kind}): none", result.stdout.splitlines())
        self.assertFalse(Path(seen["root"]).exists(), "the private installation was not removed")

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
                       ("MIDNIGHT_ACCEPTANCE_STAGE_ROOT", "MIDNIGHT_ACCEPTANCE_RUN_DIR",
                        "MIDNIGHT_ACCEPTANCE_STRESS")}
        environment.update({
            "TEMP": str(temporary), "TMP": str(temporary),
            "CARGO_HOME": "cargo home before", "RUSTUP_HOME": "rustup home before",
            # Without --plugins the runner stages the packages this variable names.
            "MIDNIGHT_ACCEPTANCE_PLUGINS": str(plugins),
            "USERPROFILE": str(profile), "RECORD_EXIT": "0",
            "START": str(start), "RUNNER": str(ROOT / "scripts/acceptance.ps1"),
            "LAUNCHER": LAUNCHER, "RUNS": str(runs), "RECORDER": str(recorder),
            "REPORT": str(report), **QUALIFIED,
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
        self.assertIsNone(before["variables"]["MIDNIGHT_ACCEPTANCE_STRESS"])
        for step in steps[1:]:
            with self.subTest(step=step["step"]):
                self.assertEqual(step["code"], 0, result.stdout + result.stderr)
                self.assertEqual(step["state"], before)
        for run in (1, 2):
            with self.subTest(run=run):
                seen = json.loads((start / f"record-{run}.json").read_text(encoding="utf-8"))
                # --stress reaches the suite as its flag and staged children as the variable.
                self.assertEqual(seen["arguments"], ["--stress"])
                self.assertEqual(seen["stress"], "1")
                self.assertEqual(seen["installed"], {"viewers": ["basic"], "filesystems": ["native.windows"],
                                                     "thumbnailers": THUMBNAILERS})
                self.assert_no_worker_grants(seen["file_acls"])
                self.assertEqual(Path(seen["root"]).parent, profile)
                self.assertFalse(Path(seen["root"]).exists(), "the private installation was not removed")


if __name__ == "__main__":
    unittest.main()
