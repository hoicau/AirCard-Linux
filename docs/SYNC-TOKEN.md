# Sync token setup

## Automatic setup (recommended)

In **Apply & restore**, select your paired iPhone and open the intended card in Wallet
when prompted. After a card identifier is selected, AirCard obtains the sync token
and fills in its path automatically. No terminal command or Apple Account login is needed.
For Restore/Recover, or to retry a failed download, click **Get sync token automatically**.

AirCard downloads the pinned public source linked below over HTTPS, verifies its exact
SHA-256 and ten-entry table, then saves entry 0 as an 84-byte file with mode 0600.
The per-user location is `$XDG_DATA_HOME/aircard/token.bin`, or
`~/.local/share/aircard/token.bin` when `XDG_DATA_HOME` is unset or relative.
The AirCard directory has mode 0700. A valid cached file is selected again on launch,
so subsequent use works offline. Setup preserves existing files.

Install `curl` and `ca-certificates` through your distribution's package manager for the
first download. If GitHub cannot be reached, check the connection and retry the setup button.
The download has a 25-second deadline. Failure leaves the token field available for
**Use an existing token / show path** and **Browse**. Invalid cached files are reported;
move the invalid file aside, or select a valid private token instead. A manually selected
file must contain exactly 84 raw bytes and have mode 0600 (`chmod 600 /path/to/token.bin`).

CLI setup uses the same validation and private-file policy, without an iPhone connected:

```sh
./aircard setup-token
# Or choose a new output file in an existing directory:
./aircard setup-token --output /path/to/token.bin
```

The command prints the saved path, never token contents. An existing valid token is reused
without a network request. The public compatibility token is shared protocol material;
AirCard keeps its local copy private. Download success does not establish compatibility
with a particular iOS version. No token table is included in binaries or release archives.

## Source and manual fallback

ATC on the tested iPhone requests a Grappa challenge. The native implementation accepts an
84-byte token from a private local file. Pairing and TLS still use the existing trusted host
session. Tokens and pairing records are never printed by AirCard or embedded in its archives.

The compatibility tests used index 0 of the public token table in
[shinkuan/AirCard-Linux, commit 7686fd2](https://github.com/shinkuan/AirCard-Linux/blob/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/grappa.rs).
The table is protocol material, not an Apple library. Future iOS versions can reject it.
AirCard does not claim that it is universally valid. Automatic setup uses only this pinned
version and does not search for replacements after an authentication failure.

To reproduce that setup, the following reads the pinned public source as text, validates the
table shape, and creates a new 0600 token file. It never executes downloaded code. Run from
a private local directory (use the ignored `.local` directory in a source checkout) and do not add the output to Git:

```sh
mkdir -p .local
cd .local
python3 - <<'PY'
import os, re, urllib.request
url = 'https://raw.githubusercontent.com/shinkuan/AirCard-Linux/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/grappa.rs'
with urllib.request.urlopen(url, timeout=20) as response:
    source = response.read(1024 * 1024).decode('utf-8')
tokens = re.findall(r'"([0-9a-fA-F]{168})"', source)
if len(tokens) != 10:
    raise SystemExit('Unexpected upstream token table; no file was created')
with os.fdopen(os.open('token.bin', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb') as output:
    output.write(bytes.fromhex(tokens[0]))
    output.flush()
    os.fsync(output.fileno())
PY
```

Select that file in the GUI or pass `--grappa-token token.bin`. If authentication is rejected,
keep any recovery directory and use the reported protocol stage to diagnose compatibility.
Delete temporary test copies after restoration. Protect a retained token file with mode 0600.
To remove the cached setup token, delete only your AirCard data directory
(`~/.local/share/aircard` by default) when it is no longer needed.
