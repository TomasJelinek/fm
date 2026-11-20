"""The native helpers follow the build scripts' link rule and wait out brief holds.

Generated output lies strictly below the checkout's own _build: in the repository that
holds this package in plugins/, the repository's or a plugin's (plugins/NAME/_build),
recognised however a path spells the checkout; outside it, below the first _build of a
custom directory. From that _build down nothing may be a link, junction or other reparse
point, nor anywhere in a cached tool tree or the extracted dav1d source; the checkout
and the folders above it may be links or hidden folders such as AppData. Fresh downloads
and stages are renamed with a Windows-only retry of about 2 s, then the first error.
"""
import ast
import hashlib
import importlib.util
import inspect
import io
import os
from pathlib import Path
import shutil
import stat
import sys
import tarfile
import tempfile
import threading
import time
import unittest
from unittest import mock
import zipfile

NATIVE = Path(__file__).resolve().parents[1]
CHECKOUT = NATIVE.parents[2]
HELPERS = ('bootstrap-tools.py', 'prepare-source.py')


def directory_link(link, target):
    """A link to a directory: a junction on Windows (no privilege needed), else a symlink."""
    link, target = Path(link), Path(target)
    if os.name == 'nt':
        import _winapi
        _winapi.CreateJunction(str(target), str(link))
    else:
        link.symlink_to(target, target_is_directory=True)
    return link


def remove_link(path):
    """Remove a directory link itself: a junction with rmdir, a symlink with unlink."""
    if os.name == 'nt':
        os.rmdir(path)
    else:
        os.unlink(path)


def hidden_directory(path):
    """A new directory hidden like a profile's AppData (an ordinary one elsewhere)."""
    path.mkdir(parents=True)
    if os.name == 'nt':
        import ctypes
        if not ctypes.windll.kernel32.SetFileAttributesW(str(path), stat.FILE_ATTRIBUTE_HIDDEN):
            raise OSError(f'Cannot hide fixture directory: {path}')
    return path


class Hold:
    """`path` open without delete sharing, as endpoint protection keeps a file just
    written: Windows refuses to rename or replace it until a timer closes the handle."""

    def __init__(self, path, seconds):
        import _winapi
        # GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, OPEN_EXISTING
        self._handle = _winapi.CreateFile(str(path), 0x80000000, 3, 0, 3, 0, 0)
        self._lock = threading.Lock()
        self._timer = threading.Timer(seconds, self.release)
        self._timer.start()

    def release(self):
        import _winapi
        self._timer.cancel()
        with self._lock:
            if self._handle is not None:
                _winapi.CloseHandle(self._handle)
                self._handle = None


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def outside_build_temporary(test):
    """A temporary directory with no _build component (the tests point TEMP into _build)."""
    for candidate in (os.environ.get('RUNNER_TEMP'), '/tmp',
                      os.path.join(os.environ.get('LOCALAPPDATA', ''), 'Temp')):
        if candidate and os.path.isdir(candidate) and '_build' not in Path(candidate).resolve().parts:
            directory = tempfile.TemporaryDirectory(prefix='native custom ', dir=candidate)
            test.addCleanup(directory.cleanup)
            return Path(directory.name)
    raise unittest.SkipTest('No temporary directory outside _build')


