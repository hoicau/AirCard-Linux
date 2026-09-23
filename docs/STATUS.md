# Verification status

Current delivery scope: Wallet card artwork only, including preview, device/card selection,
private backups, restore and interrupted-operation recovery. No lock-screen customization,
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
- GUI: twelve native synthetic views captured and inspected, including light/dark,
  expanded automatic save paths, Restore/Recover guidance and compact windows. Device
  operations run the validated CLI.
- Wi-Fi: not accepted on hardware; owner deferred it. No USB fallback is performed.

The Wallet-only release meets the local USB acceptance scope. Wi-Fi, additional cards/OS
versions and hosted CI outcomes are outside this verified boundary.
Hardware reports and private fixtures stay in ignored local directories and are not shipped.

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
