"""HTML build wrappers require the pinned runtime and preserve previous packages on failure."""
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


class WebSdkEnvironment(unittest.TestCase):
    @unittest.skipUnless(shutil.which("pwsh"), "PowerShell delegation requires pwsh")
    def test_powershell_delegates_to_the_same_python_packager_and_preserves_toolchain_environment(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary) as directory:
            root = Path(directory)
            tools = root / "bin"
            tools.mkdir()
            python = tools / "python"
            report = root / "delegation.json"
            python.write_text(f"#!{sys.executable}\nimport json,os,sys\n"
                              "from pathlib import Path\n"
                              "Path(os.environ['WEB_SDK_DELEGATION_REPORT']).write_text(json.dumps({\n"
                              "'args':sys.argv[1:],'cargo':os.environ['CARGO'],\n"
                              "'cargo_home':os.environ['CARGO_HOME'],'rustup_home':os.environ['RUSTUP_HOME']}))\n")
            python.chmod(0o755)
            output = root / "_build/release"
            environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                               CARGO="selected-cargo", CARGO_HOME=str(root / "cargo home"),
                               RUSTUP_HOME=str(root / "rustup home"), WEB_SDK_DELEGATION_REPORT=str(report))
            result = subprocess.run([shutil.which("pwsh"), "-NoProfile", "-File",
                                     str(ROOT / "web-viewer-api/scripts/package.ps1"), "-Output", str(output)],
                                    env=environment, text=True, capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            actual = json.loads(report.read_text())
            self.assertEqual(actual["args"], ["-B", str(ROOT / "web-viewer-api/scripts/package.py"), str(output)])
            self.assertEqual(actual["cargo"], environment["CARGO"])
            self.assertEqual(actual["cargo_home"], environment["CARGO_HOME"])
            self.assertEqual(actual["rustup_home"], environment["RUSTUP_HOME"])

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
            cargo.chmod(0o755)
            environment = dict(os.environ, CARGO=str(cargo), CARGO_HOME=str(root / "selected cargo"),
                               RUSTUP_HOME=str(root / "selected rustup"), WEB_SDK_ENV_REPORT=str(report))
            result = subprocess.run([sys.executable, "-B", str(scripts / "package.py")],
                                    env=environment, text=True, capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual([json.loads(line) for line in report.read_text().splitlines()],
                             [[environment["CARGO_HOME"], environment["RUSTUP_HOME"]]] * 2)


if __name__ == "__main__":
    unittest.main()