class Fixture(unittest.TestCase):
    """Copies of the helpers in a repository below a hidden folder, reached through a link."""

    def setUp(self):
        generated = CHECKOUT / '_build/tmp'
        generated.mkdir(parents=True, exist_ok=True)
        owner = tempfile.TemporaryDirectory(prefix='native links ', dir=generated)
        self.addCleanup(owner.cleanup)
        self.owner = Path(owner.name)
        self.real = hidden_directory(self.owner / 'AppData') / 'repository'
        native = self.real / 'plugins/image-decoder/native'
        native.mkdir(parents=True)
        for name in HELPERS:
            shutil.copyfile(NATIVE / name, native / name)
        (self.real / 'plugins/midnight-viewer-image').mkdir()
        (self.real / '_build').mkdir()
        self.outside = self.owner / 'outside'
        self.outside.mkdir()
        directory_link(self.real / '_build/redirect', self.outside)
        directory_link(self.real / '_build/loop', self.real)
        self.linked = directory_link(self.owner / 'linked', self.real.parent) / 'repository'
        self.bootstrap = load(native / 'bootstrap-tools.py', 'fixture_native_bootstrap')
        self.source = load(native / 'prepare-source.py', 'fixture_dav1d_source')

    def zip_artifact(self, entries):
        out = io.BytesIO()
        with zipfile.ZipFile(out, 'w') as archive:
            for name, payload in entries.items():
                archive.writestr(name, payload)
        data = out.getvalue()
        return self.bootstrap.Artifact('fixture', 'fixture.zip', 'https://example.invalid/fixture',
                                       hashlib.sha256(data).hexdigest(), 'zip', 'fixture'), data

    def dav1d_release(self, contents=b'original\n'):
        """A stand-in dav1d release whose meson.build gets one build fix."""
        out = io.BytesIO()
        with tarfile.open(fileobj=out, mode='w:xz') as archive:
            member = tarfile.TarInfo(f'dav1d-{self.source.VERSION}/meson.build')
            member.size = len(contents)
            archive.addfile(member, io.BytesIO(contents))
        data = out.getvalue()
        fix = ('meson.build', hashlib.sha256(b'original\n').hexdigest(),
               hashlib.sha256(b'patched\n').hexdigest(), b'original\n', b'patched\n')
        self.enterContext(mock.patch.object(self.source, 'SHA256', hashlib.sha256(data).hexdigest()))
        self.enterContext(mock.patch.object(self.source, 'BUILD_FIXES', (fix,)))
        return data


class BuildFolders(Fixture):
    def test_linked_hidden_checkout_and_plugin_or_custom_build_folders_are_accepted(self):
        for value in (self.linked / '_build/native-tools',
                      self.real / '_build/native-tools',
                      # A plugin's own _build, as a viewer built on its own passes it.
                      self.linked / 'plugins/midnight-viewer-image/_build/native-tools',
                      self.linked / 'plugins/image-decoder/_build/native-tools',
                      # Outside the checkout: the path's first _build, here the test's own.
                      self.owner / 'custom/_build/native-tools'):
            with self.subTest(value=value):
                self.assertEqual(self.bootstrap.ordinary_root(value), value)
                self.assertTrue(value.is_dir())
                self.assertEqual(self.source.build_directory(value / 'dav1d'), value / 'dav1d')

    def test_custom_target_reached_through_a_link_is_accepted(self):
        outside = outside_build_temporary(self)
        (outside / 'target').mkdir()
        spelled = directory_link(outside / 'linked target', outside / 'target')
        self.assertEqual(self.bootstrap.ordinary_root(spelled / '_build/native-tools'),
                         spelled / '_build/native-tools')
        self.assertTrue((outside / 'target/_build/native-tools').is_dir())
        directory_link(outside / 'target/_build/redirect', self.outside)
        with self.assertRaisesRegex(ValueError, 'must not traverse a link'):
            self.bootstrap.ordinary_root(spelled / '_build/redirect/native-tools')

    def test_links_from_build_down_and_other_checkout_folders_are_refused(self):
        (self.real / 'plugins/midnight-viewer-pdf').mkdir()
        directory_link(self.real / 'plugins/midnight-viewer-pdf/_build', self.outside)
        for value, message in (
                (self.linked / '_build/redirect/native-tools', 'must not traverse a link'),
                (self.real / '_build/loop/_build/native-tools', 'must not traverse a link'),
                (self.linked / 'plugins/midnight-viewer-pdf/_build/native-tools', 'must not traverse a link'),
                (self.linked / 'native-tools', 'must be under an explicit _build'),
                (self.linked / 'other/_build/native-tools', 'must be under an explicit _build'),
                (self.linked / 'plugins/midnight-viewer-image/target/native-tools', 'must be under an explicit _build'),
                (self.linked / 'plugins/_build/native-tools', 'must be under an explicit _build'),
                (self.linked / '_build', 'must be under an explicit _build'),
                (self.linked / '_build/../_build/native-tools', 'parent traversal')):
            for check in (self.bootstrap.ordinary_root, self.source.build_directory):
                with self.subTest(value=value, check=check.__name__), self.assertRaisesRegex(ValueError, message):
                    check(value)
        with self.assertRaisesRegex(ValueError, 'absolute'):
            self.bootstrap.ordinary_root(Path('relative/_build/native-tools'))
        self.assertEqual(list(self.outside.iterdir()), [])
        for created in ('native-tools', 'other', 'plugins/midnight-viewer-image/target', 'plugins/_build'):
            self.assertFalse((self.real / created).exists(), created)


