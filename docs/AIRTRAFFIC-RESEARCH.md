# AirTraffic protocol research: stage 2

**Historical receive-only baseline, commit `a84e0d6`.** The outbound investigation and
current stopping point are superseded by [ReadyForSync research](READY-FOR-SYNC.md).
The observations and limitations below describe that earlier passive implementation.

Date: 2026-09-22. Scope: public source review and passive observations of one explicitly
authorized, already paired iOS 27.0 iPhone over USB. No Apple binaries were used.

## Decision

**Stop at the receive-only PoC for this iteration.** Linux can start `com.apple.atc`,
complete its required TLS handshake, and parse an incoming `SyncAllowed` message.
This meets the stage-2 first-message objective. The outbound message envelope,
request correlation/acknowledgment semantics, and `ReadyForSync` have not been validated.
The stage-3 prerequisite is therefore unmet. No sync, asset-completion notification,
Books metadata change, GUI or release package was attempted.

Phase 1 remains partly incomplete because no Wi-Fi device entry was discoverable on
this host. A strict Wi-Fi probe returned `NoDevice`; no fallback to USB was used.

## Evidence and limitations

The local-only sanitized run (local-only report) records command arguments,
exit codes, iOS version, service TLS requirements, byte lengths, parser observations
and AFC cleanup outcome. It contains no UDID, pairing certificate, raw syslog line,
raw ATC payload, device filename or Books data.

Initial independent passive sessions:

| Session | TLS | Bytes | Observation |
| --- | --- | --- | --- |
| A | true | 1485 | First u32 interpreted LE: 184; BE: 3087007744; initial framing still unclassified |
| B | true | 1543 | Four complete binary plists: 184, 146, 801, 396 payload bytes |
| C | true | 1543 | Same four lengths; `Command/Params/Type` field shapes repeated; fourth command `SyncAllowed` |

The lengths satisfy `4*4 + 184 + 146 + 801 + 396 = 1543`. No bytes remained between
frames. The first-byte direction was **device to host**, after service startup and TLS.
The application sent **zero ATC payload bytes**. Each passive session waited three
seconds and closed the service. `receive_timeout` describes the observation window
ending; when `first_message_validated=true`, it is not a rejected handshake.

Observed incoming framing on this device:

```text
4-byte unsigned little-endian payload length
payload-length bytes, beginning bplist00
repeat
```

`Command` is a string, `Params` is a dictionary, and `Type` is an integer.
Two other root keys per frame were not exported. Their semantics, the integer Type
values and the first three message types remain outside the validated model.
`SyncAllowed` is accepted only at the **root Command field** with a dictionary Params
and integer Type, in a complete LE32-framed stream. Merely finding the string anywhere
inside a plist cannot advance the observed state. This is an incoming message/structure
validation for one device, not proof of a complete envelope specification.

The original BE32 framing hypothesis did not match this phone. It remains a clearly
named synthetic research codec; it is never selected for live transmission. Candidate
parsing is receive-only, bounded to 64 KiB observation data, 128 frames, 1 MiB per plist,
32 collection levels and 16384 parser events. Expanded string/data bytes are bounded
before constructing recursive plist values. The generic 1 MiB frame limit has not been
validated as sufficient for real production manifests.

## Public-source facts versus unknown wire formats

References are pinned in [PROVENANCE.md](PROVENANCE.md).
AirCard-Windows `src/airtraffic.rs` and airlift `Sources/airtraffic_host.m` call Apple
`ATHostConnection*` and `ATCFMessage*` functions. They show API parameter values and
high-level order, but contain no implementation of those functions' transport framing.
Public references do not establish how to generate the unknown envelope fields.

| Operation | Publicly observable high-level parameters | Linux status |
| --- | --- | --- |
| HostInfo | Type=iTunes, Version=13.7.0.161, MacOSVersion, SyncHostName, LibraryID, SyncedDataclasses=[Book], SyncedAssetTypes=[Book], Wakeable=false | Pure plist model only; Linux host identity is a proposed parameter value, never sent |
| SyncRequest | dataclasses=[Book], anchors={}, hostInfo dictionary | Documented API arguments; exact wire field names/envelope unknown |
| MetadataSyncFinished | syncTypes={Book:1}, anchors={} | Documented API arguments; not sent |
| AssetManifest | Book array of dictionaries including AssetID and IsDownload | Pure validator rejects unknown dataclasses, unexpected/missing/duplicate IDs and unsafe paths |
| AssetCompleted | assetIdentifier, dataclass=Book, assetPath | Documented API arguments; no outbound implementation |

Neither reference proves success by waiting for an explicit final `SyncFinished` after
all notifications; each sleeps before reporting success. The Linux state model requires
an explicit completion event before entering Finished and makes no upstream success
assumption. Unknown integer values or fields will not be guessed into a production path.

