#!/usr/bin/env python3
"""Bounded checks for the current Wallet CLI; writes require explicit --apply.

Keep Wallet and Books closed during writes. Failed tests retain all private backups
and journals in --work-dir. Never delete those files before recovery completes.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import zlib


def redact(value):
    if isinstance(value, dict):
        return {key: "[redacted]" if key in {"udid", "hash", "device_fingerprint", "path", "card_hash"} else redact(item)
                for key, item in value.items()}
    if isinstance(value, list):
        return [redact(item) for item in value]
    return value


def artwork(path):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    width, height = 480, 320
    pixels = b"".join(b"\0" + bytes([30, 100 + y // 3, 150]) * width for y in range(height))
    data = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))
    with open(path, "xb") as output:
        output.write(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/aircard")
    parser.add_argument("--output", required=True)
    parser.add_argument("--work-dir", required=True)
    parser.add_argument("--apply", action="store_true", help="Run Apply/Restore and the automatically restored card test")
    parser.add_argument("--card-hash-file", type=Path, help="Private file containing the explicitly selected card hash")
    parser.add_argument("--interrupt-test", action="store_true", help="Also kill a test after verified artwork installation, then recover it")
    parser.add_argument("--skip-scan", action="store_true", help="Use when live detection was already tested before closing Wallet")
    args = parser.parse_args()
    if args.apply and not args.card_hash_file:
        parser.error("--apply requires --card-hash-file for the user's selected card")
    if args.interrupt_test and not args.apply:
        parser.error("--interrupt-test requires --apply")
    binary = str(Path(args.binary).resolve())
    work = Path(args.work_dir).resolve()
    work.mkdir(mode=0o700, parents=True, exist_ok=True)
    if work.is_symlink() or work.stat().st_mode & 0o077:
        parser.error("--work-dir must be a private directory with mode 0700")
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    report = {"recorded_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "scope": "authorized USB Wallet checks; identifiers omitted", "checks": []}

    def run(label, command, terminal=None, interrupt=False):
        print(label + ": started", flush=True)
        process = subprocess.Popen([binary, *map(str, command)], stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, text=True, start_new_session=True)
        events = []
        interrupted = False
        # The CLI watchdog bounds native calls; the outer deadline also protects this harness.
        import threading
        def stop():
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        timer = threading.Timer(900, stop)
        timer.start()
        try:
            for line in process.stdout:
                event = json.loads(line)
                events.append(redact(event))
                if event.get("event") == "stage":
                    print(label + ": " + event["stage"], flush=True)
                if interrupt and event.get("event") == "card_artwork_verified":
                    os.killpg(process.pid, signal.SIGKILL)
                    interrupted = True
            code = process.wait(timeout=30)
        finally:
            timer.cancel()
            if process.poll() is None:
                stop()
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
        passed = interrupted if interrupt else code == 0 and (terminal is None or any(
            e.get("event") == terminal and e.get("ok", True) for e in events))
        report["checks"].append(dict(check=label, exit_code=code, passed=passed, events=events))
        print(label + (": passed" if passed else ": FAILED; retain the work directory"), flush=True)
        if not passed:
            raise RuntimeError(label + " failed; inspect the private report and recover any remaining journal")
        return events

    try:
        devices = run("devices", ["devices"], "devices_complete")
        if sum(e.get("event") == "device" and e.get("transport") == "usb" for e in devices) != 1:
            raise RuntimeError("Exactly one USB iPhone is required")
        run("probe", ["probe", "--transport", "usb"], "probe_complete")
        if not args.skip_scan:
            run("scan", ["scan", "--transport", "usb", "--duration", "5"], "syslog_complete")
        image, preview, resources = (work / name for name in ["artwork.png", "preview.png", "resources.zip"])
        artwork(image)
        run("prepare-card", ["prepare-card", image, "--preview", preview, "--output", resources])
        assert preview.is_file() and resources.is_file()
        token = work / "token.bin"
        run("setup-token", ["setup-token", "--output", token], "token_ready")
        assert token.stat().st_size == 84 and token.stat().st_mode & 0o077 == 0
        run("reuse-token", ["setup-token", "--output", token], "token_ready")
        if args.apply:
            card_hash = args.card_hash_file.read_text().strip()
            common = ["--transport", "usb", "--timeout", "25", "--grappa-token", token]
            journal = work / "test-recovery"
            run("card-test", ["card-test", "--card-hash", card_hash, "--journal", journal,
                              "--hold-seconds", "0", "--apply", *common], "card_test_complete")
            assert not journal.exists()
            backup, journal = work / "backup.json", work / "apply-recovery"
            run("card-apply", ["card-apply", image, "--card-hash", card_hash, "--backup", backup,
                               "--journal", journal, "--apply", *common], "card_operation_complete")
            assert backup.is_file() and not journal.exists()
            journal = work / "restore-recovery"
            run("validate-backup", ["card-restore", backup, "--journal", journal, *common], "dry_run")
            run("card-restore", ["card-restore", backup, "--journal", journal, "--apply", *common], "card_operation_complete")
            assert not journal.exists()
            if args.interrupt_test:
                journal = work / "interrupted-recovery"
                try:
                    run("interrupted-test", ["card-test", "--card-hash", card_hash, "--journal", journal,
                                              "--hold-seconds", "30", "--apply", *common], interrupt=True)
                finally:
                    if journal.exists():
                        run("validate-recovery", ["card-recover", journal, *common], "dry_run")
                        run("card-recover", ["card-recover", journal, "--apply", *common], "card_recovery_complete")
                assert not journal.exists()
    finally:
        with os.fdopen(fd, "w") as output:
            json.dump(report, output, indent=2)
            output.write("\n")


if __name__ == "__main__":
    main()
