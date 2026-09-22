# AirCard-Linux

Hardware JSON reports under `docs/evidence/` are local-only and excluded from Git history.
Public research notes retain aggregate results; no evidence JSON is distributed.

> **Current checkpoint (2026-09-22):** USB ReadyForSync and controlled single-Book sync,
> byte verification and full restore have succeeded. Stage 4 offline resources and
> snapshot/recovery CLI are implemented; final feature acceptance remains incomplete.
> See [current status and commands](docs/STATUS.md) and
> [Debian/Ubuntu, Arch/Manjaro and Fedora installation guide](docs/INSTALL.md).
> The earlier PoC report below is historical and predates the authorized Grappa experiment.

**Experimental native Linux CLI PoC.** Work only with your own explicitly authorized,
already paired iPhone. This implementation never initiates pairing or bypasses trust.

On one **iOS 27.0** device, USB enumeration, existing-pair lockdownd sessions, TLS syslog,
AFC list/write/read/cleanup, and incoming ATC `SyncAllowed` were observed.
The native HostInfo/RequestingSync handshake is implemented behind `--apply`, but the
test phone returns `SyncFailed / ErrorCode=4`; **ReadyForSync is not hardware-verified**.
Wi-Fi routing is implemented for network entries supplied by usbmuxd, but **Wi-Fi has
not passed hardware acceptance** on this host. No full sync, Wallet/passcode modification,
Books backup/restore, GUI, AppImage or Flatpak is implemented.