## State and error model

The following is the tested **abstract** model. Only Connecting and receipt of SyncAllowed
have hardware evidence. Send events are simulated test events, not runtime device calls.

| Current state | Input/action | Next state | Budget / failure |
| --- | --- | --- | --- |
| Connecting | Start service + verified TLS, receive SyncAllowed | SyncAllowed | CLI `--timeout` window; start/session/TLS/receive classified separately |
| SyncAllowed | Send confirmed HostInfo envelope | HostInfo | Future send phase; disabled |
| HostInfo | Send confirmed SyncRequest envelope | SyncRequest | Future send phase; disabled |
| SyncRequest | Receive ReadyForSync | ReadyForSync | Future receive deadline; not observed |
| ReadyForSync | Send MetadataSyncFinished | MetadataSyncFinished | Future send phase; disabled |
| MetadataSyncFinished | Receive and validate AssetManifest | AssetManifest | Future manifest deadline/size/ID errors |
| AssetManifest | Explicitly approved single-asset completion | AssetCompleted | Future apply gate; disabled |
| AssetCompleted | Receive SyncFinished | Finished | Future completion deadline; no success inferred from sleep |
| Any active state | SyncFailed, timeout, cancellation or disconnect | Failed | Terminal; report original phase |
| Finished / Failed | Repeated or late event | Unchanged, order error | No duplicate completion or automatic retry |

All other transitions fail closed with the previous state and event. Phase budgets for
future states are intentionally not operational configuration until their wire protocol
exists. The current receive loop polls in at most 250 ms intervals. A separate CLI worker
watchdog bounds C calls, with an extra five seconds beyond the command deadline.

Device errors distinguish no route, ambiguous route, missing/invalid existing pairing,
trust denied/pending, locked device, service refusal, TLS, disconnect, timeout, AFC missing
path, invalid input and native errors. Native domain/code is retained without logging
pair records. Some C errors are inherently ambiguous and remain `Native`; do not infer
that every service refusal is a locked screen.

## TLS and pairing findings

`lockdownd_client_new_with_handshake` can initiate `Pair`. This PoC instead obtains only
the HostID from the existing usbmuxd pairing record, creates a plain lockdownd client,
and calls `lockdownd_start_session`. That function performs the session TLS negotiated
by lockdownd. Pairing data remains in process memory; raw record bytes and the extracted
HostID buffer are wiped before release. The libplist tree/native library may have their
own transient copies; no claim of complete process-memory erasure is made.

libimobiledevice 1.4.0 `service_client_new` ignores the return value of
`service_enable_ssl`. Raw ATC/syslog connections use `idevice_connect` followed by a
checked `idevice_connection_enable_ssl` call. TLS is never disabled to obtain a message.
AFC uses its public API only for the descriptor's non-TLS case; a TLS AFC descriptor is
rejected until that API limitation is solved. Observed AFC descriptor TLS was false;
observed syslog and ATC descriptor TLS were true.

## Reproduction and cleanup

1. Connect the same authorized paired device, unlock it and ensure no competing sync is running.
2. Run `devices`, `probe`, a bounded `syslog`, then `atc-smoke --transport usb --timeout 3`.
3. Repeat the passive smoke to check framing consistency. Retain only aggregate JSON.
4. Run the controlled `afc-self-test --apply` only when accepting its explicit write/read/delete scope.
5. Confirm `cleanup_ok=true`. Raw observation buffers are dropped when each worker exits.
6. Attempt `probe --transport wifi` independently. Record unavailable routes as incomplete hardware coverage.

The evidence runner creates only the explicitly requested report file. Delete disposable
reports afterward. The delivered aggregate evidence is an intentional source artifact.
No native daemon was started or reconfigured by this PoC. No device backup was needed
because no Books or user asset was modified. Disconnects/forced process termination can
interrupt AFC cleanup; the generated scratch path and cleanup failure must then be used
for manual recovery via normal AFC access. Atomic remote cleanup cannot be guaranteed
across a device disconnect.

## Requirements before continuing

- Establish outbound envelope fields and acknowledgment rules from a public implementation
  or authorized reference observations, without extracting or modifying Apple private libraries.
- Confirm HostInfo/SyncRequest behavior and actually observe ReadyForSync.
- Establish read-only manifest behavior, a complete Books snapshot/restore mechanism,
  strictly scoped target paths and a reversible single synthetic asset plan.
- Complete an independent Wi-Fi transport run with a discoverable paired network entry.
- Reassess whether normal permitted asset paths can provide the desired customization;
  the upstream directory-escape behavior conflicts with this task's security boundary.

Until those conditions hold, keep device sync and GUI development stopped.