class CachedTrees(Fixture):
    """Verified downloads and tool trees are used by path: nothing in them may be a link."""

    def test_linked_tool_cache_folders_and_files_are_refused(self):
        artifact, data = self.zip_artifact({'tool.py': b'print(1)\n'})
        root = self.bootstrap.ordinary_root(self.linked / '_build/native-tools')
        directory_link(root / 'downloads', self.outside)
        def forbidden(*args, **kwargs):
            self.fail('A linked cache attempted a download')
        with self.assertRaisesRegex(ValueError, 'Linked generated path'):
            self.bootstrap.fetch(root, artifact, False, forbidden)
        remove_link(root / 'downloads')
        (root / 'downloads').mkdir()
        # The verified archive itself, reached through a link: a file symlink where the
        # platform allows one without privileges, else a junction.
        elsewhere = self.owner / 'elsewhere'
        elsewhere.mkdir()
        (elsewhere / artifact.filename).write_bytes(data)
        try:
            (root / 'downloads' / artifact.filename).symlink_to(elsewhere / artifact.filename)
        except OSError:
            directory_link(root / 'downloads' / artifact.filename, elsewhere)
        with self.assertRaisesRegex(ValueError, 'Invalid cached archive file'):
            self.bootstrap.fetch(root, artifact, False, forbidden)
        (root / 'tools').mkdir()
        directory_link(root / 'tools' / artifact.name, self.outside)
        with self.assertRaisesRegex(ValueError, 'Linked generated path'):
            self.bootstrap.unpack(root, artifact, data)
        self.assertEqual(list(self.outside.iterdir()), [])

    def test_a_link_inside_a_cached_tool_tree_is_refused(self):
        artifact, data = self.zip_artifact({'package/tool.py': b'print(1)\n'})
        root = self.bootstrap.ordinary_root(self.linked / '_build/native-tools')
        destination = self.bootstrap.unpack(root, artifact, data)
        directory_link(destination / 'package/extra', self.outside)
        with self.assertRaisesRegex(ValueError, 'Linked cached tool path'):
            self.bootstrap.unpack(root, artifact, data)

    def test_links_below_the_dav1d_root_or_in_its_source_are_refused_before_use(self):
        for place in ('msvc-x64', f'dav1d-{self.source.VERSION}.tar.xz', f'dav1d-{self.source.VERSION}/include'):
            with self.subTest(place=place):
                root = self.source.build_directory(self.linked / '_build' / place.replace('/', '-').replace('.', '-'))
                (root / place).parent.mkdir(parents=True, exist_ok=True)
                directory_link(root / place, self.outside)
                with self.assertRaisesRegex(ValueError, 'must not contain a link'):
                    self.source.prepare(root, offline=True)
        self.assertEqual(list(self.outside.iterdir()), [])

    def test_the_archive_is_verified_and_extracted_from_the_same_bytes(self):
        data = self.dav1d_release()
        root = self.source.build_directory(self.linked / '_build/dav1d')
        archive = root / f'dav1d-{self.source.VERSION}.tar.xz'
        archive.write_bytes(data)
        verify = self.source.archive_bytes
        def verified_then_swapped(path):
            checked = verify(path)
            # Someone replaces the cached archive after its checksum was compared.
            out = io.BytesIO()
            with tarfile.open(fileobj=out, mode='w:xz') as swapped:
                member = tarfile.TarInfo(f'dav1d-{self.source.VERSION}/meson.build')
                member.size = len(b'swapped\n')
                swapped.addfile(member, io.BytesIO(b'swapped\n'))
            path.write_bytes(out.getvalue())
            return checked
        with mock.patch.object(self.source, 'archive_bytes', verified_then_swapped):
            source = self.source.prepare(root, offline=True)
        self.assertEqual((source / 'meson.build').read_bytes(), b'patched\n')
        self.assertTrue((root / 'source.json').is_file())
        # The swapped archive is refused at its next use; the verified one reuses the tree,
        # also spelled without the link.
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            self.source.prepare(root, offline=True)
        archive.write_bytes(data)
        reused = self.source.prepare(self.real / '_build/dav1d', offline=True)
        self.assertEqual(reused, self.real / '_build/dav1d' / source.name)
        self.assertEqual((reused / 'meson.build').read_bytes(), b'patched\n')