The branch descends from [AirCard-Windows v1.2.2](https://github.com/Lumid-Off/AirCard-Windows/tree/d41aa1f2e1012bcd0af25d26f7579f0c5af645f7).
The original MIT license and reference history are retained. Apple DLLs/frameworks are
not dependencies. See [provenance](docs/PROVENANCE.md).

## Build

Rust stable (edition 2024), a C compiler, pkg-config and native development libraries:

```sh
# Debian / Ubuntu
sudo apt-get install build-essential pkg-config libimobiledevice-dev libplist-dev libusbmuxd-dev libimobiledevice-utils usbmuxd avahi-utils
# Arch / Manjaro
sudo pacman -S --needed base-devel pkgconf libimobiledevice libplist libusbmuxd usbmuxd avahi

rustup toolchain install stable --profile minimal --component rustfmt --component clippy
cargo build --locked --workspace --release
./target/release/aircard --help
```

Minimum native versions: libimobiledevice 1.3.0, libplist 2.2.0, libusbmuxd 2.0.0.
The lockfile fixes Rust dependency versions. Ubuntu 22.04/24.04 stable CI runs without
hardware. Local validation was performed on Manjaro, not in those Ubuntu runners.

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## CLI

All device commands accept `--udid <UDID>` and `--transport usb|wifi`. Without selectors,
exactly one route must be available; ambiguous routes are an error. There is no automatic
USB fallback for a Wi-Fi request. `--timeout` bounds the operation with a separate native
worker watchdog (plus five seconds of shutdown/setup allowance).

```sh
cargo run -p cli -- devices
cargo run -p cli -- devices --show-identifiers   # local use; keep identifiers private
cargo run -p cli -- probe --transport usb
cargo run -p cli -- syslog --transport usb --duration 30
cargo run -p cli -- scan --transport usb --duration 30
cargo run -p cli -- snapshot --transport usb
cargo run -p cli -- atc-smoke --transport usb --timeout 5
cargo run -p cli -- atc-ready --transport usb             # plan only, no device access
cargo run -p cli -- atc-ready --transport usb --timeout 10 --apply  # handshake only
cargo run -p cli -- afc-list --transport usb
cargo run -p cli -- afc-self-test --transport usb          # dry-run
cargo run -p cli -- afc-self-test --transport usb --apply  # controlled write/read/delete
cargo run -p cli -- probe --transport wifi
```

- Output is JSON Lines. Logs contain stages, native error domain/code, elapsed time and byte counts. Default device identifiers, card hashes, filenames and raw syslog text are suppressed.
- `syslog` continuously receives for its requested duration (up to 3600 seconds). `--raw` explicitly prints potentially private syslog to your terminal; do not commit it. No log file is created automatically.
- `scan` ports the upstream Wallet keyword/base64/entropy/dummy-hash filters. `--show-hashes` explicitly reveals matched hashes locally. Real Wallet card detection is still unverified: observed syslog sessions had no matching card events.
- `snapshot` emits **diagnostic metadata only**, not a complete or restorable Books snapshot. State-changing synchronization is gated on a future real backup/restore implementation.
- `atc-smoke` starts `com.apple.atc` and honors the TLS requirement, then receives only. The report distinguishes a validated incoming `SyncAllowed` envelope from a full protocol handshake. Exit 0 means the stage-2 receive-only target passed; it does not mean `ReadyForSync` or synchronization succeeded. Exit 3 means ATC startup or first-message validation failed. Other failures use exit 1, argument errors 2, watchdog 124, cancellation 130.
- `atc-ready` prints a dry-run plan without accessing the device. `--apply` opens the normal paired/TLS service, sends HostInfo and RequestingSync(Book), handles Ping/Pong, and closes on an exact ReadyForSync/session 1 or failure. No metadata or assets are sent. It never suppresses SyncFailed. The current test device rejects session 1 with error 4; an unmet Grappa requirement is suspected, not proven as the definition of that code. See [handshake research and evidence](docs/READY-FOR-SYNC.md). Applied success exits 0, protocol rejection 3, pre-service selection/pairing errors 1, cancellation 130.
- `afc-read <relative-path>` reads at most 1 MiB and reports only its length. Paths reject traversal, drive prefixes, control characters and symlink components. AFC operations remain inside the normal media service scope.
- `afc-self-test --apply` creates a unique `AirCard-Linux-PoC-*` directory, writes a 26-byte synthetic file, verifies it, and removes both. It refuses existing targets. No other CLI command writes device files. Cleanup errors are retained alongside the original error. A disconnect or forced termination can prevent remote cleanup; the emitted scratch path identifies exactly what must be removed after reconnection, through a normal AFC client. Do not delete unrelated directories. No rollback or recovery claim is made for AirTraffic sync.

For a reproducible sanitized evidence artifact:

```sh
python3 scripts/verify-device.py --binary target/release/aircard --output /tmp/aircard-check.json
# Include the explicitly authorized controlled AFC write test:
python3 scripts/verify-device.py --binary target/release/aircard --apply --output /tmp/aircard-write-check.json
# Review aggregate results, then delete temporary reports when finished.
rm /tmp/aircard-check.json /tmp/aircard-write-check.json
```

## Current acceptance and stopping point

| Phase | Status |
| --- | --- |
| 0: repository/build/CI | Implemented; four local checks pass; hosted CI awaits push |
| 1: Linux USB | Real iOS 27.0 device verified; syslog counters retained as sanitized evidence |
| 1: Wi-Fi | Strict route selection implemented; no discovered network entry; acceptance incomplete |
| 2: ATC | Service/TLS and incoming little-endian binary plist `SyncAllowed` verified on USB |
| ReadyForSync handshake | Native client and mock tests implemented; real USB returns SyncFailed/code 4 |
| 3: single-asset sync | Blocked: actual ReadyForSync not reached; Grappa interoperability unresolved |
| 4: assets and full snapshots | Deferred behind the protocol gate; original pure Rust sources remain in reference history |
| 5: GUI/distribution | Not started |

See [the original passive report](docs/AIRTRAFFIC-RESEARCH.md),
[the current handshake report](docs/READY-FOR-SYNC.md), and
sanitized hardware evidence (local-only report).
The upstream customization mechanism depends on escaping the expected asset directory.
This PoC preserves the requested path-validation/security boundary; it does not expose
that mechanism. Future resource generation must remain separate from device writes.

## Device and service diagnostics

```sh
systemctl status usbmuxd --no-pager
ls -l /run/usbmuxd
idevice_id -l
idevice_id -n
avahi-browse --resolve --terminate _apple-mobdev2._tcp
```

The last commands may display private device/network identifiers: inspect locally.
Connect and unlock the phone, use an existing trusted host pairing, and check installed
udev rules if USB devices are absent. On distributions with an on-demand static usbmuxd
service, attach the device before diagnosing an inactive daemon. Avoid running multiple
competing daemons or replacing pairing files. Pair manually outside this tool if needed.

Wi-Fi requires a reachable paired device with Wi-Fi sync advertised, plus a mux backend
that exposes network records. Standard Linux usbmuxd builds may enumerate USB only;
Avahi discovery by itself does not inject network devices into libimobiledevice.
This PoC does not replace usbmuxd or install an unverified network proxy. Empty network
results are reported as `NoDevice`, never silently retried over USB.

AFC normally requests no service TLS on the tested device. The adapter fails closed if
AFC requests TLS on a backend whose public constructor cannot propagate TLS failure.
ATC and syslog explicitly verify TLS success using public connection APIs.

## Uninstall and privacy

No install step, daemon, persistent device backup, or application data directory is
created. Remove your own copied binary and build output (`cargo clean`) to uninstall.
Remove any manually redirected private logs, diagnostic snapshots or temporary evidence.
System Rust/native packages are shared dependencies and should be removed only if you
no longer need them for other projects. Test scripts never save certificates or pair records.
