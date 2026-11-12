"""Private tool bootstrap validates bytes and paths before any executable is used."""
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import sys
import subprocess
import tarfile
import tempfile
import unittest
import zipfile

SCRIPT = Path(__file__).resolve().parents[1] / 'bootstrap-tools.py'

class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), 'Pinned private tool bootstrap must exist')
        spec = importlib.util.spec_from_file_location('native_bootstrap', SCRIPT)
        self.module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = self.module
        spec.loader.exec_module(self.module)
        generated = SCRIPT.parents[3] / '_build/tmp'
        generated.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=generated)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def artifact(self, data, kind='zip'):
        return self.module.Artifact('fixture', 'fixture.' + kind,
            'https://example.invalid/fixed', hashlib.sha256(data).hexdigest(), kind, 'fixture')

    def zip_bytes(self, entries):
        out = io.BytesIO()
        with zipfile.ZipFile(out, 'w') as archive:
            for name, value in entries:
                if isinstance(name, str):
                    item = zipfile.ZipInfo(name)
                    item.filename = name  # Keep hostile separators even on Windows.
                else:
                    item = name
                archive.writestr(item, value)
        return out.getvalue()

    def test_explicit_ordinary_build_root_is_required(self):
        for value in [Path('relative/_build/tools'), self.root.parent.parent.parent / 'outside', self.root / '..' / self.root.name]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.module.ordinary_root(value)
        self.assertEqual(self.module.ordinary_root(self.root), self.root)

    def test_offline_missing_and_corrupt_archive_never_fetch(self):
        data = self.zip_bytes([('file', b'valid')])
        artifact = self.artifact(data)
        def forbidden(*args, **kwargs):
            self.fail('Offline mode attempted a network request')
        with self.assertRaisesRegex(ValueError, 'Offline'):
            self.module.fetch(self.root, artifact, True, forbidden)
        downloads = self.root / 'downloads'
        downloads.mkdir(exist_ok=True)
        (downloads / artifact.filename).write_bytes(b'wrong')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            self.module.fetch(self.root, artifact, True, forbidden)

    def test_bad_or_oversized_download_is_not_published(self):
        artifact = self.artifact(b'expected')
        for data in [b'wrong', b'x' * (self.module.MAX_DOWNLOAD_BYTES + 1)]:
            with self.subTest(size=len(data)), self.assertRaises(ValueError):
                self.module.fetch(self.root, artifact, False, lambda *a, **k: io.BytesIO(data))
            self.assertEqual(list((self.root / 'downloads').iterdir()), [])

    def test_zip_paths_links_collisions_and_expansion_are_refused_before_output(self):
        link = zipfile.ZipInfo('link')
        link.create_system = 3
        link.external_attr = 0o120777 << 16
        for entries in [[('../escape', b'x')], [('/absolute', b'x')], [('a\\b', b'x')], [('a:b', b'x')], [('A', b'x'), ('a', b'y')], [(link, b'outside')]]:
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                self.module.unpack(self.root, self.artifact(self.zip_bytes(entries)), self.zip_bytes(entries))
        self.assertFalse((self.root / 'tools/fixture').exists())
        data = self.zip_bytes([('large', b'a' * 100)])
        self.module.MAX_EXTRACT_BYTES = 32
        with self.assertRaisesRegex(ValueError, 'expansion'):
            self.module.unpack(self.root, self.artifact(data), data)

    def test_cached_tree_is_checked_against_archive_and_does_not_accept_extra_code(self):
        data = self.zip_bytes([('package/tool.py', b'print(1)\n'), ('LICENSE', b'notice')])
        artifact = self.artifact(data)
        destination = self.module.unpack(self.root, artifact, data)
        self.assertEqual(self.module.unpack(self.root, artifact, data), destination)
        (destination / 'package/tool.py').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'cached|Cached'):
            self.module.unpack(self.root, artifact, data)
        (destination / 'package/tool.py').write_bytes(b'print(1)\n')
        (destination / 'sitecustomize.py').write_bytes(b'print(2)')
        with self.assertRaisesRegex(ValueError, 'cached|Cached'):
            self.module.unpack(self.root, artifact, data)

    def test_tar_source_preserves_contents_but_refuses_links(self):
        for is_link in [False, True]:
            out = io.BytesIO()
            with tarfile.open(fileobj=out, mode='w:xz') as archive:
                member = tarfile.TarInfo('nasm/configure')
                if is_link:
                    member.type = tarfile.SYMTYPE
                    member.linkname = '/outside'
                    archive.addfile(member)
                else:
                    member.size = 4
                    member.mode = 0o755
                    archive.addfile(member, io.BytesIO(b'test'))
            data = out.getvalue()
            artifact = self.artifact(data, 'tar.xz')
            if is_link:
                with self.assertRaisesRegex(ValueError, 'link|type'):
                    self.module.unpack(self.root, artifact, data)
            else:
                destination = self.module.unpack(self.root, artifact, data)
                self.assertEqual((destination / 'nasm/configure').read_bytes(), b'test')

    def test_tar_build_dependency_timestamps_survive_extract_and_verified_reuse(self):
        out = io.BytesIO()
        stamps = {'configure': 1713373509, 'configure.ac': 1713373448}
        with tarfile.open(fileobj=out, mode='w:xz') as archive:
            for name, stamp in stamps.items():
                member = tarfile.TarInfo('nasm/' + name)
                member.size = 4
                member.mtime = stamp
                archive.addfile(member, io.BytesIO(b'test'))
        data = out.getvalue()
        artifact = self.artifact(data, 'tar.xz')
        destination = self.module.unpack(self.root, artifact, data)
        for name, stamp in stamps.items():
            self.assertEqual(int((destination / 'nasm' / name).stat().st_mtime), stamp)
        os.utime(destination / 'nasm/configure.ac', (1794483234, 1794483234))
        self.module.unpack(self.root, artifact, data)
        self.assertEqual(int((destination / 'nasm/configure.ac').stat().st_mtime), stamps['configure.ac'])

    def test_tar_expansion_and_entry_limits_refuse_before_output(self):
        out = io.BytesIO()
        with tarfile.open(fileobj=out, mode='w:xz') as archive:
            for name in ['first', 'second']:
                member = tarfile.TarInfo(name)
                member.size = 64
                archive.addfile(member, io.BytesIO(b'x' * 64))
        data = out.getvalue()
        artifact = self.artifact(data, 'tar.xz')
        self.module.MAX_ENTRIES = 1
        with self.assertRaisesRegex(ValueError, 'expansion'):
            self.module.unpack(self.root, artifact, data)
        self.module.MAX_ENTRIES = 8192
        self.module.MAX_EXTRACT_BYTES = 32
        with self.assertRaisesRegex(ValueError, 'expansion'):
            self.module.unpack(self.root, artifact, data)
        self.assertFalse((self.root / 'tools/fixture').exists())

    def test_private_meson_ignores_shadow_modules_and_site_startup(self):
        tool_root = os.environ.get('MIDNIGHT_NATIVE_TEST_TOOL_ROOT')
        if not tool_root:
            self.skipTest('Set MIDNIGHT_NATIVE_TEST_TOOL_ROOT to an already verified offline tool cache')
        launcher = SCRIPT.with_name('private-meson.py')
        self.assertTrue(launcher.is_file(), 'Isolated Meson launcher must exist')
        sentinel = self.root / 'ambient-executed'
        hostile = self.root / 'hostile'
        hostile.mkdir()
        package = hostile / 'mesonbuild'
        package.mkdir()
        payload = 'from pathlib import Path\nPath(' + repr(str(sentinel)) + ').write_text("wrong")\n'
        (hostile / 'sitecustomize.py').write_text(payload, encoding='utf-8')
        (package / '__init__.py').write_text(payload, encoding='utf-8')
        (package / 'mesonmain.py').write_text('print("shadow")\n', encoding='utf-8')
        environment = dict(os.environ, PYTHONPATH=str(hostile), PYTHONUSERBASE=str(hostile),
            MIDNIGHT_IMAGE_NATIVE_TOOLS=tool_root)
        result = subprocess.run([sys.executable, '-I', '-S', '-B', str(launcher), '--version'],
            cwd=hostile, env=environment, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.strip(), '1.9.1')
        self.assertFalse(sentinel.exists(), 'Ambient Python code executed before pinned Meson')

    @unittest.skipIf(os.name == 'nt', 'Exercises sourced Bash failure semantics on Linux')
    def test_native_helper_preserves_failures_when_sourced_in_a_conditional(self):
        helper = SCRIPT.with_name('build-linux.sh')
        command = r'''
native_test_root=$2
native_test_stage=$3
python3() {
    if [[ "$4" == */prepare-source.py ]]; then
        [[ "$native_test_stage" != prepare ]] || return 47
        printf '%s\n' "$native_test_root/source"
    else
        [[ "$5" != "$native_test_stage" ]] || return 47
    fi
}
if source "$1" "$native_test_root" 2 --offline >/dev/null 2>&1; then
    exit 90
else
    native_test_status=$?
fi
[[ "$native_test_status" == 47 && -z "${SYSTEM_DEPS_DAV1D_LINK+x}" ]]
'''
        environment = dict(os.environ)
        environment.pop('SYSTEM_DEPS_DAV1D_LINK', None)
        for stage in ['prepare', 'setup', 'compile', 'install']:
            with self.subTest(stage=stage):
                result = subprocess.run(['bash', '-c', command, 'bootstrap-test',
                    str(helper), str(self.root), stage], cwd=self.root,
                    env=environment, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_symlinked_root_and_cached_tool_are_rejected(self):
        link = self.root / 'alias'
        real = self.root / 'real'
        real.mkdir()
        try:
            link.symlink_to(real, target_is_directory=True)
        except OSError as error:
            self.skipTest(str(error))
        with self.assertRaises(ValueError):
            self.module.ordinary_root(link)
        data = self.zip_bytes([('tool', b'original')])
        artifact = self.artifact(data)
        destination = self.module.unpack(self.root, artifact, data)
        (destination / 'tool').unlink()
        (real / 'tool').write_bytes(b'original')
        (destination / 'tool').symlink_to(real / 'tool')
        with self.assertRaises(ValueError):
            self.module.unpack(self.root, artifact, data)

if __name__ == '__main__':
    unittest.main()
