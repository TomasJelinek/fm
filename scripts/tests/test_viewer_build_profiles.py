"""Normal viewer builds preserve Debug through every mandatory plugin boundary."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


def temporary_outside_build(test):
    """A temporary directory with no _build component (CI points TEMP into _build)."""
    for candidate in (tempfile.gettempdir(), os.environ.get("RUNNER_TEMP"), "/tmp",
                      os.path.join(os.environ.get("LOCALAPPDATA", ""), "Temp")):
        if candidate and os.path.isdir(candidate) and "_build" not in Path(candidate).resolve().parts:
            directory = tempfile.TemporaryDirectory(prefix="viewer package ", dir=candidate)
            test.addCleanup(directory.cleanup)
            return Path(directory.name)
    raise unittest.SkipTest("No temporary directory outside _build")


@unittest.skipUnless(sys.platform == "linux", "Linux viewer build helpers")
class ViewerBuildProfiles(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="viewer profiles ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "calls.jsonl"
        self.config = self.root / "SDK config.toml"
        self.config.write_text("# fixture\n")
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                                VIEWER_TEST_LOG=str(self.log))
        for name in ("MIDNIGHT_TEST_VIEWERS", "MIDNIGHT_VIEWERS_OFFLINE", "CARGO_TARGET_DIR"):
            self.environment.pop(name, None)
        self.executable(self.bin / "rustc", "print('host: x86_64-unknown-linux-gnu')\n")
        self.executable(self.bin / "cargo", """
with open(os.environ['VIEWER_TEST_LOG'], 'a') as log:
    log.write(json.dumps(['cargo', sys.argv[1:]]) + '\\n')
if 'build' in sys.argv:
    manifest = pathlib.Path(sys.argv[sys.argv.index('--manifest-path') + 1])
    name = manifest.parent.name.removeprefix('midnight-viewer-')
    target = pathlib.Path(sys.argv[sys.argv.index('--target-dir') + 1])
    platform = sys.argv[sys.argv.index('--target') + 1]
    profile = 'release' if '--release' in sys.argv else 'debug'
    output = target / platform / profile
    output.mkdir(parents=True, exist_ok=True)
    (output / f'libmidnight_viewer_{name}.so').write_bytes(profile.encode())
