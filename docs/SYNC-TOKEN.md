# Sync token setup

ATC on the tested iPhone requests a Grappa challenge. The native implementation accepts an
84-byte token from a private local file. Pairing and TLS still use the existing trusted host
session. Tokens and pairing records are never printed by AirCard or embedded in its archives.

The compatibility tests used index 0 of the public token table in
[shinkuan/AirCard-Linux, commit 7686fd2](https://github.com/shinkuan/AirCard-Linux/blob/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/grappa.rs).
The table is protocol material, not an Apple library. Future iOS versions can reject it.
AirCard does not claim that it is universally valid or automatically fetch a replacement.

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
