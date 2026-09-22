# Native ReadyForSync handshake

Date: 2026-09-22. Scope: extend the Linux PoC through an exact `ReadyForSync` response
on the authorized, already paired test iPhone. This report supersedes the outbound
unknowns in the [receive-only baseline](AIRTRAFFIC-RESEARCH.md).

## Result and decision

The native handshake client and explicit `atc-ready --apply` command are implemented.
Mock transport tests reach ReadyForSync, including fragmented reads/writes and Ping/Pong.
**The real USB iOS 27.0 device has not reached ReadyForSync.** It repeatedly returns
`SyncFailed`, `Session=1`, `Type=0`, `Params.ErrorCode=4` after HostInfo/RequestingSync.
The error occurs in `SyncRequest`, and the command closes the connection and exits 3.
The stage-3 hardware prerequisite remains unmet; asset synchronization and GUI stay stopped.

The paired device's SyncAllowed reported `DataProtected=false`. A bounded simultaneous
syslog observation exported only fixed allowlisted words, including `error`, `grappa`,
`session`, `not`, from an ATC-related line. This suggests an unsatisfied Grappa session
requirement; it does **not** establish a formal definition of error code 4. A receive-only
10-second continuation after rejection produced no later ReadyForSync. The runtime
client never treats this rejection as stale or ignores it in anticipation of success.

The inspected public alternatives either use a static Grappa token table or call Apple
private frameworks. No independent Linux Grappa implementation was established within
this bounded investigation. No token table, captured token, Apple binary or private-library
code is incorporated or used by this repository. A public, independently implemented
Grappa/authentication path conforming to the project's constraints is the next research
requirement. This is an unresolved interoperability dependency, not proof that such an
implementation is impossible.

## Public source basis

