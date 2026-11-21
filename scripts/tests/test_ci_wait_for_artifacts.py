"""CI's check and release jobs wait only for the plugin artifacts that their later steps use."""
import contextlib
import importlib.util
import io
from pathlib import Path
import unittest
import urllib.error

SCRIPT = Path(__file__).resolve().parents[1] / 'ci-wait-for-artifacts.py'
spec = importlib.util.spec_from_file_location('ci_wait_for_artifacts', SCRIPT)
waiter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(waiter)

RUN = 'https://api.example/repos/owner/repo/actions/runs/7'
PRODUCERS = ['viewers', 'filesystem-libraries']
LABEL = 'windows-latest'


def leg(name, status='completed', conclusion='success', labels=(LABEL,)):
    return {'name': name, 'status': status, 'conclusion': conclusion, 'labels': list(labels)}


class Api:
    """Each poll reads its artifacts, then its jobs, from the next (artifacts, jobs) state;
    the last state repeats. An exception state fails that poll's first request."""

    def __init__(self, *states):
        self.states = list(states)
        self.current = None
        self.urls = []

    def __call__(self, url):
        self.urls.append(url)
        if url.startswith(f'{RUN}/artifacts'):
            self.current = self.states.pop(0) if len(self.states) > 1 else self.states[0]
            if isinstance(self.current, Exception):
                raise self.current
            return {'artifacts': [{'name': name, 'expired': name.endswith('-expired')}
                                  for name in self.current[0]]}
        self.assertJobs(url)
        return {'jobs': self.current[1]}

    @staticmethod
    def assertJobs(url):
        if url != f'{RUN}/jobs?filter=latest&per_page=100&page=1':
            raise AssertionError(f'unexpected request {url}')


class Clock:
    def __init__(self):
        self.now = 0.0
        self.sleeps = []

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.sleeps.append(seconds)
        self.now += seconds


