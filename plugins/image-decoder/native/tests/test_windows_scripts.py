"""Windows native wrappers keep paths below a non-ASCII directory intact."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

NATIVE = Path(__file__).resolve().parents[1]
PWSH = shutil.which('pwsh')

# The py launcher as Python before 3.15 behaves: UTF-8 mode stays off, so pipes get the
# ANSI code page, unless the command line asks for it (the first -X utf8 option wins).
PY_LAUNCHER = '''import subprocess, sys
arguments = sys.argv[1:]
if arguments[:1] == ['-3']:
    arguments = arguments[1:]
script = next(index for index, argument in enumerate(arguments) if argument.endswith('.py'))
command = [sys.executable, *arguments[:script], '-X', 'utf8=0', *arguments[script:]]
sys.exit(subprocess.run(command).returncode)
'''

# Only the presence of the MSVC shell's tools matters before the private tools exist.
DRIVER = '''param([string]$Script, [string]$BuildRoot, [string]$Report)
$ErrorActionPreference = 'Stop'
$env:VSCMD_ARG_TGT_ARCH = 'x64'
foreach ($tool in @('cl.exe', 'ninja.exe', 'nasm.exe')) {
    Set-Item -Path "function:global:$tool" -Value { $global:LASTEXITCODE = 0 }
}
[Console]::OutputEncoding = [Text.Encoding]::GetEncoding(850)
try {
    $output = & $Script -BuildRoot $BuildRoot -Offline
} finally {
    $state = @{ code_page = [Console]::OutputEncoding.CodePage; output = @($output)
        tools = $env:MIDNIGHT_IMAGE_NATIVE_TOOLS; temporary = $env:TEMP
        library = $env:SYSTEM_DEPS_DAV1D_SEARCH_NATIVE }
    [IO.File]::WriteAllText($Report, ($state | ConvertTo-Json))
}
'''

# Like the pinned tools: print the prepared directory; Meson refuses a missing source.
PREPARE_SOURCE = '''import pathlib, sys
source = pathlib.Path(sys.argv[1]) / 'dav1d-1.5.4'
source.mkdir(parents=True, exist_ok=True)
print(source)
'''
BOOTSTRAP_TOOLS = '''import pathlib, sys
root = pathlib.Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=True)
print(root)
'''
PRIVATE_MESON = '''import json, os, pathlib, sys
arguments = sys.argv[1:]
with open(os.environ['NATIVE_TEST_LOG'], 'a', encoding='utf-8') as log:
    log.write(json.dumps(arguments) + '\\n')
if arguments[0] == 'setup':
    if not pathlib.Path(arguments[2]).is_dir():
        sys.exit('Neither source directory nor build directory contain a build file')
    prefix = pathlib.Path(arguments[arguments.index('--prefix') + 1])
    (prefix / 'lib').mkdir(parents=True, exist_ok=True)
    (prefix / 'lib/libdav1d.a').write_bytes(b'static library fixture')
elif arguments[0] == '--version':
    print('1.9.1')
'''


@unittest.skipUnless(os.name == 'nt', 'Windows PowerShell wrappers')
class WindowsNativeScriptTests(unittest.TestCase):
    """The real wrappers; Python's tools and the MSVC shell are fixtures."""

    def setUp(self):
        generated = NATIVE.parents[2] / '_build/tmp'
        generated.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix='native scripts ', dir=generated)
        self.addCleanup(owner.cleanup)
        self.owner = Path(owner.name)
        # cmd.exe reads a batch file in the console code page: keep the launcher on an ASCII path.
        tools = self.owner / 'bin'
        tools.mkdir()
        (tools / 'py-launcher.py').write_text(PY_LAUNCHER, encoding='utf-8')
        (tools / 'py.cmd').write_text(f'@"{sys.executable}" "{tools / "py-launcher.py"}" %*\n')
        self.checkout = self.owner / 'Jiří 文档'
        self.native = self.checkout / 'native'
        self.native.mkdir(parents=True)
        for name, text in (('prepare-source.py', PREPARE_SOURCE), ('bootstrap-tools.py', BOOTSTRAP_TOOLS),
                           ('private-meson.py', PRIVATE_MESON)):
            (self.native / name).write_text(text, encoding='utf-8')
        self.log = self.owner / 'meson.jsonl'
        self.environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
                                NATIVE_TEST_LOG=str(self.log))

    def run_script(self, name, build_root, shell):
        shutil.copyfile(NATIVE / name, self.native / name)
        report = self.owner / 'report.json'
        driver = self.owner / 'driver.ps1'
        driver.write_text(DRIVER, encoding='utf-8')
        # The code page belongs to the console, not to the process: CREATE_NO_WINDOW gives
        # the script a console of its own and leaves the test runner's console untouched.
        result = subprocess.run(
            [shell, '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(driver), '-Script',
             str(self.native / name), '-BuildRoot', str(build_root), '-Report', str(report)],
            cwd=self.owner, env=self.environment, capture_output=True, encoding='utf-8', errors='replace',
            timeout=60, creationflags=subprocess.CREATE_NO_WINDOW)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        state = json.loads(report.read_text(encoding='utf-8'))
        self.assertEqual(state['code_page'], 850, "The caller's console encoding was not restored")
        return state

    def test_dav1d_build_below_a_non_ascii_path_survives_a_legacy_console_code_page(self):
        build_root = self.checkout / '_build/dav1d'
        state = self.run_script('build-windows.ps1', build_root, PWSH or 'powershell.exe')
        setup = [json.loads(line) for line in self.log.read_text(encoding='utf-8').splitlines()][0]
        self.assertEqual(setup[0], 'setup')
        self.assertEqual(Path(setup[2]), build_root / 'dav1d-1.5.4')
        prefix = build_root / 'prefix-msvc-x64'
        self.assertEqual([Path(line) for line in state['output']], [prefix])
        self.assertEqual(Path(state['library']), prefix / 'lib')
        self.assertTrue((prefix / 'lib/dav1d.lib').is_file())

    @unittest.skipUnless(PWSH, 'The private tool bootstrap requires PowerShell 7')
    def test_tool_bootstrap_below_a_non_ascii_path_survives_a_legacy_console_code_page(self):
        tool_root = self.checkout / '_build/native-tools'
        state = self.run_script('bootstrap-windows.ps1', tool_root, PWSH)
        self.assertEqual(Path(state['tools']), tool_root)
        self.assertEqual(Path(state['temporary']), tool_root / 'tmp')


if __name__ == '__main__':
    unittest.main()