""")

    def executable(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"#!{sys.executable}\nimport json, os, pathlib, sys\n" + body)
        path.chmod(0o755)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def project(self, name, label):
        project = self.root / label / "plugins" / f"midnight-viewer-{name}"
        scripts = project / "scripts"
        scripts.mkdir(parents=True)
        shutil.copyfile(ROOT / "plugins" / f"midnight-viewer-{name}/scripts/build.sh", scripts / "build.sh")
        (scripts / "prepare-sdk.py").write_text("import sys\nprint(sys.argv[1])\n")
        for filename in ("Cargo.toml", "LICENSE", "UNICODE-LICENSE.txt", "README.md", "THIRD-PARTY.md"):
            (project / filename).write_text("Pinned fixture notice\n")
        (project / "plugin.toml").write_text('entry_library = "fixture"\ntarget = "fixture"\n')
        (project / "notices").mkdir()
        (project / "notices/LICENSE").write_text("Third-party terms\n")
        return project

    def build(self, project, package, *profile):
        arguments = [str(self.config), str(project.parents[1] / "_build/native"), str(package), *profile]
        return subprocess.run(["bash", str(project / "scripts/build.sh"), *arguments],
                              env=self.environment, text=True, capture_output=True, timeout=20)

    def test_legacy_plugin_default_and_explicit_profiles_select_matching_artifacts(self):
        for name in ("basic", "source", "markdown"):
            for profile in (None, "debug", "release"):
                with self.subTest(plugin=name, profile=profile):
                    project = self.project(name, f"{name}-{profile}")
                    package = project.parents[1] / "_build/package"
                    # A notice of a dependency the rebuilt plugin no longer ships, from a
                    # package that predates the generated-notices marker.
                    (package / "notices/removed-crate-1.0.0").mkdir(parents=True)
                    (package / "notices/removed-crate-1.0.0/LICENSE").write_text("Stale terms\n")
                    result = self.build(project, package, *([profile] if profile else []))
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertEqual((package / f"libmidnight_viewer_{name}.so").read_bytes(),
                                     (profile or "debug").encode())
                    self.assertEqual((package / "notices/LICENSE").read_text(), "Third-party terms\n")
                    self.assertFalse((package / "notices/removed-crate-1.0.0").exists())
                    self.assertEqual((package / "notices/.midnight-generated").read_text(),
                                     f"midnight-viewer-{name} notices\n")

    def test_notices_outside_build_are_replaced_only_with_the_generated_marker(self):
        # Documented packages may be built anywhere (for example /app/viewers/...);
        # there only the marker proves that this script generated the notices.
        outside = temporary_outside_build(self)
        for name in ("basic", "source", "markdown"):
            with self.subTest(plugin=name):
                project = self.project(name, f"{name}-outside")
                package = outside / "app/viewers" / f"midnight-viewer-{name}"
                (package / "notices").mkdir(parents=True)
                (package / "notices/user file").write_text("not generated\n")
                result = self.build(project, package)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("did not generate", result.stderr)
                self.assertEqual(sorted(path.name for path in (package / "notices").iterdir()), ["user file"])
                (package / "notices/user file").unlink()
                (package / "notices").rmdir()
                self.assertEqual(self.build(project, package).returncode, 0)
                (package / "notices/removed-crate-1.0.0").mkdir()
                (package / "notices/removed-crate-1.0.0/LICENSE").write_text("Stale terms\n")
                result = self.build(project, package)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(sorted(path.name for path in (package / "notices").iterdir()),
                                 [".midnight-generated", "LICENSE"])

    def wrapper(self):
        (self.root / "scripts").mkdir()
        shutil.copyfile(ROOT / "scripts/build-viewers.sh", self.root / "scripts/build-viewers.sh")
        logger = self.root / "log.py"
        logger.write_text("import json, os, sys\n"
                          "with open(os.environ['VIEWER_TEST_LOG'], 'a') as log:\n"
                          "    log.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                          "if sys.argv[1] == 'html' and os.environ.get('VIEWER_TEST_HTML_FAILURE'):\n"
                          "    raise SystemExit(51)\n")
        for name in ("basic", "source", "markdown"):
            project = self.root / "plugins" / f"midnight-viewer-{name}"
            (project / "scripts").mkdir(parents=True)
            (project / "Cargo.toml").write_text("# fixture\n")
            (project / "scripts/build.sh").write_text(
                f'#!/bin/bash\nexec python3 "{logger}" {name} "$@"\n')
        for name, arguments in (("image-viewer", '"$4" "$1"'), ("html-viewer", 'html "$1"')):
            (self.root / f"scripts/build-{name}.sh").write_text(
                f'#!/bin/bash\nexec python3 "{logger}" {arguments}\n')

    def test_normal_build_forwards_debug_and_requires_html_after_other_viewers(self):
        self.wrapper()
        result = subprocess.run(["bash", str(self.root / "scripts/build-viewers.sh"),
                                 "debug", str(self.config)], env=self.environment,
                                text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        builds = [row for row in self.calls() if row[0] != "cargo"]
        self.assertEqual([row[0] for row in builds], ["basic", "source", "markdown", "image", "pdf", "html"])
        self.assertTrue(all(row[-1] == "debug" for row in builds[:3]))
        self.assertTrue(all(row[1] == "debug" for row in builds[3:]))

    def test_missing_html_does_not_report_successful_normal_viewer_build(self):
        self.wrapper()
        self.environment["VIEWER_TEST_HTML_FAILURE"] = "1"
        result = subprocess.run(["bash", str(self.root / "scripts/build-viewers.sh"),
                                 "debug", str(self.config)], env=self.environment,
                                text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 51, result.stdout + result.stderr)


@unittest.skipUnless(os.name == "nt", "Windows viewer build helpers")
class ViewerBuildNoticesWindows(unittest.TestCase):
    """build.ps1 replaces only notices that it generated or that lie in a _build tree."""

    DRIVER = """param([Parameter(Mandatory)][string]$Cases, [Parameter(Mandatory)][string]$Results)
