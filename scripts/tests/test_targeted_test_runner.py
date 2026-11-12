"""Actual child rendezvous verifies targeted runner concurrency without timing guesses."""
import importlib.util
import json
import os
from pathlib import Path
import socketserver
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / 'run-targeted-tests.py'

FIXTURE = r'''
import json, os, socket, sys
name = sys.argv[1]
args = sys.argv[2:]
tests = [name + '::first', name + '::second']
if '--list' in args:
    for test in tests:
        print(test + ': test')
    sys.exit(0)
filters = args[:args.index('--test-threads')]
matched = [test for test in tests if not filters or any(value in test for value in filters)]
record = {'name': name, 'cwd': os.getcwd(), 'tmp': os.environ['TMPDIR'],
          'render': os.environ['MIDNIGHT_UI_RENDER_DIR'],
          'threads': os.environ['RUST_TEST_THREADS'],
          'provider': os.environ.get('HOST_TEST_INSTALL_ROOT'),
          'native': os.environ.get('MIDNIGHT_NATIVE_TEST_ROOT')}
address = os.environ.get('TEST_RENDEZVOUS')
if address:
    host, port = address.split(':')
    with socket.create_connection((host, int(port)), timeout=10) as connection:
        connection.sendall((json.dumps(record) + '\n').encode())
        reply = connection.makefile('rb').readline()
        if reply != b'go\n':
            sys.exit(91)
print(json.dumps(record), flush=True)
if name == 'failure':
    print('test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s')
    sys.exit(7)
print(f'test result: ok. {len(matched)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s')
'''


class Rendezvous:
    def __init__(self, width):
        self.records = []
        self.lock = threading.Lock()
        self.ready = threading.Event()
        self.barrier = threading.Barrier(width, timeout=10)
        owner = self
        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                record = json.loads(self.rfile.readline())
                with owner.lock:
                    owner.records.append(record)
                    owner.ready.set()
                try:
                    owner.barrier.wait()
                    self.wfile.write(b'go\n')
                except (threading.BrokenBarrierError, OSError):
                    pass
        self.server = socketserver.ThreadingTCPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def address(self):
        return ':'.join(map(str, self.server.server_address))

    def close(self):
        self.barrier.abort()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


class TargetedRunnerTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), 'The targeted artifact runner must exist')
        spec = importlib.util.spec_from_file_location('targeted_test_runner', SCRIPT)
        self.runner = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = self.runner
        spec.loader.exec_module(self.runner)
        cpu_patch = mock.patch.object(self.runner, "available_cpus", return_value=4)
        cpu_patch.start()
        self.addCleanup(cpu_patch.stop)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.package = self.root / 'crates/demo'
        self.package.mkdir(parents=True)
        (self.package / 'Cargo.toml').write_text('[package]\nname="demo"\nversion="0.1.0"\n')
        self.deps = self.root / '_build/debug/deps'
        self.deps.mkdir(parents=True)
        self.fixture = self.root / 'fixture.py'
        self.fixture.write_text(FIXTURE)
        self.metadata = {'workspace_root': str(self.root), 'target_directory': str(self.root / '_build'),
                         'workspace_members': ['demo-id'], 'packages': [{
                             'id': 'demo-id', 'name': 'demo', 'manifest_path': str(self.package / 'Cargo.toml'),
                             'targets': []}]}
        self.receipts = [{'reason': 'build-finished', 'success': True}]
        self.launched = []
        self.children = []
        self.lock = threading.Lock()

    def add(self, name, filter=None):
        filename = name + '-0123456789abcdef' + ('.exe' if os.name == 'nt' else '')
        executable = self.deps / filename
        executable.write_bytes(b'owned test fixture')
        self.metadata['packages'][0]['targets'].append({'name': name, 'kind': ['test'], 'test': True})
        self.receipts.append({'reason': 'compiler-artifact', 'package_id': 'demo-id',
                              'target': {'name': name, 'kind': ['test']},
                              'profile': {'test': True}, 'executable': str(executable)})
        return self.runner.Selection('demo', 'test', name, filter)

    def spawn(self, command, **kwargs):
        name = Path(command[0]).name.split('-')[0]
        with self.lock:
            self.launched.append((name, list(command[1:])))
        child = subprocess.Popen([sys.executable, str(self.fixture), name, *command[1:]], **kwargs)
        with self.lock:
            self.children.append(child)
        return child

    def run_cases(self, selections, **kwargs):
        env = dict(os.environ, HOST_TEST_INSTALL_ROOT='preserved-provider',
                   MIDNIGHT_NATIVE_TEST_ROOT='preserved-native-root')
        env.update(kwargs.pop('environment', {}))
        log_dir = self.root / '_build/qa/run'
        result = self.runner.execute(self.root, self.metadata, self.receipts, selections,
                                     log_dir, jobs=kwargs.pop('jobs', 2),
                                     thread_budget=kwargs.pop('thread_budget', 4),
                                     environment=env, spawn=kwargs.pop('spawn', self.spawn), **kwargs)
        return result, json.loads((log_dir / 'summary.json').read_text())

    def test_nested_workspace_runs_from_plugin_cwd_but_keeps_logs_under_repo_build(self):
        selected = self.add('alpha')
        self.metadata['workspace_root'] = str(self.package)
        code, summary = self.run_cases([selected], workspace=self.package)
        self.assertEqual(code, 0, summary)
        self.assertEqual(summary['workspace'], str(self.package))
        case = summary['cases'][0]
        self.assertEqual(case['cwd'], str(self.package))
        self.assertTrue(Path(case['test_log']).is_relative_to(self.root / '_build'))
        record = json.loads(Path(case['test_log']).read_text().splitlines()[0])
        self.assertEqual(record['cwd'], str(self.package))
        self.assertTrue(Path(record['tmp']).is_relative_to(self.root / '_build'))

    def test_nested_metadata_requires_explicit_matching_workspace(self):
        selected = self.add('alpha')
        self.metadata['workspace_root'] = str(self.package)
        for workspace in (None, self.root):
            with self.subTest(workspace=workspace):
                with self.assertRaisesRegex(ValueError, 'different workspace'):
                    self.run_cases([selected], workspace=workspace)
        self.assertEqual(self.launched, [])

    def test_manifest_outside_selected_plugin_workspace_is_rejected(self):
        selected = self.add('alpha')
        workspace = self.root / 'plugins/other'
        workspace.mkdir(parents=True)
        self.metadata['workspace_root'] = str(workspace)
        with self.assertRaisesRegex(ValueError, 'manifest'):
            self.run_cases([selected], workspace=workspace)
        self.assertEqual(self.launched, [])

    def test_workspace_escape_and_lexical_alias_are_rejected_before_launch(self):
        selected = self.add('alpha')
        for workspace in (self.root.parent, self.package / '..' / 'demo'):
            with self.subTest(workspace=workspace):
                self.metadata['workspace_root'] = str(workspace)
                with self.assertRaisesRegex(ValueError, 'ordinary.*workspace|workspace.*ordinary'):
                    self.run_cases([selected], workspace=workspace)
        self.assertEqual(self.launched, [])

    def test_linked_workspace_is_rejected_before_launch(self):
        selected = self.add('alpha')
        link = self.root / 'plugin-link'
        try:
            link.symlink_to(self.package, target_is_directory=True)
        except OSError as error:
            self.skipTest(f'Directory symlinks unavailable: {error}')
        self.metadata['workspace_root'] = str(self.package)
        with self.assertRaisesRegex(ValueError, 'ordinary.*workspace|workspace.*ordinary'):
            self.run_cases([selected], workspace=link)
        self.assertEqual(self.launched, [])

    def test_two_real_children_rendezvous_and_each_has_private_output_and_package_cwd(self):
        barrier = Rendezvous(2)
        self.addCleanup(barrier.close)
        selections = [self.add(name) for name in ['alpha', 'beta', 'gamma', 'delta']]
        code, summary = self.run_cases(selections, environment={'TEST_RENDEZVOUS': barrier.address})
        self.assertEqual(code, 0, summary)
        self.assertEqual(summary['jobs'], 2)
        self.assertEqual(summary['peak_processes'], 2)
        self.assertEqual(len(barrier.records), 4)
        self.assertEqual(len({row['tmp'] for row in barrier.records}), 4)
        self.assertEqual(len({row['render'] for row in barrier.records}), 4)
        for row in barrier.records:
            self.assertEqual(row['cwd'], str(self.package))
            self.assertEqual(row['provider'], 'preserved-provider')
            self.assertEqual(row['native'], 'preserved-native-root')
            self.assertLessEqual(int(row['threads']) * summary['jobs'], summary['thread_budget'])
        self.assertTrue(all(child.poll() == 0 for child in self.children))

    def test_filters_on_one_artifact_share_list_and_execution_and_validate_each_filter(self):
        first = self.add('alpha', 'alpha')
        second = self.runner.Selection('demo', 'test', 'alpha', 'first')
        code, summary = self.run_cases([first, second])
        self.assertEqual(code, 0, summary)
        self.assertEqual(len(self.launched), 2)
        self.assertEqual(summary['cases'][0]['matched'], 2)
        self.assertEqual(summary['cases'][0]['filters'], ['alpha', 'first'])
        self.assertEqual(summary['cases'][0]['counts']['passed'], 2)

    def test_nonmatching_filter_fails_before_any_test_body(self):
        code, summary = self.run_cases([self.add('alpha', 'absent')])
        self.assertNotEqual(code, 0)
        self.assertIn('matched no tests', summary['cases'][0]['error'])
        self.assertEqual(len(self.launched), 1)
        self.assertIn('--list', self.launched[0][1])

    def test_serial_failure_preserves_artifacts_logs_and_does_not_admit_later_case(self):
        code, summary = self.run_cases([self.add('failure'), self.add('later')], jobs=1)
        self.assertNotEqual(code, 0)
        self.assertEqual(summary['cases'][0]['exit_code'], 7)
        self.assertEqual(summary['peak_processes'], 1)
        self.assertEqual(summary['cases'][1]['status'], 'not-run')
        self.assertTrue(all(name == 'failure' for name, _ in self.launched))
        self.assertEqual(len(list(self.deps.iterdir())), 2)
        self.assertTrue(Path(summary['cases'][0]['test_log']).is_file())

    def test_summary_counts_are_anchored_and_120_is_not_zero(self):
        text = 'noise 0 passed; 0 failed\ntest result: ok. 120 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n'
        self.assertEqual(self.runner.parse_counts(text)['passed'], 120)
        self.assertIsNone(self.runner.parse_counts('120 passed; 0 failed'))

    def test_duplicate_artifact_is_rejected_before_launch(self):
        selected = self.add('alpha')
        self.receipts.append(dict(self.receipts[-1]))
        with self.assertRaises(ValueError):
            self.run_cases([selected])
        self.assertEqual(self.launched, [])

    def test_launch_error_is_summarized_and_earlier_artifacts_are_retained(self):
        def refuse(*args, **kwargs):
            raise OSError('fixture spawn failure')
        code, summary = self.run_cases([self.add('alpha')], spawn=refuse)
        self.assertNotEqual(code, 0)
        self.assertIn('fixture spawn failure', summary['cases'][0]['error'])
        self.assertEqual(len(list(self.deps.iterdir())), 1)

    def test_each_coalesced_filter_must_match(self):
        selected = self.add('alpha', 'first')
        absent = self.runner.Selection('demo', 'test', 'alpha', 'absent')
        code, summary = self.run_cases([selected, absent])
        self.assertEqual(code, 1)
        self.assertEqual(summary['cases'][0]['filter_matches'], {'first': 1, 'absent': 0})
        self.assertEqual(len(self.launched), 1)

    def test_outside_artifact_is_rejected_before_launch(self):
        selected = self.add('alpha')
        outside = self.root / Path(self.receipts[-1]['executable']).name
        outside.write_bytes(b'not in the target directory')
        self.receipts[-1]['executable'] = str(outside)
        with self.assertRaisesRegex(ValueError, 'Unowned'):
            self.run_cases([selected])
        self.assertEqual(self.launched, [])

    def test_artifact_change_during_listing_prevents_test_execution(self):
        selected = self.add('alpha')
        artifact = Path(self.receipts[-1]['executable'])
        def change_after_listing(command, **kwargs):
            child = self.spawn(command, **kwargs)
            original_wait = child.wait
            def wait(*args, **options):
                code = original_wait(*args, **options)
                artifact.write_bytes(b'changed after validated receipt')
                return code
            child.wait = wait
            return child
        code, summary = self.run_cases([selected], spawn=change_after_listing)
        self.assertEqual(code, 1)
        self.assertIn('changed after listing', summary['cases'][0]['error'])
        self.assertEqual(len(self.launched), 1)

    def test_interruption_terminates_owned_child_and_preserves_unrelated_process(self):
        barrier = Rendezvous(2)
        self.addCleanup(barrier.close)
        options = {'stdin': subprocess.PIPE, 'stdout': subprocess.DEVNULL, 'stderr': subprocess.DEVNULL}
        if os.name == 'nt':
            options['creationflags'] = subprocess.CREATE_NO_WINDOW
        unrelated = subprocess.Popen([sys.executable, '-c', 'import sys; sys.stdin.buffer.read()'], **options)
        def stop_unrelated():
            unrelated.stdin.close()
            unrelated.wait(timeout=5)
        self.addCleanup(stop_unrelated)
        def interrupt(_):
            self.assertTrue(barrier.ready.wait(timeout=10), 'Owned test child must reach the rendezvous')
            raise KeyboardInterrupt
        with mock.patch.object(self.runner, 'as_completed', side_effect=interrupt):
            code, summary = self.run_cases([self.add('alpha')], jobs=1,
                                          environment={'TEST_RENDEZVOUS': barrier.address})
        self.assertEqual(code, 130)
        self.assertTrue(summary['interrupted'])
        self.assertTrue(all(child.poll() is not None for child in self.children))
        self.assertIsNone(unrelated.poll())
        self.assertEqual(summary['cleanup_errors'], [])

    def test_exceptional_wait_and_timeout_do_not_abandon_live_child(self):
        for timeout in (False, True):
            with self.subTest(timeout=timeout):
                barrier = Rendezvous(2)
                self.addCleanup(barrier.close)
                selected = self.add('timeout' if timeout else 'waiterror')
                def failing_wait(command, **kwargs):
                    child = self.spawn(command, **kwargs)
                    if '--list' not in command:
                        original_wait = child.wait
                        first = True
                        def wait(*args, **options):
                            nonlocal first
                            if first:
                                first = False
                                self.assertTrue(barrier.ready.wait(timeout=10))
                                if timeout:
                                    raise subprocess.TimeoutExpired(command, 300)
                                raise OSError('fixture wait failed with live child')
                            return original_wait(*args, **options)
                        child.wait = wait
                    return child
                log = self.root / '_build/qa' / ('timeout' if timeout else 'waiterror')
                code = self.runner.execute(self.root, self.metadata, self.receipts, [selected], log,
                                           jobs=1, spawn=failing_wait,
                                           environment=dict(os.environ, TEST_RENDEZVOUS=barrier.address))
                summary = json.loads((log / 'summary.json').read_text())
                self.assertEqual(code, 1)
                self.assertIn('exceeded' if timeout else 'fixture wait failed', summary['cases'][0]['error'])
                self.assertTrue(all(child.poll() is not None for child in self.children))

    def test_one_cpu_caps_requested_parallelism_without_rendezvous(self):
        with mock.patch.object(self.runner, 'available_cpus', return_value=1):
            code, summary = self.run_cases([self.add('alpha'), self.add('beta')], jobs=4)
        self.assertEqual(code, 0)
        self.assertEqual(summary['jobs'], 1)
        self.assertEqual(summary['peak_processes'], 1)

    def test_final_drain_retries_a_retained_child_after_one_shot_termination_failure(self):
        barrier = Rendezvous(2)
        self.addCleanup(barrier.close)
        attempts = []
        def failing_once(command, **kwargs):
            child = self.spawn(command, **kwargs)
            if '--list' not in command:
                original_wait, original_terminate = child.wait, child.terminate
                first_wait = True
                def wait(*args, **options):
                    nonlocal first_wait
                    if first_wait:
                        first_wait = False
                        self.assertTrue(barrier.ready.wait(timeout=10))
                        raise OSError('fixture exceptional wait')
                    return original_wait(*args, **options)
                def terminate():
                    attempts.append(child.pid)
                    if len(attempts) == 1:
                        raise OSError('fixture one-shot terminate failure')
                    return original_terminate()
                child.wait, child.terminate = wait, terminate
            return child
        code, summary = self.run_cases([self.add('alpha')], spawn=failing_once, jobs=1,
                                      environment={'TEST_RENDEZVOUS': barrier.address})
        self.assertEqual(code, 1)
        self.assertIn('one-shot terminate failure', summary['cases'][0]['error'])
        self.assertEqual(len(attempts), 2)
        self.assertTrue(all(child.poll() is not None for child in self.children))
        self.assertEqual(summary['cleanup_errors'], [])


if __name__ == '__main__':
    unittest.main()
