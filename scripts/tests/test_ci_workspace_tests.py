"""CI keeps Cargo target coverage while retiring only its successful test artifacts."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / 'ci-workspace-tests.py'
spec = importlib.util.spec_from_file_location('ci_workspace_tests', SCRIPT)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def target(name, kind='test', test=True, required=()):
    return {'name': name, 'kind': [kind], 'test': test, 'doctest': kind == 'lib' and test,
            'required-features': list(required)}


def graph(root, first, second=()):
    return {'target_directory': str(root / '_build'), 'workspace_members': ['a-id', 'b-id'],
            'packages': [{'id': 'a-id', 'name': 'a', 'targets': list(first)},
                         {'id': 'b-id', 'name': 'b', 'targets': list(second)}],
            'ci_active_features': {'a-id': {'enabled'}, 'b-id': set()},
            'resolve': {'nodes': [{'id': 'a-id', 'features': ['enabled']},
                                  {'id': 'b-id', 'features': []}]}}


class WorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.deps = self.root / '_build/debug/deps'
        self.deps.mkdir(parents=True)

    def artifact(self, name='selection', package='a-id', path=None):
        path = path or self.deps / (name + '-0123456789abcdef.exe')
        path.write_bytes(b'generated test executable')
        return {'reason': 'compiler-artifact', 'package_id': package,
                'target': {'name': name, 'kind': ['test']},
                'profile': {'test': True}, 'executable': str(path)}

    def test_plan_keeps_workspace_units_binaries_docs_and_example_builds(self):
        metadata = graph(self.root, [target('a', 'lib'), target('tool', 'bin'),
                                    target('demo', 'example', False),
                                    target('checked', 'example'), target('integration')],
                         [target('b', 'lib', False)])
        gates = runner.plan(metadata)
        args = [gate['args'] for gate in gates]
        self.assertIn(['--lib', '--exclude', 'b'], args)
        self.assertIn(['--bin', 'tool'], args)
        self.assertIn(['--doc', '--exclude', 'b'], args)
        self.assertIn(['--examples', '--no-run'], args)
        self.assertIn(['--example', 'checked'], args)
        self.assertNotIn(['--example', 'demo'], args)
        self.assertIn(['--test', 'integration', '--message-format=json'], args)

    def test_duplicate_names_run_once_and_false_or_unavailable_features_do_not_run(self):
        metadata = graph(self.root, [target('shared'), target('disabled', test=False),
                                    target('optional', required=['enabled'])],
                         [target('shared'), target('missing', required=['absent'])])
        tests = [gate for gate in runner.plan(metadata) if gate['cleanup']]
        self.assertEqual([gate['name'] for gate in tests], ['optional', 'shared'])
        self.assertEqual(tests[1]['packages'], {'a-id', 'b-id'})

    def test_mixed_enabled_and_disabled_same_name_fails_before_running_anything(self):
        metadata = graph(self.root, [target('same')], [target('same', test=False)])
        with self.assertRaisesRegex(ValueError, 'same'):
            runner.plan(metadata)

    def test_success_removes_only_reported_test_executable_and_matching_pdb(self):
        metadata = graph(self.root, [target('selection')])
        artifact = self.artifact()
        executable = Path(artifact['executable'])
        pdb = executable.with_suffix('.pdb')
        pdb.write_bytes(b'owned symbols')
        unrelated = self.deps / 'unrelated-0123456789abcdef.exe'
        unrelated.write_bytes(b'keep')
        commands = []
        def invoke(command, root, gate):
            commands.append(command)
            return 0, [artifact] if gate['cleanup'] else []
        self.assertEqual(runner.execute(self.root, metadata, invoke), 0)
        self.assertFalse(executable.exists())
        self.assertFalse(pdb.exists())
        self.assertTrue(unrelated.exists())
        self.assertTrue(all('--workspace' in command and '--locked' in command for command in commands))
        self.assertTrue(all(command[command.index('--features') + 1] == 'a/enabled' for command in commands))

    @contextlib.contextmanager
    def retirement_clock(self, windows=True, on_sleep=None):
        clock = {'now': 0.0, 'sleeps': []}
        def sleep(seconds):
            clock['sleeps'].append(seconds)
            clock['now'] += seconds
            if on_sleep is not None:
                on_sleep()
        with mock.patch.object(runner, 'WINDOWS_SHARING_ERRORS',
                               (32, 33) if windows else (), create=True), \
                mock.patch('time.monotonic', side_effect=lambda: clock['now']), \
                mock.patch('time.sleep', side_effect=sleep):
            yield clock

    def retire_artifact(self, artifact):
        gate = runner.plan(graph(self.root, [target('selection')]))[0]
        runner.retire(self.root / '_build', gate, [artifact])

    @staticmethod
    def sharing_error(code):
        failure = PermissionError(13, 'owned fixture native failure')
        failure.winerror = code
        return failure

    def test_transient_windows_sharing_violations_retry_only_the_validated_artifact(self):
        for code in (32, 33):
            with self.subTest(code=code):
                artifact = self.artifact()
                executable = Path(artifact['executable'])
                unrelated = self.deps / 'unrelated-0123456789abcdef.exe'
                unrelated.write_bytes(b'keep')
                unlink = Path.unlink
                attempts = []
                def locked(path):
                    attempts.append(path)
                    if len(attempts) < 3:
                        raise self.sharing_error(code)
                    return unlink(path)
                with self.retirement_clock() as clock, \
                        mock.patch.object(Path, 'unlink', locked):
                    self.retire_artifact(artifact)
                self.assertEqual(attempts, [executable] * 3)
                self.assertEqual(clock['sleeps'], [0.1, 0.1])
                self.assertFalse(executable.exists())
                self.assertEqual(unrelated.read_bytes(), b'keep')

    def test_persistent_sharing_stops_at_deadline_and_preserves_first_error(self):
        artifact = self.artifact()
        first = self.sharing_error(32)
        later = self.sharing_error(33)
        attempts = []
        def locked(path):
            attempts.append(path)
            raise first if len(attempts) == 1 else later
        with self.retirement_clock() as clock, mock.patch.object(Path, 'unlink', locked):
            with self.assertRaises(OSError) as caught:
                self.retire_artifact(artifact)
        self.assertIs(caught.exception, first)
        self.assertGreater(len(attempts), 1)
        self.assertLessEqual(len(attempts), 21)
        self.assertAlmostEqual(clock['now'], 2.0)
        self.assertTrue(all(0 < delay <= 0.1 for delay in clock['sleeps']))
        self.assertTrue(Path(artifact['executable']).exists())

    def test_permission_errors_and_other_platforms_never_retry(self):
        for code, windows in [(5, True), (None, True), (32, False), (33, False)]:
            with self.subTest(code=code, windows=windows):
                artifact = self.artifact()
                failure = self.sharing_error(code)
                with self.retirement_clock(windows=windows) as clock, \
                        mock.patch.object(Path, 'unlink', side_effect=failure) as unlink:
                    with self.assertRaises(OSError) as caught:
                        self.retire_artifact(artifact)
                self.assertIs(caught.exception, failure)
                unlink.assert_called_once()
                self.assertEqual(clock['sleeps'], [])
                self.assertTrue(Path(artifact['executable']).exists())

    def test_retry_refuses_a_substituted_regular_file(self):
        artifact = self.artifact()
        executable = Path(artifact['executable'])
        replacement = self.deps / 'replacement'
        replacement.write_bytes(b'preserve replacement')
        with self.retirement_clock(on_sleep=lambda: replacement.replace(executable)), \
                mock.patch.object(Path, 'unlink', side_effect=self.sharing_error(32)) as unlink:
            with self.assertRaisesRegex(ValueError, 'changed|Unowned'):
                self.retire_artifact(artifact)
        unlink.assert_called_once()
        self.assertEqual(executable.read_bytes(), b'preserve replacement')

    def test_retry_refuses_a_directory_substitution(self):
        artifact = self.artifact()
        executable = Path(artifact['executable'])
        saved = self.deps / 'saved'
        def substitute():
            executable.replace(saved)
            executable.mkdir()
        with self.retirement_clock(on_sleep=substitute), \
                mock.patch.object(Path, 'unlink', side_effect=self.sharing_error(32)) as unlink:
            with self.assertRaisesRegex(ValueError, 'changed|Unowned'):
                self.retire_artifact(artifact)
        unlink.assert_called_once()
        self.assertTrue(executable.is_dir())
        self.assertEqual(saved.read_bytes(), b'generated test executable')

    def test_retry_refuses_a_symlink_substitution(self):
        artifact = self.artifact()
        executable = Path(artifact['executable'])
        protected = self.root / 'protected'
        protected.write_bytes(b'preserve target')
        link = self.deps / 'replacement-link'
        try:
            link.symlink_to(protected)
        except OSError:
            self.skipTest('symlink creation requires platform privilege')
        with self.retirement_clock(on_sleep=lambda: link.replace(executable)), \
                mock.patch.object(Path, 'unlink', side_effect=self.sharing_error(32)) as unlink:
            with self.assertRaisesRegex(ValueError, 'changed|Unowned'):
                self.retire_artifact(artifact)
        unlink.assert_called_once()
        self.assertTrue(executable.is_symlink())
        self.assertEqual(protected.read_bytes(), b'preserve target')

    def test_failed_gate_keeps_artifacts_and_stops_later_targets(self):
        metadata = graph(self.root, [target('selection'), target('z_later')])
        artifact = self.artifact()
        visited = []
        def invoke(command, root, gate):
            visited.append(gate['name'])
            return (37, [artifact]) if gate['cleanup'] else (0, [])
        self.assertEqual(runner.execute(self.root, metadata, invoke, batch_size=1), 37)
        self.assertTrue(Path(artifact['executable']).exists())
        self.assertNotIn('z_later', visited)

    def test_outside_or_unexpected_artifact_never_gets_deleted(self):
        metadata = graph(self.root, [target('selection')])
        for path in [self.root / 'selection-0123456789abcdef.exe',
                     self.deps / 'other-0123456789abcdef.exe']:
            with self.subTest(path=path):
                artifact = self.artifact(path=path)
                def invoke(command, root, gate):
                    return 0, [artifact] if gate['cleanup'] else []
                with self.assertRaises(ValueError):
                    runner.execute(self.root, metadata, invoke)
                self.assertTrue(path.exists())

    def test_missing_reported_target_and_external_target_directory_fail_closed(self):
        metadata = graph(self.root, [target('selection')])
        with self.assertRaisesRegex(ValueError, 'artifact'):
            runner.execute(self.root, metadata, lambda *_: (0, []))
        metadata['target_directory'] = str(self.root / 'user-output')
        calls = []
        with self.assertRaisesRegex(ValueError, '_build'):
            runner.execute(self.root, metadata, lambda *args: calls.append(args))
        self.assertEqual(calls, [])

    def test_symlinked_owned_name_cannot_authorize_another_file(self):
        protected = self.root / 'protected'
        protected.write_bytes(b'keep')
        link = self.deps / 'selection-0123456789abcdef.exe'
        try:
            link.symlink_to(protected)
        except OSError:
            self.skipTest('symlink creation requires platform privilege')
        artifact = {'reason': 'compiler-artifact', 'package_id': 'a-id',
                    'target': {'name': 'selection', 'kind': ['test']},
                    'profile': {'test': True}, 'executable': str(link)}
        metadata = graph(self.root, [target('selection')])
        with self.assertRaises(ValueError):
            runner.execute(self.root, metadata,
                           lambda _, __, gate: (0, [artifact] if gate['cleanup'] else []))
        self.assertTrue(protected.exists())
        self.assertTrue(link.is_symlink())


    def test_real_command_stream_parses_cargo_artifact_and_keeps_test_output(self):
        artifact = self.artifact()
        gate = runner.plan(graph(self.root, [target('selection')]))[0]
        command = [sys.executable, '-c',
                   'import json; print(' + repr(json.dumps(artifact)) + '); print("test selected ... ok"); print(' + repr('{"error":"readable diagnostic"}') + ')']
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code, artifacts = runner.run_command(command, self.root, gate)
        self.assertEqual(code, 0)
        self.assertEqual(artifacts, [artifact])
        self.assertIn('test selected ... ok', output.getvalue())
        self.assertIn('readable diagnostic', output.getvalue())

    def test_redirected_legacy_console_preserves_unicode_test_output(self):
        payload = "test Unicode \u6587\u6863 \U0001f319 ... ok"
        raw = io.BytesIO()
        output = io.TextIOWrapper(raw, encoding='cp1252', errors='strict')
        gate = {'cleanup': False, 'packages': set()}
        command = [sys.executable, '-c',
                   'import sys; sys.stdout.buffer.write(' + repr((payload + '\n').encode('utf-8')) + ')']
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            runner.configure_output()
            code, artifacts = runner.run_command(command, self.root, gate)
        output.flush()
        self.assertEqual(code, 0)
        self.assertEqual(artifacts, [])
        self.assertIn(payload, raw.getvalue().decode('utf-8'))

    def test_unexpected_package_or_non_test_profile_cannot_authorize_cleanup(self):
        metadata = graph(self.root, [target('selection')])
        for change in [{'package_id': 'external'}, {'profile': {'test': False}}]:
            artifact = self.artifact()
            artifact.update(change)
            with self.assertRaisesRegex(ValueError, 'identity'):
                runner.execute(self.root, metadata,
                               lambda _, __, gate: (0, [artifact] if gate['cleanup'] else []))
            self.assertTrue(Path(artifact['executable']).exists())

    def test_active_tree_ignores_metadata_inactive_platform_features(self):
        metadata = graph(self.root, [target('enabled', required=['enabled']),
                                    target('inactive', required=['unix-only'])])
        for package in metadata['packages']:
            package.update(version='0.1.0', manifest_path=str(self.root / package['name'] / 'Cargo.toml'),
                           features={'enabled': [], 'unix-only': []})
        metadata['resolve']['nodes'][0]['features'].append('unix-only')
        tree = (f'a v0.1.0 ({self.root / "a"})|enabled\n'
                f'b v0.1.0 ({self.root / "b"})|\n')
        runner.apply_feature_tree(metadata, tree)
        self.assertEqual([gate['name'] for gate in runner.plan(metadata)], ['enabled'])
        with self.assertRaisesRegex(ValueError, 'omitted'):
            runner.apply_feature_tree(metadata, tree.splitlines()[0])

    def test_excluded_disabled_library_keeps_its_active_root_features(self):
        metadata = graph(self.root, [target('a', 'lib')], [target('b', 'lib', False)])
        metadata['ci_active_features']['b-id'] = {'default', 'from-default'}
        metadata['resolve']['nodes'][1]['features'] = ['default', 'from-default', 'inactive-platform']
        commands = []
        def invoke(command, root, gate):
            commands.append(command)
            return 0, []
        self.assertEqual(runner.execute(self.root, metadata, invoke), 0)
        self.assertTrue(commands)
        for command in commands:
            self.assertIn('--exclude', command)
            self.assertEqual(command[command.index('--features') + 1],
                             'a/enabled,b/default,b/from-default')

    def test_native_preflight_rejects_environment_and_hierarchical_target_overrides(self):
        cargo_home = self.root / 'cargo-home'
        cargo_home.mkdir()
        child = self.root / 'workspace' / 'member'
        child.mkdir(parents=True)
        environment = {'CARGO_HOME': str(cargo_home)}
        runner.require_native_configuration(child, environment)
        with self.assertRaisesRegex(ValueError, 'CARGO_BUILD_TARGET'):
            runner.require_native_configuration(child, dict(environment, CARGO_BUILD_TARGET='other-target'))
        for directory in [child / '.cargo', self.root / '.cargo', cargo_home]:
            directory.mkdir(exist_ok=True)
            for filename in ['config', 'config.toml']:
                config = directory / filename
                config.write_text('[build]\ntarget = "other-target"\n')
                with self.assertRaisesRegex(ValueError, 'build.target'):
                    runner.require_native_configuration(child, environment)
                config.unlink()

    def test_native_preflight_uses_legacy_config_precedence_and_rejects_includes(self):
        directory = self.root / '.cargo'
        directory.mkdir()
        environment = {'CARGO_HOME': str(self.root / 'cargo-home')}
        (directory / 'config').write_text('[build]\njobs = 2\n')
        (directory / 'config.toml').write_text('[build]\ntarget = "ignored-by-Cargo"\n')
        runner.require_native_configuration(self.root, environment)
        (directory / 'config').write_text('include = "another.toml"\n')
        with self.assertRaisesRegex(ValueError, 'include'):
            runner.require_native_configuration(self.root, environment)

if __name__ == '__main__':
    unittest.main()