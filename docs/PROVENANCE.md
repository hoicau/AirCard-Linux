# Provenance

AirCard-Linux descends from Lumid-Off/AirCard-Windows **v1.2.2**, commit
`d41aa1f2e1012bcd0af25d26f7579f0c5af645f7`. Its original MIT LICENSE remains byte-for-byte
unchanged. The upstream tag and Git ancestry preserve the original implementation:

```sh
git show v1.2.2:src/image_skin.rs
git show v1.2.2:src/scanner.rs
git show v1.2.2:src/airlift.rs
```

- [AirCard-Windows](https://github.com/Lumid-Off/AirCard-Windows/tree/d41aa1f2e1012bcd0af25d26f7579f0c5af645f7): image preparation, card resource names, hash-filter behavior and stored StreamingZip layout are adapted from its MIT source. The GUI follows its artwork/background-task workflow. Windows DLL loading is omitted. Lock-screen theme conversion and application are outside this product.
- [airlift host source](https://github.com/0xjohnnydev/airlift/blob/c684cd41ca0ded2d1ab780c15f6ead05509ce062/Sources/airtraffic_host.m) and [public Python implementation](https://github.com/0xjohnnydev/airlift/blob/c684cd41ca0ded2d1ab780c15f6ead05509ce062/airlift.py): references for high-level sync parameters and observable link/move behavior. No Apple host framework is copied.
- [Public native ATC implementation](https://github.com/shinkuan/AirCard-Linux/blob/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/atc.rs) and [StreamingZip implementation](https://github.com/shinkuan/AirCard-Linux/blob/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/streaming_zip.rs): pinned MIT references for native framing and envelope behavior. This client's bounded transport, strict state validation and recovery policy are implemented in Rust.
- [libimobiledevice 1.4.0](https://github.com/libimobiledevice/libimobiledevice/tree/1.4.0): public headers define the native adapter ABI. Public lockdown/service code informed pairing and explicit TLS error checks.
- [book2pad](https://github.com/rk700/book2pad/blob/6bf346e0fa1eab23b94c935116a4462609a56b62/book2pad): reference for expanded EPUB storage in transport acceptance fixtures; no Python implementation is copied or Books management exposed.

No MobileDevice.dll, AirTrafficHost.dll, private Apple framework or Apple library is loaded,
bundled, modified or reverse engineered. The public ATC path/link mechanism is implemented
for explicitly selected card targets. Pairing and requested service TLS remain verified.
Grappa tokens come from caller-supplied local files; no token table is shipped.

## Linux adapter

Existing bindings were considered, including libimobiledevice-sys and rusty_libimobiledevice.
The project needs a small existing-pair session API, USB/Wi-Fi enumeration, checked TLS and
clear ownership. `linux-adapter` therefore compiles a minimal C adapter against installed
public headers and exposes owned Rust RAII handles. It is the only unsafe crate; core,
protocol, device policy, CLI and GUI forbid unsafe Rust.

libplist header differences stay inside the compiled adapter. Native handles are non-Send.
Normal AFC on the tested phone requests no TLS. If another device requests AFC TLS the
backend fails closed: libimobiledevice's public AFC service constructor does not propagate
all TLS-enable failures and offers no public connection-injection API. Raw ATC and syslog
connections explicitly check `idevice_connection_enable_ssl`.

The native system libraries are dynamically linked and retain their upstream LGPL licenses.
Rust dependencies and embedded-font license texts are included in packaged notices. The
eframe/egui MIT text is also retained in `docs/licenses/egui-MIT.txt`. `plist` is pinned to
1.10.1 to use its opt-in event parser for limits before constructing nested values.
