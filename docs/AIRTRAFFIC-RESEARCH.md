# Native AirTraffic protocol findings

## Conclusion

**Continue with the Wallet-only implementation.** Linux can start `com.apple.atc` through
existing paired lockdownd sessions, negotiate the requested TLS, receive `SyncAllowed`,
reach `ReadyForSync`, validate asset manifests and complete transfers without Apple host
libraries. Native card artwork installation and byte readback have succeeded on one USB
connected iOS 27.0 device. The owner confirmed the changed Wallet card and restored original.

This replaces the earlier receive-only stopping conclusion. Wi-Fi and other iOS versions
remain unverified. See [current acceptance](STATUS.md) and [provenance](PROVENANCE.md).

## Observed transport and envelopes

The first messages travel from device to host after service TLS. ATC frames use a 4-byte
**little-endian** length followed by a binary plist. Root fields include `Command`, `Session`,
`Type` and optional `Params`. The validated message type is 0. Startup session 0 and sync
session 1 are handled explicitly. A retained session-1 startup message cannot substitute
for a fresh `ReadyForSync` in the current request.

The first receive-only observations contained complete payload lengths 184, 146, 801 and
396 bytes (1543 bytes including four length headers). Later sessions can differ. These are
observations, not fixed packet sizes. Frames, plist expansion and nesting are bounded.
Unknown ordering, types, sessions, commands and oversized payloads fail with a named stage.

## State sequence

| State | Required input/output |
| --- | --- |
| Connecting | Paired service and verified TLS; bounded startup capabilities/messages |
| SyncAllowed | Explicit root `SyncAllowed`; reject protected/rejected device state |
| HostInfo | HostInfo on session 0 with validated library identity and capabilities |
| SyncRequest | RequestingSync for Book on session 1 |
| ReadyForSync | Explicit current-session ReadyForSync; Ping receives Pong |
| MetadataSyncFinished | Send FinishedSyncingMetadata with Book sync type |
| AssetManifest | Require every requested ID exactly once, permitted Book type, IsDownload=true; preserved IDs must have IsDownload=false |
| AssetCompleted | Send FileComplete for the exact validated asset ID and target; check preconditions before each completion |
| Finished | Explicit SyncFinished, followed by independent movement/byte verification |

A global deadline bounds each connection. Send errors are terminal because partial delivery
cannot be retried safely. Cancellation, disconnect, malformed messages, service denial,
manifest errors and changed preconditions retain their stage in structured events. Logging
contains directions, command/session identifiers, byte lengths and durations, never full
manifest payloads, pair records or certificates.

## Authentication and StreamingZip

The tested device requests a Grappa challenge. An explicit caller-supplied 84-byte token
from the pinned public Linux implementation enabled the handshake. Without it the earlier
attempt was rejected; pairing alone did not satisfy that check. Accepted capability values
are validated before a token is sent. [Token setup](SYNC-TOKEN.md) documents the exact source.
No Apple binary or authentication-token table is distributed with AirCard.

StreamingZip is a separate service and format: **big-endian** length plus binary-plist
`MediaSubdir`, followed by the stored ZIP stream. Success requires its bounded
`DataComplete` response. ATC framing and StreamingZip framing are never interchanged.
ZIP metadata and Unix mode fields follow the public AirCard/Airlift implementation.

The generated link/move behavior permits known Wallet resources to be exported into owned
Media staging paths for AFC backup. Direct AFC access to a protected link was denied on the
tested device, so the application uses ATC moves and validates bytes after each move.
Only typed card-background and generated-cache paths are permitted; ordinary archive and
file inputs retain traversal and size checks. Details are in [Wallet transactions](WALLET-TRANSACTIONS.md).

## Remaining unknowns

No claim is made about the complete private protocol, undocumented commands, future Grappa
variants, different Wallet layouts or compatibility across iOS releases. Successful service
startup alone is never reported as successful application. Other cards/OS versions and Wi-Fi
need separate acceptance. Private raw device evidence stays local and is excluded from Git
and release archives; the reproducible unit/mock tests use synthetic payloads only.
