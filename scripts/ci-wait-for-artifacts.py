#!/usr/bin/env python3
"""Wait until this workflow run has uploaded the named artifacts.

CI's check and release jobs build before the plugin jobs that package the viewers and
filesystems have finished; they wait here only before the steps that install those
packages. Waiting ends with an error as soon as a producer job of the same runner label
has finished without success (its artifacts would never come), or at the deadline.

Uses GITHUB_API_URL, GITHUB_REPOSITORY and GITHUB_RUN_ID, and GH_TOKEN or GITHUB_TOKEN
with `actions: read`.
"""
import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request

PAGE_SIZE = 100


def request_json(url, token):
    request = urllib.request.Request(url, headers={
        'Accept': 'application/vnd.github+json',
        'Authorization': f'Bearer {token}',
        'User-Agent': 'midnight-explorer-ci',
        'X-GitHub-Api-Version': '2022-11-28',
    })
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def collect(fetch, url, key):
    """Every item of a paginated list endpoint."""
    items, page = [], 1
    while True:
        separator = '&' if '?' in url else '?'
        batch = fetch(f'{url}{separator}per_page={PAGE_SIZE}&page={page}')[key]
        items.extend(batch)
        if len(batch) < PAGE_SIZE:
            return items
        page += 1


def is_producer(job, producers, label):
    """A job of a producer: its matrix leg on this label, a leg without labels (one that a
    failed dependency skipped) or the unexpanded job with the bare name."""
    for producer in producers:
        if job['name'] == producer:
            return True
        if job['name'].startswith(producer + ' (') and (not job.get('labels') or label in job['labels']):
            return True
    return False


def retryable(error):
    """Rate limits, server errors and connection failures pass; a missing permission,
    token or run does not."""
    if isinstance(error, urllib.error.HTTPError):
        if error.code == 403:
            headers = error.headers or {}
            return headers.get('x-ratelimit-remaining') == '0' or 'retry-after' in headers
        return error.code == 429 or error.code >= 500
    return True


def failed_producers(jobs, producers, label):
    return sorted(job['name'] for job in jobs
                  if is_producer(job, producers, label) and job.get('status') == 'completed'
                  and job.get('conclusion') != 'success')


def wait(names, producers, label, fetch, run_url, timeout, interval,
         clock=time.monotonic, sleep=time.sleep, log=print):
    """Return 0 once every name is an unexpired artifact of the run, otherwise 1."""
    expected = set(names)
    started = clock()
    deadline = started + timeout
    reported = None
    while True:
        try:
            artifacts = collect(fetch, f'{run_url}/artifacts', 'artifacts')
            present = {artifact['name'] for artifact in artifacts if not artifact.get('expired')}
            missing = sorted(expected - present)
            if not missing:
                log(f'All {len(expected)} artifacts are available after {clock() - started:.0f} s')
                return 0
            failed = failed_producers(collect(fetch, f'{run_url}/jobs?filter=latest', 'jobs'),
                                      producers, label)
            if failed:
                log(f'::error::Producer job(s) finished without success: {", ".join(failed)}; '
                    f'missing artifacts: {", ".join(missing)}')
                return 1
            if missing != reported:
                log(f'Waiting for {len(missing)} artifact(s): {", ".join(missing)}')
                reported = missing
        except (urllib.error.URLError, TimeoutError, OSError, ValueError, KeyError) as error:
            if not retryable(error):
                log(f'::error::Artifact query failed: {error}')
                return 1
            log(f'::warning::Artifact query failed, retrying: {error}')
        remaining = deadline - clock()
        if remaining <= 0:
            log(f'::error::Timed out after {timeout:.0f} s waiting for: '
                f'{", ".join(reported or sorted(expected))}')
            return 1
        sleep(min(interval, remaining))


def main(argv=None, environment=None, fetch=None):
    environment = os.environ if environment is None else environment
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n', 1)[0])
    parser.add_argument('names', nargs='+', metavar='ARTIFACT')
    parser.add_argument('--producer', action='append', required=True,
                        help='Job id whose matrix legs upload the artifacts (repeatable)')
    parser.add_argument('--label', required=True,
                        help='Runner label of the producer legs to watch, such as windows-latest')
    parser.add_argument('--timeout-minutes', type=float, default=60)
    parser.add_argument('--interval-seconds', type=float, default=20)
    args = parser.parse_args(argv)
    try:
        api = environment.get('GITHUB_API_URL', 'https://api.github.com').rstrip('/')
        run_url = f'{api}/repos/{environment["GITHUB_REPOSITORY"]}/actions/runs/{environment["GITHUB_RUN_ID"]}'
    except KeyError as error:
        parser.error(f'{error.args[0]} is not set; run this inside a GitHub Actions job')
    if fetch is None:
        token = environment.get('GH_TOKEN') or environment.get('GITHUB_TOKEN')
        if not token:
            parser.error('GH_TOKEN or GITHUB_TOKEN with actions: read is required')
        fetch = lambda url: request_json(url, token)  # noqa: E731
    return wait(args.names, args.producer, args.label, fetch, run_url,
                args.timeout_minutes * 60, args.interval_seconds)


if __name__ == '__main__':
    sys.exit(main())
