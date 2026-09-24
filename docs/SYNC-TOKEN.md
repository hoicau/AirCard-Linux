# Sync token setup

## Automatic offline setup

In **Apply Artwork**, select your paired iPhone and open the intended card in Wallet
when prompted. After a card identifier is selected, AirCard saves its built-in sync
token and fills in its path automatically. Setup works offline, including the first
launch, and requires no terminal command or Apple Account login.
For Restore/Recover, or to retry setup, open **Advanced Options** and click
**Set up sync token**.

The ten public compatibility-token constants are stored in
[`crates/cli/src/sync_token.rs`](../crates/cli/src/sync_token.rs).
Setup uses entry 0, decodes its 168 hexadecimal characters into 84 bytes, and writes
a file with mode 0600. It makes no network requests and does not invoke `curl`.
The per-user location is `$XDG_DATA_HOME/aircard/token.bin`, or
`~/.local/share/aircard/token.bin` when `XDG_DATA_HOME` is unset or relative.
The AirCard directory has mode 0700. A valid cached file is reused on subsequent
launches, including an existing custom token. Setup never overwrites existing files.

## CLI and custom files

CLI setup uses the same offline installation and private-file policy, without an
iPhone connected:

```sh
./aircard setup-token
# Or choose a new output file in an existing directory:
./aircard setup-token --output /path/to/token.bin
```

The command prints the saved path, never token contents. Select a custom file under
**Advanced Options → Use an existing token / show path**, or pass
`--grappa-token /path/to/token.bin` to a device command. A custom file must contain
exactly 84 raw bytes and have mode 0600 (`chmod 600 /path/to/token.bin`).

Invalid cached files are reported and preserved. Move an invalid file aside or choose
a valid private file before retrying setup. To remove the cached token, delete only
`token.bin` in the AirCard data directory; keep any backups and recovery folders.

## Compatibility and provenance

ATC on the tested iPhone requests a Grappa challenge. The native implementation reads
an 84-byte token from a private local file. Pairing and TLS continue to use the existing
trusted host session. Token contents and pairing records are never printed in logs.
Local token files and pairing records are excluded from release archives.

Previous compatibility testing used entry 0 of the public token table in
[shinkuan/AirCard-Linux, commit 7686fd2](https://github.com/shinkuan/AirCard-Linux/blob/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/grappa.rs).
The public compatibility constants are now included in AirCard's source, with the
default token embedded in the binaries. Setup always chooses entry 0 and does not
cycle through the table after authentication failure.

Successful local setup does not establish compatibility with a particular iOS version.
If authentication is rejected, retain any recovery directory and use the reported
protocol stage to diagnose compatibility. Remove temporary test copies after restoration.

## Windows token generation

Reviewed AirCard-Windows commit `a0d546e05a04752d0da40105dc3b3528ced81757`:

- [`flasher.rs::generate_token`](https://github.com/Lumid-Off/AirCard-Windows/blob/a0d546e05a04752d0da40105dc3b3528ced81757/src/flasher.rs#L40-L67)
  obtains 10 random bytes with `BCryptGenRandom` and formats 20 hexadecimal characters.
  Callers append that string to staging, link and recovery directory names.
- [`airtraffic.rs`](https://github.com/Lumid-Off/AirCard-Windows/blob/a0d546e05a04752d0da40105dc3b3528ced81757/src/airtraffic.rs#L100-L180)
  performs sync through Apple's host API. [`apple.rs`](https://github.com/Lumid-Off/AirCard-Windows/blob/a0d546e05a04752d0da40105dc3b3528ced81757/src/apple.rs#L165-L174)
  loads `AirTrafficHost.dll`. The reviewed Rust source provides no standalone
  84-byte Grappa token generator to port to native Linux.

AirCard-Linux already generates a fresh transaction UUID using
`/proc/sys/kernel/random/uuid` in [`wallet.rs`](../crates/cli/src/wallet.rs) and checks
for staging-path collisions. This supplies the same directory-isolation role as the
Windows random token. Grappa compatibility data remains a separate input; random
directory identifiers cannot substitute for it.
