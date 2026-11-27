"""Normal installations retain every provider and the complete CEF runtime inventory."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import unittest
import zipfile

from test_worker_packages import packaging


ROOT = Path(__file__).resolve().parents[2]
CEF = '156.0.26+g8ea0c73+chromium-156.0.8078.88'
# Access denied and sharing violation: a scanner can briefly hold a file in a directory
# that was just written, which blocks renaming that directory on Windows.
WINDOWS_BUSY_ERRORS = (5, 32)


def rename(source, destination):
    """Rename, retrying for up to 2 seconds while Windows reports the entry as busy."""
    deadline = None
    while True:
        try:
            source.rename(destination)
            return
        except OSError as error:
            if getattr(error, 'winerror', None) not in WINDOWS_BUSY_ERRORS:
                raise
            now = time.monotonic()
            deadline = deadline or now + 2.0
            if now >= deadline:
                raise
            time.sleep(0.05)


def write_file(root, name, content=None):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content if content is not None else ('Fixture ' + name).encode())


def runtime_inventory(package):
    files = []
    for path in sorted(package.rglob('*')):
        if path.is_file() and path != package / 'runtime-files.json':
            data = path.read_bytes()
            files.append({'path': path.relative_to(package).as_posix(), 'bytes': len(data),
                          'sha256': hashlib.sha256(data).hexdigest()})
    (package / 'runtime-files.json').write_text(json.dumps({
        'cef': CEF, 'archive_sha256': 'a' * 64, 'files': files}, sort_keys=True) + '\n')


def html_package(viewers, target):
    windows = target.endswith('windows-msvc')
    package = viewers / 'midnight-viewer-html'
    executable = 'midnight-viewer-html.exe' if windows else 'midnight-viewer-html'
    required = ['LICENSE', 'LICENSE.txt', 'CREDITS.html', 'third-party.md', 'dependency-metadata.json',
                'readme.md', 'resources.pak', 'chrome_100_percent.pak', 'chrome_200_percent.pak',
                'icudtl.dat', 'v8_context_snapshot.bin', 'vk_swiftshader_icd.json',
                'locales/en-US.pak', 'notices/cef/LICENSE-MIT', executable]
    required += (['libcef.dll', 'chrome_elf.dll', 'dxcompiler.dll', 'vk_swiftshader.dll',
                  'vulkan-1.dll', 'midnight-viewer-html.dll'] if windows else
                 ['libcef.so', 'libvk_swiftshader.so', 'libvulkan.so.1', 'chrome-sandbox'])
    for name in required:
        write_file(package, name)
    manifest = {'schema': 1, 'protocol': 1, 'id': 'midnight.viewer.html', 'name': 'HTML',
                'version': '0.2.0', 'target': target, 'executable': executable,
                'extensions': ['html', 'htm', 'xhtml']}
    (package / 'web-viewer.json').write_text(json.dumps(manifest))
    runtime_inventory(package)
    return package


def complete_runtime(source, target):
    windows = target.endswith('windows-msvc')
    suffix = '.exe' if windows else ''
    for name in ('midnight-explorer' + suffix, 'midnight-elevated-listing' + suffix):
        write_file(source, name)
    viewers = source / 'viewers'
    for name in ('basic', 'source', 'markdown'):
        package = viewers / ('midnight-viewer-' + name)
        library = ('midnight_viewer_' + name + '.dll' if windows else
                   'libmidnight_viewer_' + name + '.so')
        write_file(package, library)
        write_file(package, 'plugin.toml', (
            'schema_version=1\nkind="viewer"\nversion="0.1.0"\nabi_major=1\n'
            'minimum_host_minor=0\nmodels=["text-page.v1"]\nrequired_host_services=[]\n'
            f'id="midnight.viewer.{name}"\ntarget="{target}"\nentry_library="{library}"\n'
        ).encode())
        for notice in ('LICENSE', 'third-party.md', 'notices/LICENSE-MIT'):
            write_file(package, notice)
    for name in ('image', 'pdf'):
        package = viewers / ('midnight-viewer-' + name)
        executable = 'midnight-viewer-' + name + suffix
        write_file(package, executable)
        write_file(package, 'image-viewer.json', json.dumps({
            'schema': 1, 'protocol': 2 if name == 'pdf' else 1,
            'id': 'midnight.viewer.' + name, 'name': name, 'version': '0.1.0',
            'target': target, 'executable': executable, 'extensions': ['pdf' if name == 'pdf' else 'png'],
        }).encode())
        for notice in ('LICENSE', 'third-party.md', 'notices/LICENSE-MIT'):
            write_file(package, notice)
    html_package(viewers, target)
    for kind in ('native', 'network'):
        package = source / 'filesystems' / kind
        network = kind == 'network'
        library = kind + ('.dll' if windows else '.so')
        capabilities = (['browse', 'stat', 'volumes', 'activate', 'refresh', 'resolve-text']
                        if network else ['native-address'])
        encoding = 'opaque' if network else 'windows-utf16le' if windows else 'unix-bytes'
        write_file(package, library)
        write_file(package, 'third-party.md')
        write_file(package, 'plugin.toml', (
            f'schema_version=1\nid="{kind}"\nkind="filesystem-{kind}"\nversion="0.1.0"\n'
            f'entry_library="{library}"\ntarget="{target}"\nprotocol=1\nabi_major=1\n'
            f'minimum_host_minor={2 if network else 0}\naddress_encoding="{encoding}"\n'
            f'capabilities={json.dumps(capabilities)}\n'
        ).encode())
    archive = source / 'filesystems' / 'archive'
    write_file(archive, 'archive-worker' + suffix)
    write_file(archive, 'third-party.md')
    write_file(archive, 'licenses/LICENSE-MIT')
    write_file(archive, 'plugin.toml', (
        'schema_version=1\nid="midnight.fs.archive"\nkind="filesystem-worker"\nversion="0.1.0"\n'
        f'entry_executable="archive-worker{suffix}"\ntarget="{target}"\nprotocol=3\nformats=["zip"]\n'
    ).encode())
    thumbs = source / 'thumbnailers'
    for name in ('image', 'video'):
        executable = 'midnight-thumbnail-' + name + suffix
        write_file(thumbs, executable)
        write_file(thumbs, 'midnight-thumbnail-' + name + '.json', json.dumps({
            'protocol': 1, 'id': 'midnight-' + name, 'version': '0.1.0',
            'executable': executable, 'extensions': ['avi' if name == 'video' else 'png'],
            'video': name == 'video',
        }).encode())
        write_file(thumbs, 'notices/midnight-thumbnail-' + name + '/inventory.json')
    libraries = (['avcodec-63.dll', 'avformat-63.dll', 'avutil-61.dll', 'swscale-10.dll']
                 if windows else ['libavcodec.so.63', 'libavformat.so.63', 'libavutil.so.61',
                                  'libswscale.so.10'])
    for name in libraries:
        write_file(thumbs, name)
    for name in ('ffmpeg-9.0.2.tar.xz', 'COPYING.LGPLv2.1', 'build-options.txt', 'config.h', 'config.mak'):
        write_file(thumbs, 'notices/ffmpeg-9.0.2/' + name)


class RuntimePackages(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / '_build/tmp'
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name)
        self.target = 'x86_64-unknown-linux-gnu'

    def validator(self):
        function = getattr(packaging, 'validate_runtime_installation', None)
        self.assertIsNotNone(function, 'Normal deployment lacks complete runtime validation')
        return function

    def test_normal_viewer_set_includes_the_html_worker(self):
        self.assertIn('midnight.viewer.html', packaging.REQUIRED_VIEWERS)

    def test_complete_linux_and_windows_runtime_graphs_validate_without_copying(self):
        validate = self.validator()
        for target in (self.target, 'x86_64-pc-windows-msvc'):
            source = self.root / target
            complete_runtime(source, target)
            before = sorted(str(path.relative_to(source)) for path in source.rglob('*'))
            validate(source, target)
            self.assertEqual(sorted(str(path.relative_to(source)) for path in source.rglob('*')), before)

    def test_each_required_module_is_rejected_before_normal_deployment(self):
        validate = self.validator()
        for target in (self.target, 'x86_64-pc-windows-msvc'):
            windows = target.endswith('windows-msvc')
            suffix = '.exe' if windows else ''
            source = self.root / ('runtime-' + target)
            complete_runtime(source, target)
            libraries = (['avcodec-63.dll', 'avformat-63.dll', 'avutil-61.dll', 'swscale-10.dll'] if windows else
                         ['libavcodec.so.63', 'libavformat.so.63', 'libavutil.so.61', 'libswscale.so.10'])
            for relative in [
                *(f'viewers/midnight-viewer-{name}' for name in ('basic', 'source', 'markdown', 'image', 'pdf', 'html')),
                *(f'filesystems/{name}' for name in ('native', 'network', 'archive')),
                *(f'thumbnailers/midnight-thumbnail-{name}{extension}' for name in ('image', 'video')
                  for extension in ('.json', suffix)),
                *(f'thumbnailers/notices/midnight-thumbnail-{name}' for name in ('image', 'video')),
                *(f'thumbnailers/{name}' for name in libraries),
                *(f'thumbnailers/notices/ffmpeg-9.0.2/{name}' for name in (
                    'ffmpeg-9.0.2.tar.xz', 'COPYING.LGPLv2.1', 'build-options.txt', 'config.h', 'config.mak')),
                'thumbnailers', 'midnight-explorer' + suffix, 'midnight-elevated-listing' + suffix,
            ]:
                with self.subTest(target=target, missing=relative):
                    original = source / relative
                    backup = self.root / 'removed'
                    rename(original, backup)
                    try:
                        with self.assertRaises((OSError, ValueError)):
                            validate(source, target)
                    finally:
                        rename(backup, original)

    def test_runtime_validation_cli_rejects_partial_modules_without_creating_outputs(self):
        source = self.root / 'runtime'
        complete_runtime(source, self.target)
        command = [sys.executable, '-B', str(ROOT / 'scripts/package-release.py'),
                   'validate-runtime', '--source', str(source), '--target', self.target]
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('midnight.viewer.html', json.loads(result.stdout)['viewers'])
        shutil.rmtree(source / 'filesystems/archive')
        before = sorted(str(path.relative_to(source)) for path in source.rglob('*'))
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('midnight.fs.archive', result.stderr)
        self.assertEqual(result.stdout, '')
        self.assertEqual(sorted(str(path.relative_to(source)) for path in source.rglob('*')), before)

    def test_html_inventory_notices_and_executable_modes_survive_archiving(self):
        viewers = self.root / 'viewers'
        package = html_package(viewers, self.target)
        stage = self.root / 'release'
        stage.mkdir()
        expected = {path.relative_to(package).as_posix(): path.read_bytes()
                    for path in package.rglob('*') if path.is_file()}
        try:
            inventory = packaging.copy_viewer_packages(stage, viewers, self.target)
        except FileNotFoundError as error:
            self.fail('HTML process packages are not supported by the release parser: ' + str(error))
        self.assertEqual(inventory[0]['kind'], 'web-viewer-worker')
        executable = [file['path'] for file in inventory[0]['files'] if file['executable']]
        self.assertEqual(set(executable), {'viewers/midnight-viewer-html/midnight-viewer-html',
                                           'viewers/midnight-viewer-html/chrome-sandbox'})
        for kind in ('zip', 'tar.gz'):
            output = self.root / ('release.' + kind)
            packaging.write_archive(stage, output, 'midnight-explorer', kind, executable)
            with zipfile.ZipFile(output) if kind == 'zip' else tarfile.open(output) as bundle:
                for name, data in expected.items():
                    member = 'release/viewers/midnight-viewer-html/' + name
                    copied = bundle.read(member) if kind == 'zip' else bundle.extractfile(member).read()
                    self.assertEqual(copied, data)
                sandbox = 'release/viewers/midnight-viewer-html/chrome-sandbox'
                mode = (bundle.getinfo(sandbox).external_attr >> 16 & 0o777 if kind == 'zip'
                        else bundle.getmember(sandbox).mode)
                self.assertEqual(mode, 0o755)

    def test_changed_missing_unlisted_or_duplicate_html_inventory_files_are_rejected(self):
        validate = self.validator()
        source = self.root / 'runtime'
        complete_runtime(source, self.target)
        html = source / 'viewers/midnight-viewer-html'
        for relative in ('LICENSE.txt', 'CREDITS.html', 'libcef.so', 'chrome-sandbox',
                         'locales/en-US.pak', 'notices/cef/LICENSE-MIT'):
            with self.subTest(changed=relative):
                path = html / relative
                original = path.read_bytes()
                path.write_bytes(b'Changed after build')
                with self.assertRaises(ValueError):
                    validate(source, self.target)
                path.write_bytes(original)
        inventory = html / 'runtime-files.json'
        original = inventory.read_bytes()
        document = json.loads(original)
        document['files'].append(document['files'][0])
        inventory.write_text(json.dumps(document))
        with self.assertRaises(ValueError):
            validate(source, self.target)
        inventory.write_bytes(original)
        write_file(html, 'unlisted.dll')
        with self.assertRaises(ValueError):
            validate(source, self.target)

    def test_windows_html_bootstrap_client_helpers_and_inventory_are_preserved(self):
        target = 'x86_64-pc-windows-msvc'
        viewers = self.root / 'viewers'
        package = html_package(viewers, target)
        write_file(package, 'chrome_crashpad_handler.exe')
        runtime_inventory(package)
        original = (package / 'runtime-files.json').read_bytes()
        stage = self.root / 'release'
        stage.mkdir()
        inventory = packaging.copy_viewer_packages(stage, viewers, target)
        copied = {entry['path'].split('/')[-1]: entry for entry in inventory[0]['files']}
        self.assertTrue(copied['midnight-viewer-html.exe']['executable'])
        self.assertTrue(copied['chrome_crashpad_handler.exe']['executable'])
        self.assertFalse(copied['midnight-viewer-html.dll']['executable'])
        output = stage / 'viewers/midnight-viewer-html'
        self.assertEqual((output / 'runtime-files.json').read_bytes(), original)
        for name in ('midnight-viewer-html.exe', 'midnight-viewer-html.dll', 'libcef.dll',
                     'chrome_crashpad_handler.exe', 'LICENSE.txt', 'CREDITS.html'):
            self.assertEqual((output / name).read_bytes(), (package / name).read_bytes())

    def test_html_required_notices_sandbox_and_locales_cannot_be_removed_from_inventory(self):
        validate = self.validator()
        source = self.root / 'runtime'
        complete_runtime(source, self.target)
        html = source / 'viewers/midnight-viewer-html'
        for relative in ('LICENSE', 'LICENSE.txt', 'CREDITS.html', 'chrome-sandbox',
                         'locales/en-US.pak', 'notices/cef/LICENSE-MIT'):
            with self.subTest(missing=relative):
                path = html / relative
                data = path.read_bytes()
                path.unlink()
                runtime_inventory(html)
                try:
                    with self.assertRaises(ValueError):
                        validate(source, self.target)
                finally:
                    path.write_bytes(data)
                    runtime_inventory(html)

    def test_html_package_carries_its_own_licence_beside_the_cef_licence(self):
        # The provider is GPL-3.0-or-later (its own LICENSE); CEF's LICENSE.txt covers only CEF.
        viewers = self.root / 'viewers'
        package = html_package(viewers, self.target)
        stage = self.root / 'release'
        stage.mkdir()
        inventory = packaging.copy_viewer_packages(stage, viewers, self.target)
        copied = {entry['path'] for entry in inventory[0]['files']}
        self.assertIn('viewers/midnight-viewer-html/LICENSE', copied)
        self.assertIn('viewers/midnight-viewer-html/LICENSE.txt', copied)
        (package / 'LICENSE').unlink()
        runtime_inventory(package)
        refused = self.root / 'refused'
        refused.mkdir()
        with self.assertRaisesRegex(ValueError, 'missing required file LICENSE:'):
            packaging.copy_viewer_packages(refused, viewers, self.target)
        self.assertEqual(list(refused.iterdir()), [])

    def test_html_inventory_rejects_traversal_boolean_sizes_and_duplicate_fields(self):
        validate = self.validator()
        source = self.root / 'runtime'
        complete_runtime(source, self.target)
        inventory = source / 'viewers/midnight-viewer-html/runtime-files.json'
        original = inventory.read_text()
        for key, value in [('path', '../outside'), ('path', 'locales\\escape.pak'),
                           ('bytes', True), ('bytes', -1), ('sha256', 'not-a-hash')]:
            with self.subTest(key=key, value=value):
                changed = json.loads(original)
                changed['files'][0][key] = value
                inventory.write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    validate(source, self.target)
        inventory.write_text(original.replace('"cef":', '"cef":"duplicate", "cef":', 1))
        with self.assertRaises(ValueError):
            validate(source, self.target)

    def release(self, source, readme_version='0.2.0', binary='midnight-explorer'):
        readme = self.root / 'README.txt'
        readme.write_text(f'Midnight Explorer {readme_version} - fixture\n'
                          f'https://github.com/TomasJelinek/fm/tree/v{readme_version}\n')
        output = self.root / 'release-output'
        command = [sys.executable, '-B', str(ROOT / 'scripts/package-release.py'),
                   '--binary', str(source / binary), '--metadata', str(self.root / 'absent.json'),
                   '--dependency-list', str(self.root / 'absent.txt'), '--rust-docs', str(self.root),
                   '--target', self.target, '--format', 'tar.gz', '--readme', str(readme),
                   '--revision', '0' * 40, '--output', str(output)]
        return subprocess.run(command, capture_output=True, text=True), output

    def test_refused_release_inputs_leave_no_partial_stage(self):
        source = self.root / 'runtime'
        complete_runtime(source, self.target)
        shutil.rmtree(source / 'viewers/midnight-viewer-html')
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
        for readme_version in (version, '0.0.0-stale'):
            with self.subTest(readme=readme_version):
                result, output = self.release(source, readme_version)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists(), 'A refused release left a staging directory')

    def test_the_application_executable_keeps_its_released_name(self):
        # Releases up to 0.5.0 named the application `me`; an installation or a release
        # built from such an executable is refused rather than packaged under that name.
        validate = self.validator()
        for target, previous in ((self.target, 'me'), ('x86_64-pc-windows-msvc', 'me.exe')):
            with self.subTest(target=target):
                source = self.root / ('renamed-' + target)
                complete_runtime(source, target)
                current = 'midnight-explorer' + ('.exe' if target.endswith('windows-msvc') else '')
                self.assertEqual(packaging.application_executable(source / current, target), current)
                with self.assertRaisesRegex(ValueError, current):
                    packaging.application_executable(source / previous, target)
                rename(source / current, source / previous)
                with self.assertRaisesRegex((OSError, ValueError), current):
                    validate(source, target)
        with self.assertRaises(ValueError):
            packaging.application_executable(Path('midnight-explorer.exe'), self.target)
        source = self.root / 'runtime'
        complete_runtime(source, self.target)
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
        rename(source / 'midnight-explorer', source / 'me')
        result, output = self.release(source, version, binary='me')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('must be named midnight-explorer', result.stderr)
        self.assertFalse(output.exists(), 'A refused release left a staging directory')


if __name__ == '__main__':
    unittest.main()
