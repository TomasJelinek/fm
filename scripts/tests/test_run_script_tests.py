"""The script test runner runs each module as `unittest discover` would, several at once."""
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'scripts/run-script-tests.py'
# A guard against a hang, not a duration: each fixture module is a Python process.
HANG_GUARD = 120

PASSING = '''
import unittest
class Passing(unittest.TestCase):
    def test_one(self):
        pass
    @unittest.skip('fixture')
    def test_skipped(self):
        pass
'''
FAILING = '''
import unittest
class Failing(unittest.TestCase):
    def test_fails(self):
        self.fail('fixture failure')
    def test_passes(self):
        pass
'''
# Each module waits for the other's marker, so both pass only when they run at once.
RENDEZVOUS = '''
import os, time, unittest
from pathlib import Path
class Rendezvous(unittest.TestCase):
    def test_meets_the_other_module(self):
        folder = Path(__file__).parent
        (folder / '{me}.ready').write_text('')
        deadline = time.monotonic() + 30
        while not (folder / '{other}.ready').exists():
            self.assertLess(time.monotonic(), deadline, 'the other module never ran beside this one')
            time.sleep(0.01)
'''


class ScriptTestRunnerTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / '_build/tmp'
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix='script-test-runner-', dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.folder = Path(self.owner.name)

    def module(self, name, source):
        (self.folder / name).write_text(textwrap.dedent(source), encoding='utf-8')

    def run_runner(self, *arguments):
        return subprocess.run([sys.executable, '-B', str(SCRIPT), '-s', str(self.folder), *arguments],
                              capture_output=True, text=True, encoding='utf-8', errors='replace',
                              timeout=HANG_GUARD)

    def test_every_module_runs_whole_and_the_counts_are_unittests_own(self):
        self.module('test_a.py', PASSING)
        self.module('test_b.py', PASSING)
        # Helpers that discovery does not take are not run either.
        self.module('helper.py', FAILING)
        result = self.run_runner('-j', '2')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in ('test_a', 'test_b'):
            self.assertIn(f'===== {name}: ok, 2 tests (1 skipped) in ', result.stdout)
        self.assertNotIn('helper', result.stdout)
        self.assertIn('Ran 4 tests (2 skipped) of 2 modules in ', result.stdout)
        # Each module's own unittest report, which validation's counter sums, stays whole.
        self.assertEqual(re.findall(r'^Ran (\d+) tests? in .*\n\nOK \(skipped=1\)$', result.stdout,
                                    re.MULTILINE), ['2', '2'])
        self.assertTrue(result.stdout.rstrip().endswith('OK'))

    def test_a_failing_module_fails_the_run_after_every_module_ran(self):
        self.module('test_bad.py', FAILING)
        self.module('test_good.py', PASSING)
        result = self.run_runner('-j', '1')
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn('===== test_bad: FAILED, 2 tests (0 skipped) in ', result.stdout)
        self.assertIn('fixture failure', result.stdout)
        self.assertIn('===== test_good: ok, 2 tests (1 skipped) in ', result.stdout)
        self.assertIn('FAILED modules: test_bad', result.stdout)

    def test_a_module_without_tests_fails(self):
        self.module('test_empty.py', 'import unittest\n')
        result = self.run_runner()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn('FAILED modules: test_empty', result.stdout)

    def test_modules_run_side_by_side(self):
        self.module('test_left.py', RENDEZVOUS.format(me='left', other='right'))
        self.module('test_right.py', RENDEZVOUS.format(me='right', other='left'))
        result = self.run_runner('-j', '2')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('Ran 2 tests (0 skipped) of 2 modules in ', result.stdout)

    def test_a_test_package_is_refused_rather_than_left_out(self):
        self.module('test_a.py', PASSING)
        (self.folder / 'nested').mkdir()
        (self.folder / 'nested/__init__.py').write_text('', encoding='utf-8')
        result = self.run_runner()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('test packages are not supported', result.stderr)

    def test_jobs_must_be_positive(self):
        self.module('test_a.py', PASSING)
        result = self.run_runner('-j', '0')
        self.assertEqual(result.returncode, 2)
        self.assertIn('--jobs must be at least 1', result.stderr)


if __name__ == '__main__':
    unittest.main()
