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
- GUI: six native synthetic views captured and inspected (light/dark, help, apply/restore,
  explicit confirmation and compact window); device operations run the validated CLI.
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

Local verification passed: 72 tests, `cargo fmt --check`, workspace clippy with warnings
denied, release build, six GUI captures and native archive launch/allowlist/checksum checks.

CI is configured for `ubuntu-latest` with latest stable Rust: fmt, clippy with warnings denied,
workspace tests, release build, synthetic native GUI captures and allowlisted binary archives.
Configured checks do not establish that hosted jobs have run or that devices were tested there.
