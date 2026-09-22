#!/usr/bin/env python3
"""Run bounded hardware checks and retain sanitized JSON events only (no raw device logs)."""
import argparse
import datetime
import json
import os
from pathlib import Path
import subprocess
import signal
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', default='target/release/aircard')
parser.add_argument('--output', required=True)
parser.add_argument('--apply', action='store_true', help='Include the controlled AFC scratch write/read/delete')
args = parser.parse_args()
binary = str(Path(args.binary).resolve())
commands = [
    ['devices'],
    ['probe', '--transport', 'usb'],
    ['syslog', '--transport', 'usb', '--duration', '3'],
    ['scan', '--transport', 'usb', '--duration', '3'],
    ['snapshot', '--transport', 'usb'],
    ['atc-smoke', '--transport', 'usb', '--timeout', '3'],
    ['probe', '--transport', 'wifi'],
]
if args.apply:
    commands.append(['afc-self-test', '--transport', 'usb', '--apply'])
# Reserve before touching the device; never overwrite an existing report or symlink.
fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
report = {'schema_version': 1, 'recorded_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
          'scope': 'one authorized paired iPhone; no ATC writes; identifiers and raw logs omitted', 'checks': []}
try:
    for command in commands:
        process = subprocess.Popen([binary, *command], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, start_new_session=True)
        try:
            stdout, _ = process.communicate(timeout=30)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            os.killpg(process.pid, signal.SIGTERM)
            try:
                stdout, _ = process.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                stdout, _ = process.communicate()
        events = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        if any(event.get('cleanup_ok') is True for event in events):
            for event in events:
                if 'scratch_path' in event:
                    event['scratch_path'] = '[ephemeral test directory; cleanup verified]'
        report['checks'].append({'command': ['aircard', *command], 'exit_code': process.returncode, 'events': events})
        print(json.dumps({'command': command[0], 'transport': 'wifi' if 'wifi' in command else 'usb_or_all', 'exit_code': process.returncode}), flush=True)
finally:
    with os.fdopen(fd, 'w') as output:
        json.dump(report, output, indent=2)
        output.write('\n')

sys.exit(0 if all(check["exit_code"] == 0 for check in report["checks"]) else 1)
