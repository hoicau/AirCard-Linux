# Current checkpoint: 2026-09-22

The owner requested an immediate local commit and stop before GUI development.
This checkpoint contains stage 4 functionality; it does not claim all original hardware
acceptance criteria or protected Wallet/passcode application are complete.

## Verified

- Existing-pair USB service access and native TLS ATC on one iOS 27.0 phone.
- Exact ReadyForSync, strict Book manifest, FileComplete and SyncFinished using the
  explicitly authorized public Grappa token experiment. No Apple binaries are used.
- A 1929-byte synthetic EPUB reached Books with exact byte equality. Original content
  was preserved and the complete 56-entry, 762433-byte Books tree restored.
- Core center-cropped 1536x969 PNG/PDF card resources, passthm en/ru/uk/ja/all variants,
  bold names, TelephonyUI 8/9/10 mapping, normalized PNG previews and ZIP export.
- Bounded image/archive/plist/path handling, symlink and conflicting theme rejection.
- Complete bounded Books snapshots and recovery through AfcAccess, with durable private
  local journals, device binding, integrity checks and concurrent-content protection.
- Local fmt, clippy with warnings denied, 55 tests and release build passed.

The real-device evidence is single-asset-4 (local-only report).
It came from the experiment harness. The new formal CLI commands compile and unit tests
pass; the formal CLI flow has not received another complete hardware run.

## Commands

```sh
# Offline dry-run: parse, validate, crop/map, print resource summary; no device connection.
cargo run -p cli -- prepare-card artwork.png
cargo run -p cli -- prepare-theme theme.passthm --language all --telephony 10
# Optional local exports; existing files are never overwritten.
cargo run -p cli -- prepare-card artwork.png --preview preview.png --output wallet.zip
cargo run -p cli -- prepare-theme theme.passthm --language ru --bold --preview keys.zip --output resources.zip
# Full Books snapshot (the older `snapshot` command remains diagnostic metadata only).
cargo run -p cli -- books-snapshot backup.json --transport usb
cargo run -p cli -- books-restore backup.json                 # offline validation/dry-run
cargo run -p cli -- books-restore backup.json --transport usb --apply
# Controlled transaction: always restore and verify, retaining the journal if recovery fails.
cargo run -p cli -- books-test --journal recovery.json         # offline dry-run
cargo run -p cli -- books-test --journal recovery.json --transport usb --grappa-token token.bin --apply
cargo run -p cli -- atc-ready --transport usb --grappa-token token.bin --apply
```

Token input is exactly 84 bytes in a private regular file (chmod 600). No token is bundled,
auto-fetched, generated from private libraries or logged. The observed supported Grappa
profile is version 1 / device type 0 / protocol version 1; other profiles fail closed.
Caller-owned token files and deliberately exported backups/previews remain the caller's
responsibility. Delete them after use. Temporary transaction journals are deleted only
following verified restoration; keep failed journals and recover on the same device.

Snapshots cover the complete normal AFC Books tree within 1024 entries, 16 MiB per file,
64 MiB total. Larger collections fail before staging. They are not whole-iPhone backups.
JSON backups contain private book data: do not commit or share them. Checksums detect
accidental changes, not malicious tampering. Restore refuses unknown concurrent content
changes and does not erase new user books. Keep Books inactive during transactions.
Hard termination may require explicit recovery from the retained journal after reconnection.

## Outstanding / deliberately stopped

Wi-Fi hardware acceptance and Books application restart verification were skipped at the
owner's request. No GUI or binary packaging was started. Hosted CI has not run; no push
was requested. Protected Wallet/TelephonyUI device writes are unavailable: upstream
path-escape/symlink mechanisms conflict with this project's security boundary. Those
resources can be prepared, previewed and exported offline. The installation guide covers
Debian/Ubuntu, Arch/Manjaro and Fedora; local builds were tested on Manjaro only.
