# AirCard-Linux

Native Linux **Wallet card artwork customization**, with a Rust CLI and egui desktop app.
Prepare an image, select a card on your paired iPhone, apply the background, and restore
its original artwork from a private local backup.

USB card application, exact readback, independent backup/restore and recovery after a
forced process interruption have been verified on one iOS 27.0 iPhone. Its owner confirmed
both the changed Wallet artwork and the restored original. See [verification status](docs/STATUS.md)
for the tested boundary; other devices and Wi-Fi remain unverified.

The product scope is card artwork only. Device discovery, Wallet scanning, image previews,
backups and recovery support that workflow. AirTraffic/Books synchronization is an internal
transport, with the original Books state restored after each operation. No Apple DLLs or
frameworks are loaded or distributed. Original MIT licensing and provenance are retained.

## Install and launch

Follow the [Debian/Ubuntu, Arch Linux and Fedora dependency guide](docs/INSTALL.md).
Build with Rust stable (minimum 1.88) and the checked-in lockfile:

```sh
cargo build --locked --workspace --release
./target/release/aircard-gui
```

Keep `aircard`, `aircard-gui` and the release's `lib/` directory together. Run as your normal user. The CLI also works
without a desktop session. Native `.tar.gz` binaries include dependency/build information
and checksums. CI archives bundle matching libimobiledevice, libusbmuxd and libplist
libraries, including their native glue dependency when required. This avoids device-library
SONAME differences between Ubuntu and Arch/Manjaro. Your system still supplies a compatible
glibc, TLS/graphics libraries and the usbmuxd service; the archive is not fully static.

## Apply a card background

1. Connect and unlock your own already paired iPhone. Keep Books closed.
2. In **Apply Artwork**, select your iPhone. AirCard automatically chooses a paired iPhone
   when only one is available, preferring USB. If multiple iPhones are connected, choose yours.
   Card detection starts automatically. Wait for **Waiting for Wallet**, then open Wallet
   on your iPhone and tap the intended card. A single detected identifier is selected automatically;
   multiple results require a choice. Check the target and close Wallet.
   Detection stops after a match or 60 seconds.
3. Browse for PNG/JPEG/WebP under **Select artwork**. The centered 1536 × 969 preview updates
   automatically. You can also enter a path and press Enter.
4. The built-in sync token is saved automatically after a card is selected and kept privately
   for future launches. Backup and recovery paths are generated automatically.
   **Advanced Options** contains token setup, backup locations, restore/recovery, manual
   USB/Wi-Fi selection and artwork exports. Token setup works offline, including the first launch. See [token setup](docs/SYNC-TOKEN.md).
5. Click **Apply Artwork…**, review the target card and device, then confirm. Reopen Wallet
   after completion.

Detection reads Wallet activity from iPhone logs; it does not enumerate every stored card.
If no identifier appears, return to Wallet's card list, click **Scan**, wait
for the prompt and reopen the intended card. If no logs arrive, unlock/reconnect over USB
and refresh devices. Some cards or iOS versions may hide identifiers. Linux cannot open
Wallet or select a card for you. A detected identifier can also come from background Wallet
activity, so verify the target before applying; retry with only the intended card open if unsure.

**Advanced Options → Check device** verifies trust and file access without writing to the phone. The GUI checks
that the adjacent CLI version matches before starting a task. **Help → Connection diagnostics**
provides service checks and installation guidance. A transient timeout or disconnect while
opening a read-only trust, log or file-access session gets one retry on the same device and
route. An explicit Wi-Fi selection never switches to USB. Artwork writes are never retried
automatically; failures retain actionable stage and recovery guidance.

AirCard replaces existing background resources only: `cardBackgroundCombined@3x.png`,
`cardBackgroundCombined@2x.png` and `cardBackgroundCombined.pdf`. It invalidates the
selected card's generated display caches. Card account details and `pass.json` are outside
the writable target set. The GUI rejects an image changed after preview approval.

## Restore and recover

The GUI saves operations under `$XDG_DATA_HOME/aircard/operations`, or
`~/.local/share/aircard/operations` by default. Apply backups are grouped by iPhone and
card, with a private, unique subfolder for each operation. Device and card directory names
use SHA-256 digests; raw identifiers are not used as path names.
Backups keep the previous artwork; recovery folders hold the progress and originals needed
after an interruption. Recovery folders disappear after successful cleanup; backups remain.

In **Advanced Options → Restore or recover**, **Restore** selects the latest backup for the current iPhone
and selected card. Switching cards refreshes this selection. Browse to choose an older
or imported backup, including backups from the previous device-only directory layout.
Existing files stay in their original locations. A new recovery path is generated automatically.
Close Wallet and Books, review the restore plan and confirm. Reopen Wallet when complete.
Keep the backup until you no longer need to restore that artwork.