- [AirCard-Windows v1.2.2](https://github.com/Lumid-Off/AirCard-Windows/blob/d41aa1f2e1012bcd0af25d26f7579f0c5af645f7/src/airtraffic.rs)
  and [airlift](https://github.com/0xjohnnydev/airlift/blob/c684cd41ca0ded2d1ab780c15f6ead05509ce062/Sources/airtraffic_host.m)
  provide the high-level Book request and HostInfo fields.
- [IpaInstall ATH.cpp](https://github.com/Kerrbty/IpaInstall/blob/ad2849f1ddd90f713154750b0cc2a43225656f13/client/aid2/ATH.cpp)
  shows that the `SendSyncRequest` API emits the wire command **RequestingSync**.
  Its keybag/Apple API operations are not used.
- [shinkuan/AirCard-Linux native ATC client](https://github.com/shinkuan/AirCard-Linux/blob/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/atc.rs)
  (MIT) establishes LE32 framing, the three-field outbound envelope, sessions 0/1,
  LocalCloudSupport=false and Ping/Pong behavior. This implementation independently
  applies those wire behaviors with stricter ordering and resource bounds. The reference's
  token table, suppression of SyncFailed and substitution of AssetManifest for ReadyForSync
  are not adopted.
- [AirCard-iOS Grappa wrapper](https://github.com/Mak5er/AirCard-iOS/blob/097a058c984ffc33ccb697b9dfe8058be3e86244/rust-core/src/grappa.rs)
  delegates to [GrappaHelper.m](https://github.com/Mak5er/AirCard-iOS/blob/097a058c984ffc33ccb697b9dfe8058be3e86244/ios-app/GrappaHelper.m).
  Inspection confirmed that the helper dynamically loads AirTrafficDevice/AirTrafficHost
  private frameworks and uses a pre-generated token table. It is not an independent
  native Linux generator and was not executed or copied into the project.

## Observed and implemented wire format

Each frame is `u32 little-endian payload length` followed by exactly that many bytes
of a binary plist. Inbound root fields observed on the test phone are Command (string),
Params (dictionary), Type=0, Session (unsigned integer), Id (unsigned integer).
The four startup commands are Capabilities, InstalledAssets, AssetMetrics, SyncAllowed,
all on session 0. Initial payload lengths were 184, 146, 801 and 396 bytes.
Id increments across connections; its correlation semantics remain unknown. This client
neither emits nor guesses Id values. It accepts only Type=0; different types fail closed.

The public native source establishes this outbound subset:

| Command | Session | Params |
| --- | --- | --- |
| HostInfo | 0 | HostInfo dictionary, LocalCloudSupport=false |
| RequestingSync | 1 | Dataclasses=[Book], DataclassAnchors={}, same HostInfo dictionary |
| Pong | 1 | Omitted |

HostInfo uses the upstream iTunes Type/Version values, MacOSVersion=Linux,
SyncHostName=AirCard-Linux, Wakeable=false, the Book dataclasses/assets, and a fresh
LibraryID UUID per invocation. It contains no private host/device identifiers or Grappa
token. A 200 ms pause between HostInfo and RequestingSync follows the public reference;
its necessity is not established. There is no device response acknowledging HostInfo
separately, so the HostInfo state means the full frame was sent, not accepted.

A symmetric five-field outgoing envelope was initially considered from passive fields,
then discarded when the public native client was found, **before any transmission**.
All actual outgoing experiments used the documented three-field subset. The tested
framing/envelope subset is not a claim of complete AirTraffic protocol compatibility.

## State, bounds and failure policy

| State | Input/action | Result |
| --- | --- | --- |
| Connecting | Receive startup metadata, then SyncAllowed/session 0 | SyncAllowed |
| SyncAllowed | DataProtected=true or malformed flag | Fail without sending HostInfo |
| SyncAllowed | Send HostInfo/session 0 | HostInfo |
| HostInfo | Send RequestingSync/session 1 | SyncRequest |
| SyncRequest | Receive exact ReadyForSync/session 1 | ReadyForSync, close connection |
| Waiting | Ping/session 0 or 1 | Pong/session 1, continue waiting |
| Any | SyncFailed on any session | DeviceRejected, preserve ErrorCode/session/previous stage |
| Any | Wrong session/order, malformed plist/envelope, timeout, cancellation or disconnect | Failed, close connection |

AssetManifest, SyncFinished or simply no error cannot stand in for ReadyForSync. Every
SyncFailed is terminal, including session 0. There is no automatic retry/reconnect after
a failed write or session: a native error may have followed a partial transmission.
The user can explicitly retry once the underlying failure is understood.

One deadline spans all handshake phases. Read polls are at most 250 ms and preserve
partial headers and payloads across timeout/interrupt returns. Successful partial writes
advance the offset exactly; failed writes are never retransmitted. The Linux adapter sets
SO_SNDTIMEO before the public send API; native/TLS calls additionally have the CLI worker
watchdog because a socket option alone does not guarantee a total TLS operation deadline.
The watchdog allows five seconds beyond `--timeout`, followed by up to two seconds for
termination/reaping. Protocol success is rechecked against deadline/cancellation after read.

Limits: 1 MiB per payload, 4 MiB total input, 128 received frames, the core parser's depth
and event/string/data budgets. Logs retain only allowlisted command enums, direction,
state, session, payload length, elapsed time, DataProtected and numeric ErrorCode.
They never serialize incoming Params or raw payloads. Unknown command names become
`Unknown`. Sent-byte counters cover successful send returns; bytes sent before an error
may be unknown. No success or rollback is inferred from those counters.

## Reproduce and interpret

```sh
cargo build --locked --workspace --release
# No device connection or pairing lookup, even when no device is available:
cargo run -p cli -- atc-ready --transport usb
# Explicitly allow opening a Book handshake session, stop before any metadata/assets:
cargo run -p cli -- atc-ready --transport usb --timeout 10 --apply
# Test a distinct network route; there is no USB fallback:
cargo run -p cli -- atc-ready --transport wifi --timeout 10 --apply
```

Exit 0 without --apply means only that a plan was printed. Exit 0 with --apply requires
an exact validated ReadyForSync. Protocol/service failure exits 3; pre-service device
selection/pairing failures exit 1; cancellation exits 130, watchdog expiration 124.
`--udid` is supported for explicit selection; never commit it in logs or command history.

The sanitized final CLI runs (local-only report) are local-only
hardware evidence. They are separate from the mock successes. Both final applied USB runs sent two frames (765 bytes), received five frames (1674 bytes),
and exited 3 with DeviceRejected in SyncRequest. TLS was enabled and DataProtected was
false in both runs. A subsequent USB probe succeeded with the same 20 AFC root entries;
this checks continued service availability, not a full device-state rollback.

Validation passed: `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
(35 tests), and `cargo build --workspace --release`. The new tests cover successful mock
handshake, fragmented I/O, partial-write failure, Ping/Pong, malformed/oversized messages,
wrong order/session, explicit rejection, cancellation, deadlines and connection cleanup.
A CLI integration test verifies dry-run with an unavailable mux socket.

The original passive
USB/syslog/AFC evidence (local-only report) remains unchanged.
Wi-Fi hardware acceptance remains incomplete when usbmuxd reports no network route.

No metadata-finished message, asset completion, AFC mutation, backup or app restart was
performed for this handshake task. Sockets/native handles are dropped on every path;
worker processes are reaped. Temporary research sources/binaries were removed. Only
reviewed aggregate evidence is intentionally retained in the repository. Starting a sync
session can affect transient device sync state, which is why the live command requires
--apply. No claim of a full sync rollback or successful application modification is made.
