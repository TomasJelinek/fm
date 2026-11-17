"""HTML build wrappers require the pinned runtime and preserve previous packages on failure."""
import hashlib
import json
import importlib.util
import io
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tarfile
import unittest

# Sibling fixture module, also when run as scripts.tests.<module> from the checkout.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from pe_image import unsigned_pe  # noqa: E402


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform == "linux", "Linux HTML build wrapper")
class HtmlViewerBuild(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="HTML build ", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "checkout & dollar$"
        (self.root / "scripts").mkdir(parents=True)
        source = ROOT / "scripts/build-html-viewer.sh"
        self.assertTrue(source.is_file(), "Mandatory HTML viewer wrapper is missing")
        shutil.copyfile(source, self.root / "scripts/build-html-viewer.sh")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "calls.jsonl"
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                                HTML_TEST_LOG=str(self.log))
        for name in ("CEF_PATH", "CARGO_TARGET_DIR", "MIDNIGHT_HTML_SDK_CONFIG", "MIDNIGHT_VIEWERS_OFFLINE"):
            self.environment.pop(name, None)
        self.config = self.root / "SDK config.toml"
        self.config.write_text("# Canonical public web SDK0.2\n")
        self.program(self.bin / "cargo", """
record('cargo', sys.argv[1:])
if os.environ.get('HTML_TEST_FETCH_FAILURE'): raise SystemExit(41)
""")
        self.program(self.root / "scripts/prepare-html-cef.py", """
record('cef', sys.argv[1:])
if os.environ.get('HTML_TEST_CEF_FAILURE'): raise SystemExit(42)
destination=pathlib.Path(sys.argv[sys.argv.index('--output')+1])
destination.mkdir(parents=True,exist_ok=True)
print(json.dumps({'cef_path': str(destination)}))
""")
        api = self.root / "web-viewer-api"
        (api / "scripts").mkdir(parents=True)
        (api / "Cargo.toml").write_text("# Workspace API0.2\n")
        self.program(api / "scripts/package.py", f"record('sdk', sys.argv[1:])\nprint({str(self.config)!r})\n")
        plugin = self.root / "plugins/midnight-viewer-html"
        (plugin / "scripts").mkdir(parents=True)
        (plugin / "Cargo.toml").write_text("# fixture\n")
        self.program(plugin / "scripts/build-fixture.py", """
record('build', sys.argv[1:])
if os.environ.get('HTML_TEST_BUILD_FAILURE'): raise SystemExit(43)
package=pathlib.Path(sys.argv[3])
package.mkdir()
(package/'web-viewer.json').write_text('{"id":"midnight.viewer.html","version":"0.2.0"}')
(package/'runtime-files.json').write_text('{"files":[]}')
(package/'midnight-viewer-html').write_text(sys.argv[2])
""")
        (plugin / "scripts/build-linux.sh").write_text(
            '#!/bin/bash\nexec python3 "$(dirname -- "$0")/build-fixture.py" "$@"\n')

    def program(self, path, body):
        path.write_text(f"#!{sys.executable}\nimport json, os, pathlib, sys\n"
                        "def record(name,args):\n"
                        "    with open(os.environ['HTML_TEST_LOG'],'a') as log:\n"
                        "        log.write(json.dumps([name,args])+'\\n')\n" + body)
        path.chmod(0o755)

    def calls(self, name):
        rows = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return [row[1] for row in rows if row[0] == name]

    def run_build(self, *arguments):
        return subprocess.run(["bash", str(self.root / "scripts/build-html-viewer.sh"), *arguments],
                              cwd=self.root.parent, env=self.environment, text=True, capture_output=True, timeout=20)

    def test_debug_default_caches_matching_web_sdk_and_rebuilds_into_fresh_package(self):
        for _ in range(2):
            result = self.run_build()
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(self.calls("sdk")), 1)
        builds = self.calls("build")
        self.assertEqual([build[1] for build in builds], ["debug", "debug"])
        self.assertNotEqual(builds[0][2], builds[1][2])
        package = self.root / "_build/debug/viewers/midnight-viewer-html"
        self.assertEqual((package / "midnight-viewer-html").read_text(), "debug")
        (self.root / "web-viewer-api/Cargo.toml").write_text("# Updated public API\n")
        self.assertEqual(self.run_build().returncode, 0)
        self.assertEqual(len(self.calls("sdk")), 2)

    def test_explicit_profile_sdk_destination_and_offline_are_preserved(self):
        self.environment["MIDNIGHT_VIEWERS_OFFLINE"] = "1"
        destination = self.root / "_build/custom app"
        result = self.run_build("release", str(self.config), str(destination))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.calls("sdk"), [])
        self.assertTrue(all("--offline" in call for call in self.calls("cargo")))
        self.assertIn("--offline", self.calls("cef")[0])
        self.assertEqual(self.calls("build")[0][1], "release")
        self.assertEqual((destination / "viewers/midnight-viewer-html/midnight-viewer-html").read_text(), "release")

    def test_failed_build_preserves_the_previous_complete_module(self):
        destination = self.root / "_build/debug/viewers/midnight-viewer-html"
        destination.mkdir(parents=True)
        (destination / "existing-module").write_text("Keep previous valid module\n")
        self.environment["HTML_TEST_BUILD_FAILURE"] = "1"
        result = self.run_build("debug", str(self.config))
        self.assertEqual(result.returncode, 43, result.stdout + result.stderr)
        self.assertEqual((destination / "existing-module").read_text(), "Keep previous valid module\n")
        self.assertEqual(list(destination.iterdir()), [destination / "existing-module"])

    def test_cef_preparation_failure_prevents_build_and_installation(self):
        self.environment["HTML_TEST_CEF_FAILURE"] = "1"
        result = self.run_build("debug", str(self.config))
        self.assertEqual(result.returncode, 42, result.stdout + result.stderr)
        self.assertEqual(self.calls("build"), [])

    def test_publication_and_rollback_failure_retain_previous_module_in_owned_staging(self):
        destination = self.root / "_build/debug/viewers/midnight-viewer-html"
        destination.mkdir(parents=True)
        (destination / "existing-module").write_text("Keep previous valid module\n")
        self.program(self.bin / "mv", """
arguments = [argument for argument in sys.argv[1:] if argument != '--']
source, destination = map(pathlib.Path, arguments)
if source.name in ('package', 'previous'): raise SystemExit(55)
source.rename(destination)
""")
        result = self.run_build("debug", str(self.config))
        self.assertEqual(result.returncode, 55, result.stdout + result.stderr)
        retained = list((self.root / "_build/debug").glob(".html-build.*/previous/existing-module"))
        self.assertEqual(len(retained), 1, result.stdout + result.stderr)
        self.assertEqual(retained[0].read_text(), "Keep previous valid module\n")
        self.assertIn(str(retained[0].parent), result.stderr)


