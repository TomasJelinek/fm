#!/usr/bin/env python3
"""Experiment only: sample memory, commit/swap and disk every few seconds; summarize.

sample FILE  -- runs until killed, appending CSV rows
report FILE  -- prints minimum available memory, peak commit/swap use, minimum disk free
"""
import csv
import ctypes
import os
import shutil
import sys
import time

GB = 1024 ** 3


def memory():
    if os.name == 'nt':
        class Status(ctypes.Structure):
            _fields_ = [('length', ctypes.c_ulong), ('load', ctypes.c_ulong),
                        ('total', ctypes.c_ulonglong), ('available', ctypes.c_ulonglong),
                        ('commit_limit', ctypes.c_ulonglong), ('commit_available', ctypes.c_ulonglong),
                        ('virtual_total', ctypes.c_ulonglong), ('virtual_available', ctypes.c_ulonglong),
                        ('extended', ctypes.c_ulonglong)]
        status = Status()
        status.length = ctypes.sizeof(status)
        ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status))
        return (status.total, status.available, status.commit_limit - status.commit_available,
                status.commit_limit)
    values = {}
    with open('/proc/meminfo') as stream:
        for line in stream:
            key, value = line.split(':', 1)
            values[key] = int(value.split()[0]) * 1024
    return (values['MemTotal'], values['MemAvailable'], values['SwapTotal'] - values['SwapFree'],
            values['SwapTotal'])


def sample(path):
    workspace = os.environ.get('GITHUB_WORKSPACE', '.')
    temporary = os.environ.get('TEMP') or os.environ.get('TMPDIR') or workspace
    with open(path, 'a', newline='') as stream:
        writer = csv.writer(stream)
        while True:
            total, available, committed, limit = memory()
            writer.writerow([time.strftime('%H:%M:%S', time.gmtime()), total, available, committed, limit,
                             shutil.disk_usage(workspace).free, shutil.disk_usage(temporary).free])
            stream.flush()
            time.sleep(5)


def report(path):
    with open(path, newline='') as stream:
        rows = [row for row in csv.reader(stream) if len(row) == 7]
    if not rows:
        print('No resource samples')
        return
    values = [[row[0]] + [int(value) for value in row[1:]] for row in rows]
    low = min(values, key=lambda row: row[2])
    peak = max(values, key=lambda row: row[3])
    disk = min(values, key=lambda row: row[5])
    kind = 'commit' if os.name == 'nt' else 'swap used'
    print(f'Samples: {len(values)} from {values[0][0]} to {values[-1][0]} UTC')
    print(f'Physical memory: {low[1] / GB:.1f} GB; least available {low[2] / GB:.2f} GB at {low[0]}')
    print(f'Peak {kind}: {peak[3] / GB:.2f} GB of {peak[4] / GB:.1f} GB at {peak[0]}')
    print(f'Least free disk: workspace {disk[5] / GB:.1f} GB at {disk[0]}; '
          f'temporary {min(row[6] for row in values) / GB:.1f} GB')


if __name__ == '__main__':
    {'sample': sample, 'report': report}[sys.argv[1]](sys.argv[2])
