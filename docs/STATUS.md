# Verification status

Current delivery scope: Wallet card artwork only, including preview, device/card selection,
direct extraction, private backups, restore and interrupted-operation recovery. No lock-screen customization,
Books management or standalone protocol experiments are exposed as product functions.

## Hardware

One authorized, paired iPhone on iOS 27.0, connected over USB to Manjaro x86_64.

- Existing pairing, lockdownd, AFC and syslog relay: verified.
- Native ATC SyncAllowed, ReadyForSync, manifest and SyncFinished: verified.
- Preserved Books transport: synthetic asset was readable after restarting Books; the
  owner confirmed original books remained usable and the test book disappeared after restore.
- Wallet scanner: selected-card identifier found; identifiers remain private.
- Wallet artwork: one original background backed up, test artwork installed and read back
  exactly; generated display-cache invalidation completed.
- Wallet visual acceptance and post-restore appearance: owner confirmed both passed.
- Persistent card-apply: new backup saved, process exited successfully, staging removed.
- Independent card-restore: fresh process and Books snapshot; original artwork/cache bytes
  verified after restore; staging and journal removed.
- Abrupt interruption: process group killed after installed-artwork readback; a new
  card-recover process restored original bytes and completed cleanup from its durable journal.
- GUI: sixteen native synthetic views captured and inspected, including light/dark,
  expanded automatic save paths, Restore/Recover guidance and compact windows. Device
  operations run the validated CLI.
- Wi-Fi: not accepted on hardware; owner deferred it. No USB fallback is performed.

The Wallet-only release meets the local USB acceptance scope. Wi-Fi, additional cards/OS
versions and hosted CI outcomes are outside this verified boundary.
Hardware reports and private fixtures stay in ignored local directories and are not shipped.

## Artwork extraction verification (2026-09-30)

The owner selected a card on the paired USB iPhone running iOS 27.0.1 and closed Wallet
and Books before extraction. `card-extract` exported the available
`cardBackgroundCombined@2x.png` into a private ZIP. ZIP CRC checks passed and the extracted
bytes matched the captured device original exactly. Original artwork restoration passed
both move and readback verification; catalog restoration and staging cleanup succeeded.
No replacement artwork, cache invalidation or standalone backup was produced. The recovery
directory was removed after success. Visual appearance was not separately reaccepted.

The first attempt exposed a newly generated empty managed-sync lock that the restoration
allowlist did not recognize. The narrow cleanup fix passed a separate recovery on the same
device. Tests ensure nonempty/changed locks and managed user content still cause a conflict.
A candidate without supported resources then returned `card_artwork_not_found` and cleaned
up successfully; rescanning the previously customized card produced the successful ZIP above.

All 96 Rust tests, formatting, workspace Clippy with warnings denied and release build passed.
Nineteen native GUI smoke views passed, including extraction without a replacement image,
its confirmation dialog, light/dark themes and compact layouts. Wi-Fi extraction and abrupt
interruption during extraction remain unverified on hardware.

## Built-in token verification (2026-09-24)

The connected USB iPhone on iOS 27.0 passed pairing, AFC access and two authenticated
ATC handshakes using built-in token entry 0. The token was created with no external
programs available in `PATH`. Cached reuse preserved the token file's inode and timestamp.
Both TLS sessions advertised Grappa `(version=1, deviceType=0, protocolVersion=1)` and
returned `ReadyForSync`, in 274 ms and 238 ms respectively, with no protocol failure.
The test sent no manifest, metadata or asset payload; it did not apply or restore artwork.

The initial diagnostic AFC root listing contained 20 entries; inspection after the first
handshake contained 21. The first root-comparison check failed, and the initial names
were not retained, so the newly appearing entry was not identified. The repeat check
confirmed identical 21-entry root listings before and after its handshake. No AirCard
staging roots were present; system directories were left untouched. Temporary token files,
test executables and downloaded reference sources were removed after verification.

The 32 CLI/GUI tests, workspace clippy with warnings denied, formatting, diff checks and
release build passed. Offline tests cover the default token bytes, private file/directory
permissions, cached reuse, custom files and preservation of invalid existing files.

## Automated verification

Core tests cover binary plist limits, path validation, image output, allowed card resources,
retained catalog rows and transaction mappings. Protocol mocks cover fragmented I/O,
malformed framing, timeout, disconnect, cancellation, ordering, manifest rejection and
partial batches. Device mocks cover preservation and restoration conflicts. CLI tests cover
offline dry-run and excluded commands. GUI tests cover confirmation, redaction and supervision.

Local verification on 2026-09-23 passed: 89 Rust tests, four Python release/bundle tests,
`cargo fmt --check`, workspace clippy with warnings denied, release build, twelve GUI
captures, and native archive checksum/metadata/relocated startup checks.

The connected USB iPhone also passed automatic card detection, device diagnostics, image
preview/resource export, fresh token setup and cached reuse, controlled card-test with
automatic restoration, persistent Apply, backup validation, Restore, and abrupt interruption
after artwork readback followed by a separate Recover. All test journals were removed by
successful cleanup. At a later optional connectivity recheck, no Apple USB device was
present and usbmuxd was inactive; no post-test AFC directory-count comparison is claimed.
These checks verify bytes and cleanup; visual appearance was not reaccepted in this run.
Only a USB route was exposed during device discovery; Wi-Fi writes were not tested.

Default-location tests cover unique private paths, no creation before confirmation,
separate new outputs and existing restore inputs, device/card isolation, switching cards,
restart discovery and recovery discovery across all cards and the previous directory layout,
retained recovery after failure and removal after successful recovery. Backup diagnostics
cover missing paths, directories, symbolic links, permissions, size and invalid contents.

Branch CI is configured for `ubuntu-latest` with latest stable Rust. Tag pushes do not
rebuild. Release publishing only reuses an existing successful branch artifact for the
same commit, checking checksum, source commit, clean-tree status and version. Offline tests
cover selecting that artifact and rejecting stale, expired, mismatched or altered artifacts.
The changed hosted release upload still needs its first run after these changes are pushed.