class WaitTests(unittest.TestCase):
    def run_wait(self, api, names=('viewer-Windows-basic', 'filesystem-Windows-native'), timeout=600):
        clock, lines = Clock(), []
        code = waiter.wait(list(names), PRODUCERS, LABEL, api, RUN, timeout, 20,
                           clock=clock, sleep=clock.sleep, log=lines.append)
        return code, clock, lines

    def test_present_artifacts_end_the_wait_without_sleeping_or_listing_jobs(self):
        api = Api((['viewer-Windows-basic', 'filesystem-Windows-native', 'unrelated'], []))
        code, clock, lines = self.run_wait(api)
        self.assertEqual(code, 0)
        self.assertEqual(clock.sleeps, [])
        self.assertTrue(all('/artifacts' in url for url in api.urls))
        self.assertIn('All 2 artifacts are available after 0 s', lines)

    def test_waits_while_producers_run_and_reports_each_change_once(self):
        running = [leg('viewers (windows-latest, basic)', 'in_progress', None)]
        api = Api(([], running), ([], running), (['viewer-Windows-basic'], running),
                  (['viewer-Windows-basic', 'filesystem-Windows-native'], running))
        code, clock, lines = self.run_wait(api)
        self.assertEqual(code, 0)
        self.assertEqual(clock.sleeps, [20, 20, 20])
        self.assertEqual([line for line in lines if line.startswith('Waiting')], [
            'Waiting for 2 artifact(s): filesystem-Windows-native, viewer-Windows-basic',
            'Waiting for 1 artifact(s): filesystem-Windows-native'])

    def test_a_failed_producer_of_this_label_stops_the_wait(self):
        for job in (leg('viewers (windows-latest, source)', conclusion='failure'),
                    leg('filesystem-libraries (windows-latest, windows, native)', conclusion='cancelled'),
                    leg('viewers (windows-latest, basic)', conclusion='skipped', labels=()),
                    leg('viewers', conclusion='skipped', labels=())):
            with self.subTest(job=job['name'], conclusion=job['conclusion']):
                code, clock, lines = self.run_wait(Api(([], [job])))
                self.assertEqual(code, 1)
                self.assertEqual(clock.sleeps, [])
                self.assertIn(job['name'], lines[-1])
                self.assertTrue(lines[-1].startswith('::error::'))

    def test_failures_of_other_labels_and_jobs_do_not_stop_the_wait(self):
        jobs = [leg('viewers (ubuntu-24.04, basic)', conclusion='failure', labels=('ubuntu-24.04',)),
                leg('thumbnail-image (windows-latest)', conclusion='failure'),
                leg('viewers-extra (windows-latest)', conclusion='failure')]
        api = Api(([], jobs), (['viewer-Windows-basic', 'filesystem-Windows-native'], jobs))
        code, clock, lines = self.run_wait(api)
        self.assertEqual(code, 0)
        self.assertEqual(clock.sleeps, [20])

    def test_expired_artifacts_are_missing(self):
        code, clock, lines = self.run_wait(Api((['viewer-Windows-basic-expired'], [])),
                                           names=['viewer-Windows-basic-expired'], timeout=50)
        self.assertEqual(code, 1)
        self.assertEqual(clock.sleeps, [20, 20, 10])
        self.assertEqual(lines[-1], '::error::Timed out after 50 s waiting for: viewer-Windows-basic-expired')

    def test_transient_failures_are_retried_and_others_stop_the_wait(self):
        def http(code, headers=None):
            error = urllib.error.HTTPError(RUN, code, 'error', headers or {}, io.BytesIO())
            self.addCleanup(error.close)
            return error
        present = (['viewer-Windows-basic', 'filesystem-Windows-native'], [])
        for error in (http(500), http(502), http(429), http(403, {'x-ratelimit-remaining': '0'}),
                      http(403, {'retry-after': '60'}), urllib.error.URLError('reset'), TimeoutError()):
            with self.subTest(error=repr(error)):
                code, clock, lines = self.run_wait(Api(error, present))
                self.assertEqual(code, 0)
                self.assertEqual(clock.sleeps, [20])
                self.assertTrue(lines[0].startswith('::warning::'))
        for error in (http(401), http(403), http(404)):
            with self.subTest(error=repr(error)):
                code, clock, lines = self.run_wait(Api(error, present))
                self.assertEqual(code, 1)
                self.assertEqual(clock.sleeps, [])
                self.assertTrue(lines[-1].startswith('::error::Artifact query failed'))

    def test_every_page_is_read(self):
        pages = {1: [f'other-{index}' for index in range(100)], 2: ['viewer-Windows-basic']}
        urls = []
        def fetch(url):
            urls.append(url)
            page = int(url.rsplit('page=', 1)[1])
            return {'artifacts': [{'name': name, 'expired': False} for name in pages[page]]}
        code, clock, lines = self.run_wait(fetch, names=['viewer-Windows-basic'])
        self.assertEqual(code, 0)
        self.assertEqual(urls, [f'{RUN}/artifacts?per_page=100&page=1', f'{RUN}/artifacts?per_page=100&page=2'])

    def test_main_reads_the_run_from_the_environment(self):
        urls = []
        def fetch(url):
            urls.append(url)
            return {'artifacts': [{'name': 'a', 'expired': False}]}
        environment = {'GITHUB_API_URL': 'https://api.example/', 'GITHUB_REPOSITORY': 'owner/repo',
                       'GITHUB_RUN_ID': '7'}
        with contextlib.redirect_stdout(io.StringIO()):
            code = waiter.main(['--producer', 'viewers', '--label', LABEL, 'a'], environment, fetch)
        self.assertEqual(code, 0)
        self.assertEqual(urls, [f'{RUN}/artifacts?per_page=100&page=1'])
        for missing in ('GITHUB_REPOSITORY', 'GITHUB_RUN_ID'):
            with self.subTest(missing=missing), contextlib.redirect_stderr(io.StringIO()):
                partial = {key: value for key, value in environment.items() if key != missing}
                with self.assertRaises(SystemExit) as raised:
                    waiter.main(['--producer', 'viewers', '--label', LABEL, 'a'], partial, fetch)
                self.assertEqual(raised.exception.code, 2)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
            waiter.main(['--producer', 'viewers', '--label', LABEL, 'a'], environment)
        self.assertEqual(raised.exception.code, 2, 'a token is required')


if __name__ == '__main__':
    unittest.main()