$outcomes = [ordered]@{}
foreach ($case in (Get-Content -LiteralPath $Cases -Raw | ConvertFrom-Json)) {
    $script = $case.script
    try {
        & $script -SdkConfig $case.config -BuildRoot $case.build -OutputDirectory $case.package | Out-Null
        $outcomes[$case.name] = $null
    } catch {
        $outcomes[$case.name] = $_.Exception.Message
    }
}
[IO.File]::WriteAllText($Results, ($outcomes | ConvertTo-Json -Compress))
"""

    def test_notices_are_replaced_only_when_generated_or_below_build(self):
        import _winapi  # A junction needs no symlink privilege.
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="viewer notices ", dir=temporary)
        self.addCleanup(owner.cleanup)
        root = Path(owner.name)
        outside = temporary_outside_build(self)
        tools = root / "bin"
        tools.mkdir()
        (tools / "rustc.cmd").write_text("@echo host: x86_64-pc-windows-msvc\n")
        (root / "cargo.py").write_text(
            "import pathlib, sys\n"
            "arguments = sys.argv[1:]\n"
            "manifest = pathlib.Path(arguments[arguments.index('--manifest-path') + 1])\n"
            "name = manifest.parent.name.removeprefix('midnight-viewer-')\n"
            "profile = 'release' if '--release' in arguments else 'debug'\n"
            "output = (pathlib.Path(arguments[arguments.index('--target-dir') + 1])\n"
            "          / arguments[arguments.index('--target') + 1] / profile)\n"
            "output.mkdir(parents=True, exist_ok=True)\n"
            "(output / f'midnight_viewer_{name}.dll').write_bytes(profile.encode())\n")
        (tools / "cargo.cmd").write_text(f'@"{sys.executable}" "{root / "cargo.py"}" %*\n')
        config = root / "sdk config.toml"
        config.write_text("# fixture\n")
        linked = root / "link target"
        linked.mkdir()
        (linked / "keep").write_text("user data\n")
        cases, packages = [], {}
        for name in ("basic", "source", "markdown"):
            project = root / "plugins" / f"midnight-viewer-{name}"
            (project / "scripts").mkdir(parents=True)
            shutil.copyfile(ROOT / f"plugins/midnight-viewer-{name}/scripts/build.ps1", project / "scripts/build.ps1")
            (project / "scripts/prepare-sdk.ps1").write_text("param($SdkConfig, $BuildRoot)\n$SdkConfig\n")
            for filename in ("Cargo.toml", "LICENSE", "UNICODE-LICENSE.txt", "README.md", "THIRD-PARTY.md"):
                (project / filename).write_text("Pinned fixture notice\n")
            (project / "plugin.toml").write_text('entry_library = "fixture"\ntarget = "fixture"\n')
            (project / "notices").mkdir()
            (project / "notices/LICENSE").write_text("Third-party terms\n")
            marker = f"midnight-viewer-{name} notices\n".encode()
            scenarios = {"legacy": root / "_build/app/viewers", "foreign": outside / "foreign",
                         "marked": outside / "marked"}
            if name == "basic":
                scenarios["linked"] = outside / "linked"
            for scenario, parent in scenarios.items():
                package = parent / f"midnight-viewer-{name}"
                notices = package / "notices"
                notices.mkdir(parents=True)
                if scenario == "foreign":
                    (notices / "user file").write_text("not generated\n")
                else:
                    # A notice of a dependency that the rebuilt plugin no longer ships.
                    (notices / "removed-crate-1.0.0").mkdir()
                    (notices / "removed-crate-1.0.0/LICENSE").write_text("Stale terms\n")
                if scenario in ("marked", "linked"):
                    (notices / ".midnight-generated").write_bytes(marker)
                if scenario == "linked":
                    _winapi.CreateJunction(str(linked), str(notices / "redirect"))
                    self.addCleanup(os.rmdir, notices / "redirect")
                packages[f"{name}-{scenario}"] = package
                cases.append({"name": f"{name}-{scenario}", "script": str(project / "scripts/build.ps1"),
                              "config": str(config), "build": str(root / "_build/native" / name),
                              "package": str(package)})
        (root / "cases.json").write_text(json.dumps(cases))
        (root / "driver.ps1").write_text(self.DRIVER)
        shell = shutil.which("pwsh") or "powershell.exe"
        result = subprocess.run(
            [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(root / "driver.ps1"),
             "-Cases", str(root / "cases.json"), "-Results", str(root / "results.json")],
            env=dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"]),
            capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        outcomes = json.loads((root / "results.json").read_text(encoding="utf-8-sig"))

        def entries(name):
            return sorted(path.name for path in (packages[name] / "notices").iterdir())

        for name in ("basic", "source", "markdown"):
            with self.subTest(plugin=name):
                for scenario in ("legacy", "marked"):
                    self.assertIsNone(outcomes[f"{name}-{scenario}"], scenario)
                    self.assertEqual(entries(f"{name}-{scenario}"), [".midnight-generated", "LICENSE"])
                    self.assertEqual((packages[f"{name}-{scenario}"] / "notices/.midnight-generated").read_bytes(),
                                     f"midnight-viewer-{name} notices\n".encode())
                self.assertEqual((packages[f"{name}-legacy"] / f"midnight_viewer_{name}.dll").read_bytes(), b"debug")
                self.assertIn("did not generate", outcomes[f"{name}-foreign"])
                self.assertEqual(entries(f"{name}-foreign"), ["user file"])
        self.assertIn("must not contain links", outcomes["basic-linked"])
        self.assertEqual(entries("basic-linked"), [".midnight-generated", "redirect", "removed-crate-1.0.0"])
        self.assertEqual((linked / "keep").read_text(), "user data\n")


@unittest.skipUnless(os.name == "nt", "Windows viewer build wrapper")
class ViewerSdkWindows(unittest.TestCase):
    """The real wrapper, API packaging script and Cargo wrapper; Cargo and the viewer builds are stubs."""

    def test_viewer_sdk_below_a_non_ascii_path_survives_a_legacy_console_code_page(self):
        # midnight-api-package prints its bundle path in UTF-8, but PowerShell decodes native
        # output with the console code page, so the SDK below a non-ASCII checkout was lost.
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="viewer sdk ", dir=temporary)
        self.addCleanup(owner.cleanup)
        owner = Path(owner.name)
        # cmd.exe reads a batch file in the console code page: keep the stubs on an ASCII path.
        tools = owner / "bin"
        tools.mkdir()
        log = owner / "cargo.jsonl"
        (tools / "cargo.py").write_text(
            "import json, os, pathlib, sys\n"
            "with open(os.environ['VIEWERS_TEST_LOG'], 'a', encoding='utf-8') as log:\n"
            "    log.write(json.dumps(sys.argv[1:]) + '\\n')\n"
            "if 'run' in sys.argv:\n"
            "    # Like midnight-api-package: a new bundle, reported in UTF-8 whatever the code page.\n"
            "    bundle = pathlib.Path(sys.argv[-1]) / '_build/api-release/midnight-api-1'\n"
            "    (bundle / '.cargo').mkdir(parents=True)\n"
            "    (bundle / '.cargo/config.toml').write_text('# viewer SDK fixture\\n')\n"
            "    sys.stdout.buffer.write(b'Public API/SDK release bundle: ' + str(bundle).encode() + b'\\n')\n",
            encoding="utf-8")
        (tools / "cargo.cmd").write_text(f'@"{sys.executable}" "{tools / "cargo.py"}" %*\n')
        (tools / "rustc.cmd").write_text("@echo host: x86_64-pc-windows-msvc\n")
        checkout = owner / "Jiří 文档/checkout"
        for relative in ("scripts/build-viewers.ps1", "scripts/cargo.ps1", "scripts/utf8-native-output.ps1",
                         "api/scripts/package.ps1"):
            (checkout / relative).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, checkout / relative)
        builds = owner / "builds.txt"
        for name in ("basic", "source", "markdown"):
            scripts = checkout / f"plugins/midnight-viewer-{name}/scripts"
            scripts.mkdir(parents=True)
            (scripts / "build.ps1").write_text(
                "param($SdkConfig, $BuildRoot, $OutputDirectory, $Profile)\n"
                "[IO.File]::AppendAllText($env:VIEWERS_TEST_BUILDS, $SdkConfig + \"`n\")\n", encoding="utf-8")
        for name in ("build-image-viewer.ps1", "build-html-viewer.ps1"):
            (checkout / "scripts" / name).write_text(
                "param($Viewer, $Profile, $WorkerSdkConfig, $AppDirectory, [switch]$Test, [switch]$Offline)\n",
                encoding="utf-8")
        report = owner / "code-page.txt"
        driver = owner / "driver.ps1"
        driver.write_text(
            "param([string]$Script, [string]$Report)\n"
            "[Console]::OutputEncoding = [Text.Encoding]::GetEncoding(850)\n"
            "try { & $Script }\n"
            "finally { [IO.File]::WriteAllText($Report, [string][Console]::OutputEncoding.CodePage) }\n",
            encoding="utf-8")
        environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                           VIEWERS_TEST_LOG=str(log), VIEWERS_TEST_BUILDS=str(builds))
        for name in ("CARGO_TARGET_DIR", "MIDNIGHT_VIEWER_SDK_CONFIG", "MIDNIGHT_VIEWERS_OFFLINE"):
            environment.pop(name, None)
        # The code page belongs to the console, not to the process: CREATE_NO_WINDOW gives
        # the wrapper a console of its own and leaves the test runner's console untouched.
        result = subprocess.run(
            [shutil.which("pwsh") or "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(driver), "-Script", str(checkout / "scripts/build-viewers.ps1"), "-Report", str(report)],
            cwd=checkout, env=environment, capture_output=True, encoding="utf-8", errors="replace",
            timeout=120, creationflags=subprocess.CREATE_NO_WINDOW)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(report.read_text(), "850", "The caller's console encoding was not restored")
        config = checkout / "_build/api-release/midnight-api-1/.cargo/config.toml"
        self.assertEqual([Path(line) for line in builds.read_text(encoding="utf-8").splitlines()], [config] * 3)
        calls = [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]
        fetches = [call for call in calls if "--config" in call]
        self.assertEqual([Path(call[call.index("--config") + 1]) for call in fetches], [config] * 3)


if __name__ == "__main__":
    unittest.main()