@unittest.skipUnless(os.name == "nt", "Windows HTML build wrapper")
class HtmlViewerBuildWindows(unittest.TestCase):
    """The real wrapper, provider build and packaging scripts; only Cargo and CEF are stubs.

    cl.exe cannot compile CEF's wrapper below a deep checkout's default target, so
    MIDNIGHT_HTML_TARGET_DIR selects a short Cargo target directory. The verified,
    flattened CEF SDK stays under _build and is used in place.
    """

    CEF_FILES = ("libcef.dll", "chrome_elf.dll", "icudtl.dat", "resources.pak", "chrome_100_percent.pak",
                 "chrome_200_percent.pak", "v8_context_snapshot.bin", "dxcompiler.dll", "vk_swiftshader.dll",
                 "vk_swiftshader_icd.json", "vulkan-1.dll", "locales/en-US.pak", "LICENSE.txt", "CREDITS.html",
                 "include/cef_version.h", "archive.json")

    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix="html build ", dir=temporary)
        self.addCleanup(owner.cleanup)
        self.root = Path(owner.name)
        (self.root / "scripts").mkdir()
        shutil.copyfile(ROOT / "scripts/build-html-viewer.ps1", self.root / "scripts/build-html-viewer.ps1")
        plugin = self.root / "plugins/midnight-viewer-html"
        for relative in ("scripts/build-windows.ps1", "scripts/package-windows.ps1", "web-viewer.json",
                         "THIRD_PARTY.md", "README.md", "Cargo.toml", "Cargo.lock", ".cargo/config.toml"):
            (plugin / relative).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / "plugins/midnight-viewer-html" / relative, plugin / relative)
        (plugin / "notices/cef-rs").mkdir(parents=True)
        (plugin / "notices/cef-rs/LICENSE").write_text("Pinned notice fixture\n")
        # The flattened SDK and receipt that scripts/prepare-html-cef.py verifies.
        self.sdk = self.root / "_build/html-viewer/provider-target/windows/cef-sdk"
        (self.sdk / "locales").mkdir(parents=True)
        (self.sdk / "include").mkdir()
        self.bootstrap = unsigned_pe(trailer=b"CEF sandbox bootstrap")
        (self.sdk / "bootstrap.exe").write_bytes(self.bootstrap)
        for name in self.CEF_FILES:
            (self.sdk / name).write_bytes(f"CEF fixture {name}\n".encode())
        files = {path.relative_to(self.sdk).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                 for path in self.sdk.rglob("*") if path.is_file()}
        self.sdk.with_name("cef-sdk.verified.json").write_text(json.dumps({"archive_sha256": "0" * 64, "files": files}))
        (self.root / "scripts/prepare-html-cef.py").write_text(
            "import json, pathlib, sys\n"
            "output = pathlib.Path(sys.argv[sys.argv.index('--output') + 1])\n"
            "print(json.dumps({'cef_path': str(output)}))\n")
        tools = self.root / "bin"
        tools.mkdir()
        self.log = self.root / "cargo.jsonl"
        self.library = self.root / "provider.dll"
        self.library.write_bytes(unsigned_pe(True, b"first build"))
        (tools / "cargo.py").write_text(
            "import json, os, pathlib, sys\n"
            "arguments = sys.argv[1:]\n"
            "with open(os.environ['HTML_TEST_LOG'], 'a') as log:\n"
            "    log.write(json.dumps({'arguments': arguments, 'cef_path': os.environ.get('CEF_PATH')}) + '\\n')\n"
            "if 'metadata' in arguments:\n"
            "    print(json.dumps({'packages': []}))\n"
            "elif 'build' in arguments:\n"
            "    output = pathlib.Path(arguments[arguments.index('--target-dir') + 1]) / 'debug'\n"
            "    output.mkdir(parents=True, exist_ok=True)\n"
            "    library = pathlib.Path(os.environ['HTML_TEST_LIBRARY']).read_bytes()\n"
            "    (output / 'midnight_viewer_html.dll').write_bytes(library)\n")
        (tools / "cargo.cmd").write_text(f'@"{sys.executable}" "{tools / "cargo.py"}" %*\n')
        self.config = self.root / "sdk.toml"
        self.config.write_text("# web SDK fixture\n")
        self.environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                                HTML_TEST_LOG=str(self.log), HTML_TEST_LIBRARY=str(self.library))
        for name in ("MIDNIGHT_HTML_TARGET_DIR", "MIDNIGHT_HTML_SDK_CONFIG", "MIDNIGHT_VIEWERS_OFFLINE",
                     "CARGO_TARGET_DIR", "CEF_PATH"):
            self.environment.pop(name, None)
        self.package = self.root / "_build/debug/viewers/midnight-viewer-html"

    def build(self, **environment):
        self.log.unlink(missing_ok=True)
        result = subprocess.run(
            [shutil.which("pwsh") or "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(self.root / "scripts/build-html-viewer.ps1"), "-SdkConfig", str(self.config),
             "-AppDirectory", str(self.root / "_build/debug")],
            cwd=self.root, env=dict(self.environment, **environment), capture_output=True, text=True, timeout=120)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result.returncode, result.stdout + result.stderr, calls

    def assert_package(self, library):
        self.assertEqual((self.package / "midnight-viewer-html.exe").read_bytes(), self.bootstrap)
        self.assertEqual((self.package / "midnight-viewer-html.dll").read_bytes(), library)
        self.assertEqual((self.package / "libcef.dll").read_bytes(), b"CEF fixture libcef.dll\n")
        inventory = json.loads((self.package / "runtime-files.json").read_text(encoding="utf-8-sig"))
        self.assertIn("locales/en-US.pak", {entry["path"] for entry in inventory["files"]})

    def test_short_target_builds_from_the_verified_sdk_and_unchanged_inputs_keep_the_package(self):
        import _winapi  # A junction needs no symlink privilege.
        default = self.sdk.parent
        code, output, calls = self.build()
        self.assertEqual(code, 0, output)
        build = next(call for call in calls if "build" in call["arguments"])
        self.assertEqual(Path(build["arguments"][build["arguments"].index("--target-dir") + 1]), default)
        self.assertEqual(Path(build["cef_path"]), self.sdk)
        self.assert_package(self.library.read_bytes())
        if len(str(default)) > 80:
            self.assertIn("MIDNIGHT_HTML_TARGET_DIR", output)
        # Unchanged inputs: the build runs, the installed package is kept.
        code, output, calls = self.build()
        self.assertEqual(code, 0, output)
        self.assertIn("HTML viewer package is current", output)
        self.assertFalse(any("metadata" in call["arguments"] for call in calls))
        # A short target through a junction into the checkout's _build, as documented.
        (self.root / "_build/mh").mkdir()
        outside = tempfile.TemporaryDirectory(prefix="mh ", dir=ROOT / "_build/tmp")
        self.addCleanup(outside.cleanup)
        short = Path(outside.name) / "mh"
        _winapi.CreateJunction(str(self.root / "_build/mh"), str(short))
        self.addCleanup(os.rmdir, short)
        self.library.write_bytes(unsigned_pe(True, b"second build"))
        code, output, calls = self.build(MIDNIGHT_HTML_TARGET_DIR=str(short))
        self.assertEqual(code, 0, output)
        build = next(call for call in calls if "build" in call["arguments"])
        self.assertEqual(Path(build["arguments"][build["arguments"].index("--target-dir") + 1]), short)
        self.assertEqual(Path(build["cef_path"]), self.sdk)
        self.assertEqual(sorted(path.name for path in (self.root / "_build/mh").iterdir()),
                         ["debug", "dependency-metadata.json"])
        self.assert_package(self.library.read_bytes())
        if len(str(short)) <= 80:
            self.assertNotIn("MIDNIGHT_HTML_TARGET_DIR", output)

    def test_packaged_cef_files_must_match_the_sdk_receipt(self):
        code, output, _ = self.build()
        self.assertEqual(code, 0, output)
        installed = (self.package / "runtime-files.json").read_bytes()
        (self.sdk / "libcef.dll").write_bytes(b"changed after verification\n")
        self.library.write_bytes(self.library.read_bytes() + b"rebuilt")
        code, output, _ = self.build()
        self.assertNotEqual(code, 0)
        self.assertIn("differs from its SDK receipt: libcef.dll", output)
        self.assertEqual((self.package / "runtime-files.json").read_bytes(), installed)

    def test_standalone_distribution_is_flattened_once_and_packaged_against_its_receipt(self):
        distribution =self.root / "_build/html-viewer/cef/distribution"
        layout = {"Release": ("libcef.dll", "chrome_elf.dll", "v8_context_snapshot.bin", "dxcompiler.dll",
                              "vk_swiftshader.dll", "vk_swiftshader_icd.json", "vulkan-1.dll"),
                  "Resources": ("icudtl.dat", "resources.pak", "chrome_100_percent.pak", "chrome_200_percent.pak",
                                "locales/en-US.pak"),
                  "": ("include/cef_version.h", "cmake/cef_variables.cmake", "libcef_dll/wrapper.cc",
                       "CMakeLists.txt", "LICENSE.txt", "CREDITS.html")}
        for directory, names in layout.items():
            for name in names:
                path = distribution / directory / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(f"distribution {name}\n".encode())
        (distribution / "Release/bootstrap.exe").write_bytes(self.bootstrap)
        target = self.root / "_build/standalone-target"
        stage = target / "cef-sdk"

        def build():
            self.log.unlink(missing_ok=True)
            result = subprocess.run(
                [shutil.which("pwsh") or "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                 str(self.root / "plugins/midnight-viewer-html/scripts/build-windows.ps1"),
                 "-SdkConfig", str(self.config), "-CefDistribution", str(distribution),
                 "-TargetDirectory", str(target), "-OutputDirectory", str(self.root / "_build/standalone-package")],
                cwd=self.root, env=self.environment, capture_output=True, text=True, timeout=120)
            calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
            return result.returncode, result.stdout + result.stderr, calls

        code, output, calls = build()
        self.assertEqual(code, 0, output)
        self.assertEqual(Path(next(call for call in calls if "build" in call["arguments"])["cef_path"]), stage)
        self.assertEqual((stage / "locales/en-US.pak").read_bytes(), b"distribution locales/en-US.pak\n")
        self.assertTrue(stage.with_name("cef-sdk.staged.json").is_file())
        package = self.root / "_build/standalone-package"
        self.assertEqual((package / "midnight-viewer-html.exe").read_bytes(), self.bootstrap)
        # The staged SDK is reused; a changed copy no longer matches its receipt.
        (stage / "libcef.dll").write_bytes(b"changed after staging\n")
        code, output, _ = build()
        self.assertNotEqual(code, 0)
        self.assertIn("differs from its SDK receipt: libcef.dll", output)

    def test_target_override_must_be_an_absolute_plain_path_outside_the_sources(self):
        values = ["mh", r"D:\mh\..\other", r"D:\name with dot.", str(self.root / "plugins/target")]
        # One PowerShell process tries every value; the wrapper restores its environment.
        (self.root / "values.json").write_text(json.dumps(values))
        (self.root / "refusals.ps1").write_text(
            "param([string]$Values, [string]$Results)\n"
            "$messages = @()\n"
            "foreach ($value in (Get-Content -LiteralPath $Values -Raw | ConvertFrom-Json)) {\n"
            "    $env:MIDNIGHT_HTML_TARGET_DIR = $value\n"
            "    try { & (Join-Path $PSScriptRoot 'scripts/build-html-viewer.ps1') -SdkConfig $env:HTML_TEST_CONFIG "
            "-AppDirectory (Join-Path $PSScriptRoot '_build/debug') | Out-Null; $messages += $null }\n"
            "    catch { $messages += $_.Exception.Message }\n"
            "}\n"
            "[IO.File]::WriteAllText($Results, (ConvertTo-Json @($messages) -Compress))\n")
        result = subprocess.run(
            [shutil.which("pwsh") or "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(self.root / "refusals.ps1"), "-Values", str(self.root / "values.json"),
             "-Results", str(self.root / "refusals.json")],
            cwd=self.root, env=dict(self.environment, HTML_TEST_CONFIG=str(self.config)),
            capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        messages = json.loads((self.root / "refusals.json").read_text(encoding="utf-8-sig"))
        for value, message in zip(values, messages, strict=True):
            with self.subTest(value=value):
                self.assertIn("MIDNIGHT_HTML_TARGET_DIR", message or "")
        self.assertFalse(self.log.exists(), "A refused target reached Cargo")


class LinuxHtmlManifest(unittest.TestCase):
    def test_linux_package_keeps_the_project_manifest_identity(self):
        # The Linux package once synthesized a stale "Chromium HTML" name and its own
        # extension list; only the platform entry point may differ from Windows.
        spec = importlib.util.spec_from_file_location(
            "html_linux_packager", ROOT / "plugins/midnight-viewer-html/scripts/package-linux.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        project = json.loads((ROOT / "plugins/midnight-viewer-html/web-viewer.json").read_text())
        manifest = module.linux_manifest()
        self.assertEqual(manifest, dict(project, target="x86_64-unknown-linux-gnu",
                                        executable="midnight-viewer-html"))
        self.assertEqual(manifest["name"], "HTML viewer")


class WebSdkEnvironment(unittest.TestCase):
    HOMES = ("CARGO", "CARGO_HOME", "RUSTUP_HOME")

    def packager_fixture(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        directory = tempfile.TemporaryDirectory(prefix="web SDK delegation ", dir=temporary)
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "web-viewer-api/scripts").mkdir(parents=True)
        shutil.copyfile(ROOT / "web-viewer-api/scripts/package.ps1", root / "web-viewer-api/scripts/package.ps1")
        return root

    def run_powershell_packager(self, root, environment, zlib_ng=()):
        """Run package.ps1 inside a caller session that reports its environment afterwards."""
        shell = shutil.which("pwsh") or (shutil.which("powershell.exe") if os.name == "nt" else None)
        if not shell:
            self.skipTest("PowerShell delegation requires pwsh")
        tools = root / "bin"
        tools.mkdir()
        # The documented py launcher is preferred over python, which may be a Store alias.
        # Its -c probes answer which interpreter versions use zlib-ng.
        python = tools / ("py-fixture.py" if os.name == "nt" else "py")
        report = root / "delegation.json"
        python.write_text(
            f"#!{sys.executable}\nimport json, os, sys\nfrom pathlib import Path\n"
            "arguments = sys.argv[1:]\n"
            "if arguments[1:2] == ['-c']:\n"
            f"    sys.exit(1 if arguments[0] in {list(zlib_ng)!r} else 0)\n"
            "Path(os.environ['WEB_SDK_DELEGATION_REPORT']).write_text(json.dumps(dict(\n"
            f"    args=arguments, **{{name: os.environ.get(name) for name in {list(self.HOMES)!r}}})))\n")
        if os.name == "nt":
            (tools / "py.cmd").write_text(f'@echo off\n"{sys.executable}" "{python}" %*\n', encoding="utf-8")
        else:
            python.chmod(0o755)
        driver = root / "driver.ps1"
        driver.write_text(
            "param([string]$Script, [string]$Output, [string]$After)\n"
            "& $Script -Output $Output\n"
            "$state = [ordered]@{}\n"
            f"foreach ($name in @({', '.join(repr(name) for name in self.HOMES)})) {{\n"
            "    $state[$name] = [Environment]::GetEnvironmentVariable($name)\n"
            "}\n"
            "[IO.File]::WriteAllText($After, ($state | ConvertTo-Json -Compress))\n")
        output = root / "_build/release"
        after = root / "after.json"
        result = subprocess.run(
            [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(driver),
             "-Script", str(root / "web-viewer-api/scripts/package.ps1"), "-Output", str(output), "-After", str(after)],
            env=dict(environment, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                     WEB_SDK_DELEGATION_REPORT=str(report)),
            text=True, capture_output=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        delegated = json.loads(report.read_text())
        arguments = delegated.pop("args")
        self.assertEqual(arguments[1:], ["-B", str(root / "web-viewer-api/scripts/package.py"), str(output)])
        return arguments[0], delegated, json.loads(after.read_text(encoding="utf-8-sig"))

    def test_powershell_delegates_to_the_same_python_packager_and_preserves_toolchain_environment(self):
        root = self.packager_fixture()
        supplied = {"CARGO": "selected-cargo", "CARGO_HOME": str(root / "cargo home"),
                    "RUSTUP_HOME": str(root / "rustup home")}
        version, delegated, after = self.run_powershell_packager(root, dict(os.environ, **supplied))
        self.assertEqual(version, "-3")
        self.assertEqual(delegated, supplied)
        self.assertEqual(after, supplied)

    @unittest.skipUnless(os.name == "nt", "Windows local toolchain layout")
    def test_powershell_prefers_classic_zlib_python_and_restores_the_caller_environment(self):
        # py -3 is a zlib-ng Python here: the packager selects -3.13, gives it the
        # repository-local toolchain and leaves the calling build session unchanged.
        root = self.packager_fixture()
        (root / ".tools/cargo/bin").mkdir(parents=True)
        (root / ".tools/cargo/bin/cargo.exe").write_bytes(b"local toolchain fixture")
        environment = {name: value for name, value in os.environ.items() if name.upper() not in self.HOMES}
        version, delegated, after = self.run_powershell_packager(root, environment, zlib_ng=("-3",))
        self.assertEqual(version, "-3.13")
        self.assertEqual({name: Path(value) for name, value in delegated.items()},
                         {"CARGO": root / ".tools/cargo/bin/cargo.exe", "CARGO_HOME": root / ".tools/cargo",
                          "RUSTUP_HOME": root / ".tools/rustup"})
        self.assertEqual(after, dict.fromkeys(self.HOMES))

    def test_zlib_ng_is_refused_before_it_can_produce_unpinned_archives(self):
        # The canonical tar is identical on every Python, but zlib-ng (Python 3.14+ on
        # Windows) deflates it to other bytes than the classic-zlib archives the HTML
        # viewer pins, so packaging must stop with an actionable message instead.
        spec = importlib.util.spec_from_file_location("web_sdk_packager", ROOT / "web-viewer-api/scripts/package.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        module.require_classic_zlib("1.3")
        module.require_classic_zlib("1.2.13")
        with self.assertRaisesRegex(SystemExit, "zlib-ng"):
            module.require_classic_zlib("1.3.1.zlib-ng")

    def test_canonical_archive_is_independent_of_provenance_order_time_and_gzip_platform(self):
        spec = importlib.util.spec_from_file_location("web_sdk_packager", ROOT / "web-viewer-api/scripts/package.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory(dir=ROOT / "_build/tmp") as directory:
            results = []
            for order in (False, True):
                path = Path(directory) / f"fixture-{order}.crate"
                entries = [("p-0.2.0/src/data.bin", b"\x00\xff\r\n"),
                           ("p-0.2.0/.cargo_vcs_info.json", b"changed" if order else b"original")]
                if order:
                    entries.reverse()
                with tarfile.open(path, "w:gz") as archive:
                    for name, contents in entries:
                        info = tarfile.TarInfo(name)
                        info.size = len(contents)
                        info.mode = 0o600 if order else 0o644
                        info.mtime = 123 if order else 456
                        archive.addfile(info, io.BytesIO(contents))
                results.append(module.canonical_package(path, "p-0.2.0")[0])
            self.assertEqual(results[0], results[1])
            self.assertEqual(results[0][9], 255, "Released gzip header must not encode the packaging OS")
            with tarfile.open(fileobj=io.BytesIO(results[0]), mode="r:gz") as archive:
                self.assertEqual(archive.getnames(), ["p-0.2.0/src/data.bin"])
                self.assertEqual(archive.extractfile(archive.getmembers()[0]).read(), b"\x00\xff\r\n")

    def test_existing_packager_preserves_supplied_cargo_and_toolchain_homes(self):
        self.assert_packager_homes(supplied=True, local_toolchain=True)

    def test_unset_homes_stay_unset_without_a_local_toolchain(self):
        # A standard rustup installation exports neither home; redirecting them to
        # an absent .tools tree would download a second toolchain into the checkout.
        self.assert_packager_homes(supplied=False, local_toolchain=False)

    def test_unset_homes_select_an_existing_local_toolchain(self):
        self.assert_packager_homes(supplied=False, local_toolchain=True)

    def assert_packager_homes(self, *, supplied, local_toolchain):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="web SDK env ", dir=temporary) as directory:
            root = Path(directory)
            scripts = root / "web-viewer-api/scripts"
            scripts.mkdir(parents=True)
            shutil.copyfile(ROOT / "web-viewer-api/scripts/package.py", scripts / "package.py")
            (scripts.parent / "Cargo.toml").write_text('[workspace.package]\nversion="0.2.0"\n')
            cargo = root / "inert-cargo"
            report = root / "environment.jsonl"
            cargo.write_text(f"#!{sys.executable}\n" + '''import io,json,os,pathlib,sys,tarfile
with open(os.environ['WEB_SDK_ENV_REPORT'],'a') as output:
    output.write(json.dumps([os.environ.get('CARGO_HOME'),os.environ.get('RUSTUP_HOME')])+'\\n')
name=sys.argv[sys.argv.index('-p')+1]
target=pathlib.Path(sys.argv[sys.argv.index('--target-dir')+1])/'package'
target.mkdir(parents=True,exist_ok=True)
prefix=name+'-0.2.0'
contents=('[package]\\nname="'+name+'"\\nversion="0.2.0"\\nrust-version="1.99"\\n').encode()
with tarfile.open(target/(prefix+'.crate'),'w:gz') as archive:
    member=tarfile.TarInfo(prefix+'/Cargo.toml');member.size=len(contents)
    archive.addfile(member,io.BytesIO(contents))
''')
            if os.name == "nt":
                launcher = root / "inert-cargo.cmd"
                launcher.write_text(
                    f'@echo off\n"{sys.executable}" "{cargo}" %*\n', encoding="utf-8")
                cargo = launcher
            else:
                cargo.chmod(0o755)
            if local_toolchain:
                local = root / ".tools/cargo/bin" / ("cargo.exe" if os.name == "nt" else "cargo")
                local.parent.mkdir(parents=True)
                local.write_bytes(b"local toolchain fixture")
            environment = dict(os.environ, CARGO=str(cargo), WEB_SDK_ENV_REPORT=str(report))
            for name in ("CARGO_HOME", "RUSTUP_HOME"):
                environment.pop(name, None)
            if supplied:
                environment.update(CARGO_HOME=str(root / "selected cargo"),
                                   RUSTUP_HOME=str(root / "selected rustup"))
                expected = [environment["CARGO_HOME"], environment["RUSTUP_HOME"]]
            elif local_toolchain:
                expected = [str(root / ".tools/cargo"), str(root / ".tools/rustup")]
            else:
                expected = [None, None]
            # Environment propagation, not the archive bytes, is under test: also run on
            # zlib-ng Pythons, which the packager otherwise refuses.
            runner = ("import importlib.util, sys\n"
                      "spec = importlib.util.spec_from_file_location('web_sdk_packager', sys.argv[1])\n"
                      "module = importlib.util.module_from_spec(spec)\n"
                      "spec.loader.exec_module(module)\n"
                      "module.require_classic_zlib = lambda version=None: None\n"
                      "sys.argv = sys.argv[1:]\n"
                      "module.main()\n")
            result = subprocess.run([sys.executable, "-B", "-c", runner, str(scripts / "package.py")],
                                    env=environment, text=True, capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual([json.loads(line) for line in report.read_text().splitlines()],
                             [expected] * 2)


if __name__ == "__main__":
    unittest.main()