**Recover** resumes an interrupted operation from its existing recovery directory, selected
automatically across all cards on the selected iPhone, including after restarting the GUI.
Restore journals and journals from the previous layout are also discovered. Keep
that directory when a device disconnects or recovery fails. Reconnect the original device,
unlock it, close Wallet/Books and run Recover before another operation. Recovery rolls back
an unfinished apply. If application and durable backup already completed, it finishes
cleanup and preserves the committed result. Use Restore to undo a completed application.

Cancellation requests restoration and allows bounded cleanup. A watchdog may terminate a
stalled native call; the journal then remains available for recovery. Do not manually delete
an incomplete recovery directory. Concurrent unrelated artwork changes stop recovery with
a conflict rather than being silently discarded. Display caches are derived data and can be
regenerated by Wallet.

## CLI

Device operations accept `--udid <UDID>` and `--transport usb|wifi`. Ambiguous selection
fails and Wi-Fi never falls back to USB. Device identifiers and card hashes are redacted
unless explicitly requested. Commands emit newline-delimited JSON with stage and error details.

```sh
./target/release/aircard devices
./target/release/aircard scan --transport usb --duration 30 --show-hashes
./target/release/aircard prepare-card artwork.png --preview preview.png

mkdir -p .local
./target/release/aircard setup-token --output .local/token.bin
# Validate the plan offline first; add --apply to perform it.
./target/release/aircard card-apply artwork.png --card-hash '<HASH>' \
  --backup .local/card-original.json --journal .local/card-transaction \
  --grappa-token .local/token.bin --transport usb --timeout 25

./target/release/aircard card-restore .local/card-original.json \
  --journal .local/restore-transaction --grappa-token .local/token.bin \
  --transport usb --timeout 25 --apply

./target/release/aircard card-recover .local/card-transaction \
  --grappa-token .local/token.bin --transport usb --timeout 25 --apply
```

`prepare-card` can additionally export a resource ZIP with `--output wallet.zip`.
`probe` checks the selected paired device's AFC connection without writing.
All card-changing commands default to dry-run and require `--apply`.

## Compatibility and limits

| Environment | Verification |
| --- | --- |
| Linux x86_64, USB, iOS 27.0 | Card application, owner visual check, restore and interrupted recovery verified |
| Paired Wi-Fi | Explicit route supported; hardware acceptance pending |
| Other iOS versions/cards | Unverified; resource layout and private services can differ |
| GitHub Actions `ubuntu-latest` | Latest stable Rust, bundled device libraries, relocated startup checks on Ubuntu and Arch; no iPhone required by CI |
| Debian, Arch derivatives, Fedora | Dependency guides supplied; build locally for matching libraries |

The internal Books safeguard is bounded to 1024 entries, 16 MiB per file and 64 MiB total.
Larger or concurrently changing libraries are rejected before card changes. Sync manifests
are bounded to 128 entries including preserved books. Original background/cache files are
limited to 16 MiB each. Pairing and service TLS use libimobiledevice and remain required.
A signed Wallet update or iOS update may replace custom artwork or change compatibility.

## Development and binary packaging

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --release
./scripts/package-native.sh
# On Debian/Ubuntu with deb-src enabled, create the same bundled layout as CI:
./scripts/package-native.sh --bundle-native bundled
```

Branch pushes and pull requests run Linux CI. Tag pushes do not rebuild. Publishing a
GitHub Release (including a prerelease) reuses the successful branch CI artifact for that
exact commit. It checks the archive checksum, embedded source commit, clean-tree status
and version before uploading. The tag must match the CLI/GUI version, for example `v0.2.0`.
Wait for branch CI to pass before publishing; if the artifact expired, rerun that branch
build. Rerun the release upload after its artifact becomes available. Release uploads never
start another Linux build.

Core data/image logic is platform-independent. `device` owns the Linux backend,
`linux-adapter` isolates native FFI, `airtraffic` owns framing/state machines, and CLI/GUI
own transaction policy and local files. [Protocol notes](docs/AIRTRAFFIC-RESEARCH.md) and
[transaction/recovery design](docs/WALLET-TRANSACTIONS.md) describe observed behavior.

Bundled archives include native-library licenses, exact distribution source packages and
a checksum/version manifest. Packaging uses an explicit file allowlist. Local device evidence, card identifiers, tokens,
backups and journals are excluded from Git and archives. Logs contain counts and protocol
state, not raw device syslog or pairing material. Remove completed temporary resources and
private test data after validation. See [installation and uninstall](docs/INSTALL.md) and
[provenance](docs/PROVENANCE.md).

## Credits
- **[Lumid-Off](https://github.com/Lumid-Off)** (Windows Native Rust Port & Maintainer)
- **[mak5er](https://github.com/mak5er)** (Original macOS App & Exploit Research)
- **[AirLift](https://github.com/0xjohnnydev/airlift)** by **[0xjohnny (0xjohnnydev)](https://github.com/0xjohnnydev)**: Original AirTraffic/ATAirlock sandbox escape and proof of concept underlying `AirliftFFI`.