"""Bounded Cargo batches preserve selections and fail-closed artifact retirement."""
import contextlib
import io
import json
from pathlib import Path
import sys
import unittest

import test_ci_workspace_tests as fixtures

graph, runner, target = fixtures.graph, fixtures.runner, fixtures.target


class BatchingTests(unittest.TestCase):
    setUp = fixtures.WorkspaceTests.setUp
    artifact = fixtures.WorkspaceTests.artifact

    def test_batches_preserve_all_non_integration_gates_and_exact_selections(self):
        metadata = graph(self.root, [target('a', 'lib'), target('tool', 'bin'),
                                    target('compile', 'example', False),
                                    target('checked', 'example'), target('alpha'),
                                    target('beta'), target('shared'), target('zulu')],
                         [target('shared'), target('disabled', test=False)])
        serial = runner.plan(metadata)
        batches = runner.batch_integrations(serial, 4)
        self.assertEqual([g for g in batches if not g['cleanup']],
                         [g for g in serial if not g['cleanup']])
        tests = [g for g in batches if g['cleanup']]
        self.assertEqual([g['args'] for g in tests], [
            ['--test', 'alpha', '--test', 'beta', '--test', 'shared', '--message-format=json'],
            ['--test', 'zulu', '--message-format=json']])
        self.assertEqual(tests[0]['targets'], {
            'alpha': {'a-id'}, 'beta': {'a-id'}, 'shared': {'a-id', 'b-id'}})
        self.assertEqual(tests[1]['targets'], {'zulu': {'a-id'}})
        self.assertEqual(runner.batch_integrations(serial, 1), serial)

    def test_default_executes_eight_targets_then_the_remainder(self):
        metadata = graph(self.root, [target(f'test_{index}') for index in range(9)])
        calls = []
        def invoke(command, root, gate):
            calls.append(command)
            return 0, [self.artifact(name) for name in gate['targets']]
        self.assertEqual(runner.execute(self.root, metadata, invoke), 0)
        self.assertEqual([command.count('--test') for command in calls], [8, 1])
        self.assertEqual(list(self.deps.iterdir()), [])

    def test_same_name_group_stays_atomic_at_a_smaller_requested_bound(self):
        serial = runner.plan(graph(self.root, [target('a'), target('shared'), target('z')],
                                   [target('shared')]))
        batches = runner.batch_integrations(serial, 1)
        self.assertEqual([g['name'] for g in batches], ['a', 'shared', 'z'])
        self.assertEqual(batches[1]['targets'], {'shared': {'a-id', 'b-id'}})

    def test_invalid_batch_bound_fails_before_invoking_cargo(self):
        for size in (0, -1, 9, 1.5, True):
            with self.subTest(size=size):
                called = []
                with self.assertRaises(ValueError):
                    runner.execute(self.root, graph(self.root, [target('a')]),
                                   lambda *args: called.append(args), batch_size=size)
                self.assertEqual(called, [])

    def batch(self, names=('alpha', 'beta')):
        return runner.batch_integrations(
            runner.plan(graph(self.root, [target(name) for name in names])), 4)[0]

    def test_success_retires_exact_batch_and_keeps_unrelated_output(self):
        artifacts = [self.artifact('alpha'), self.artifact('beta')]
        unrelated = self.deps / 'unrelated-0123456789abcdef.exe'
        unrelated.write_bytes(b'keep')
        runner.retire(self.root / '_build', self.batch(), artifacts)
        self.assertTrue(unrelated.exists())
        self.assertTrue(all(not Path(a['executable']).exists() for a in artifacts))

    def test_complete_batch_is_validated_before_any_file_is_removed(self):
        for error in ('missing', 'duplicate', 'wrong_pair', 'outside'):
            with self.subTest(error=error):
                artifacts = [self.artifact('alpha'), self.artifact('beta')]
                if error == 'missing':
                    artifacts.pop()
                elif error == 'duplicate':
                    artifacts[1] = artifacts[0]
                elif error == 'wrong_pair':
                    artifacts[1]['package_id'] = 'b-id'
                else:
                    artifacts[1]['executable'] = str(self.root / 'beta-0123456789abcdef.exe')
                with self.assertRaises((ValueError, FileNotFoundError)):
                    runner.retire(self.root / '_build', self.batch(), artifacts)
                self.assertTrue(Path(artifacts[0]['executable']).exists())

    def test_duplicate_names_require_distinct_reported_executables(self):
        gate = runner.batch_integrations(runner.plan(
            graph(self.root, [target('same')], [target('same')])), 4)[0]
        first = self.artifact('same')
        second = dict(first, package_id='b-id')
        with self.assertRaisesRegex(ValueError, 'duplicate|Duplicate'):
            runner.retire(self.root / '_build', gate, [first, second])
        self.assertTrue(Path(first['executable']).exists())
        second = self.artifact('same', 'b-id', self.deps / 'same-fedcba9876543210.exe')
        runner.retire(self.root / '_build', gate, [first, second])
        self.assertFalse(Path(first['executable']).exists())
        self.assertFalse(Path(second['executable']).exists())

    def test_failed_batch_keeps_every_artifact_and_stops_later_batches(self):
        metadata = graph(self.root, [target(name) for name in ('a', 'b', 'c', 'd', 'later')])
        artifacts = [self.artifact(name) for name in ('a', 'b', 'c', 'd')]
        calls = []
        def invoke(command, root, gate):
            calls.append(command)
            return 37, artifacts
        self.assertEqual(runner.execute(self.root, metadata, invoke, batch_size=4), 37)
        self.assertEqual(len(calls), 1)
        self.assertNotIn('later', calls[0])
        self.assertNotIn('--no-fail-fast', calls[0])
        self.assertEqual(calls[0][calls[0].index('--features') + 1], 'a/enabled')
        self.assertTrue(all(Path(a['executable']).exists() for a in artifacts))

    def test_cargo_stream_collects_each_target_and_preserves_non_cargo_json(self):
        artifacts = [self.artifact('alpha'), self.artifact('beta')]
        lines = ['{invalid json', '{"test":"custom harness output"}',
                 *[json.dumps(a) for a in artifacts], 'test selected ... ok',
                 '{"reason":"build-finished","success":true}']
        command = [sys.executable, '-c', 'print(' + repr('\n'.join(lines)) + ')']
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code, reported = runner.run_command(command, self.root, self.batch())
        self.assertEqual(code, 0)
        self.assertEqual(reported, artifacts)
        self.assertIn('{invalid json', output.getvalue())
        self.assertIn('{"test":"custom harness output"}', output.getvalue())
        self.assertIn('test selected ... ok', output.getvalue())

    def test_extra_cargo_artifact_fails_without_retiring_anything(self):
        artifacts = [self.artifact('alpha'), self.artifact('beta'), self.artifact('extra')]
        command = [sys.executable, '-c',
                   'print(' + repr('\n'.join(json.dumps(a) for a in artifacts)) + ')']
        with contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(ValueError, 'more integration artifacts'):
                runner.run_command(command, self.root, self.batch())
        self.assertTrue(all(Path(a['executable']).exists() for a in artifacts))


if __name__ == '__main__':
    unittest.main()