class FreshRenames(Fixture):
    """Fresh downloads and stages wait out a brief Windows hold, and nothing else."""

    def scripted(self, failures, holds=(5, 32, 33)):
        """replace_fresh while the first renames raise `failures`, with a fake clock."""
        self.runs = getattr(self, 'runs', 0) + 1
        source, target = self.owner / f'fresh {self.runs}', self.owner / f'published {self.runs}'
        source.write_text('fresh\n')
        clock = {'now': 100.0, 'sleeps': []}
        real_replace, attempts = Path.replace, []
        def replace(path, destination):
            attempts.append(path)
            if len(attempts) <= len(failures):
                raise failures[len(attempts) - 1]
            return real_replace(path, destination)
        def sleep(seconds):
            clock['sleeps'].append(seconds)
            clock['now'] += seconds
        with mock.patch.object(self.bootstrap, 'FRESH_FILE_HOLDS', holds), \
                mock.patch.object(Path, 'replace', replace), \
                mock.patch.object(self.bootstrap.time, 'monotonic', side_effect=lambda: clock['now']), \
                mock.patch.object(self.bootstrap.time, 'sleep', side_effect=sleep):
            try:
                self.bootstrap.replace_fresh(source, target)
                outcome = None
            except OSError as error:
                outcome = error
        return outcome, attempts, clock, target

    @staticmethod
    def held(code):
        error = PermissionError(13, f'fixture hold {code}')
        error.winerror = code
        return error

    def test_holds_are_waited_out_with_a_short_backoff_then_the_first_error_is_raised(self):
        outcome, attempts, clock, target = self.scripted([self.held(5), self.held(32), self.held(33)])
        self.assertIsNone(outcome)
        self.assertEqual(len(attempts), 4)
        self.assertEqual(target.read_text(), 'fresh\n')
        self.assertTrue(all(0 < delay <= 0.2 for delay in clock['sleeps']), clock)
        first = self.held(32)
        outcome, attempts, clock, target = self.scripted([first] + [self.held(33)] * 100)
        self.assertIs(outcome, first)
        self.assertAlmostEqual(clock['now'] - 100.0, 2.0)
        self.assertFalse(target.exists())
        missing = FileNotFoundError(2, 'fixture missing')
        missing.winerror = 2
        for failure, holds in ((missing, (5, 32, 33)), (PermissionError(13, 'plain'), (5, 32, 33)),
                               (self.held(5), ()), (self.held(32), ())):  # () is every other platform
            with self.subTest(failure=failure, holds=holds):
                outcome, attempts, clock, _ = self.scripted([failure], holds)
                self.assertIs(outcome, failure)
                self.assertEqual((len(attempts), clock['sleeps']), (1, []))
        self.assertEqual(self.bootstrap.FRESH_FILE_HOLDS, (5, 32, 33) if os.name == 'nt' else ())

    def test_the_helpers_rename_fresh_output_with_the_two_second_default(self):
        self.assertEqual(inspect.signature(self.bootstrap.replace_fresh).parameters['timeout'].default, 2.0)
        for name, minimum in (('bootstrap-tools.py', 2), ('prepare-source.py', 2)):
            with self.subTest(script=name):
                tree = ast.parse((NATIVE / name).read_text(encoding='utf-8'))
                calls = [node for node in ast.walk(tree) if isinstance(node, ast.Call)
                         and getattr(node.func, 'id', getattr(node.func, 'attr', None)) == 'replace_fresh']
                self.assertGreaterEqual(len(calls), minimum)
                self.assertTrue(all(len(call.args) == 2 and not call.keywords for call in calls),
                                'A helper passes its own retry timeout')
                direct = [ast.unparse(node) for node in ast.walk(tree) if isinstance(node, ast.Call)
                          and getattr(node.func, 'attr', None) in ('replace', 'rename')
                          and getattr(node.func.value, 'id', None) == 'temporary']
                self.assertEqual(direct, [], 'A fresh temporary file is renamed without the retry')

    def test_downloads_and_build_fixes_are_renamed_through_the_retry(self):
        artifact, data = self.zip_artifact({'tool.py': b'print(1)\n'})
        root = self.bootstrap.ordinary_root(self.linked / '_build/native-tools')
        renames = []
        real = self.bootstrap.replace_fresh
        def recorded(source, target):
            renames.append(Path(target).name)
            return real(source, target)
        with mock.patch.object(self.bootstrap, 'replace_fresh', recorded):
            self.assertEqual(self.bootstrap.fetch(root, artifact, False, lambda *a, **k: io.BytesIO(data)), data)
        release = self.dav1d_release()
        with mock.patch.object(self.source.BOOTSTRAP, 'replace_fresh', recorded), \
                mock.patch.object(self.source.urllib.request, 'urlopen', lambda *a, **k: io.BytesIO(release)):
            self.source.prepare(self.linked / '_build/dav1d')
        self.assertEqual(renames, ['fixture.zip', f'dav1d-{self.source.VERSION}.tar.xz', 'meson.build'])

    def test_publication_raises_its_first_error_after_the_longer_executable_wait(self):
        artifact, data = self.zip_artifact({'tool': b'payload'})
        root = self.bootstrap.ordinary_root(self.linked / '_build/native-tools')
        failures = [self.held(5)] + [self.held(32)] * 20
        attempts, waits = [], []
        def rename(source, target):
            attempts.append(target)
            raise failures[len(attempts) - 1]
        with mock.patch.object(self.bootstrap, 'FRESH_FILE_HOLDS', (5, 32, 33)), \
                mock.patch.object(self.bootstrap, 'PUBLISH_RETRY_DELAYS', (0.1, 0.2, 0.4)), \
                mock.patch.object(Path, 'rename', rename), mock.patch('time.sleep', waits.append):
            with self.assertRaises(PermissionError) as caught:
                self.bootstrap.unpack(root, artifact, data)
        self.assertIs(caught.exception, failures[0])
        self.assertEqual((len(attempts), waits), (4, [0.1, 0.2, 0.4]))
        self.assertEqual(list((root / 'tools').iterdir()), [], 'Staging must be removed')

    @unittest.skipUnless(os.name == 'nt', 'Windows refuses to rename a held file')
    def test_real_holds_of_fresh_downloads_and_stages_are_waited_out(self):
        artifact, data = self.zip_artifact({'tool.py': b'print(1)\n'})
        root = self.bootstrap.ordinary_root(self.linked / '_build/native-tools')
        holds = []
        self.addCleanup(lambda: [held.release() for held in holds])
        checked = self.bootstrap.checked_bytes
        def downloaded(path, expected):
            result = checked(path, expected)
            holds.append(Hold(path, 0.15))  # The download just written, before its rename.
            return result
        started = time.monotonic()
        with mock.patch.object(self.bootstrap, 'checked_bytes', downloaded):
            self.assertEqual(self.bootstrap.fetch(root, artifact, False, lambda *a, **k: io.BytesIO(data)), data)
        self.assertGreaterEqual(time.monotonic() - started, 0.1, 'The hold was not met')
        self.assertEqual((root / 'downloads' / artifact.filename).read_bytes(), data)
        release = self.dav1d_release()
        fix = self.source.apply_build_fixes
        def extracted(source):
            holds.append(Hold(source / 'meson.build', 0.15))  # Extracted moments ago.
            return fix(source)
        root = self.source.build_directory(self.linked / '_build/dav1d')
        (root / f'dav1d-{self.source.VERSION}.tar.xz').write_bytes(release)
        started = time.monotonic()
        with mock.patch.object(self.source, 'apply_build_fixes', extracted):
            source = self.source.prepare(root, offline=True)
        self.assertGreaterEqual(time.monotonic() - started, 0.1, 'The hold was not met')
        self.assertEqual((source / 'meson.build').read_bytes(), b'patched\n')

    @unittest.skipUnless(os.name == 'nt', 'Windows refuses to rename a held file')
    def test_a_hold_beyond_the_budget_raises_the_first_error(self):
        # A 100 ms budget instead of the scripts' 2 s; the hold is released at once after.
        source, target = self.owner / 'fresh.download', self.owner / 'published'
        source.write_bytes(b'fresh')
        held = Hold(source, 2.0)
        self.addCleanup(held.release)
        started = time.monotonic()
        with self.assertRaises(PermissionError) as caught:
            self.bootstrap.replace_fresh(source, target, 0.1)
        elapsed = time.monotonic() - started
        held.release()
        self.assertEqual(caught.exception.winerror, 32)
        self.assertGreaterEqual(elapsed, 0.1)
        self.assertLess(elapsed, 2.0)
        self.assertFalse(target.exists())


if __name__ == '__main__':
    unittest.main()
