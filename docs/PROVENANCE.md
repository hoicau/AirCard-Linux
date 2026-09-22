# Provenance and implementation boundary

The Linux branch descends from Lumid-Off/AirCard-Windows **v1.2.2**, commit
`d41aa1f2e1012bcd0af25d26f7579f0c5af645f7`. The original MIT `LICENSE` is preserved byte-for-byte.
The local `reference/aircard-windows` branch and `v1.2.2` tag retain the original source.
The Linux working tree replaces the Windows application. To inspect the original:

```sh
git show v1.2.2:src/airtraffic.rs
git show v1.2.2:src/scanner.rs
git show v1.2.2:src/image_skin.rs
git show v1.2.2:src/passthm.rs
```

- [AirCard-Windows source](https://github.com/Lumid-Off/AirCard-Windows/tree/d41aa1f2e1012bcd0af25d26f7579f0c5af645f7): `scanner.rs` is adapted into `crates/core/src/scanner.rs`; persistence and name extraction are omitted. Hash filters retain upstream behavior, including known false-positive heuristics.
- [airlift host source](https://github.com/0xjohnnydev/airlift/blob/c684cd41ca0ded2d1ab780c15f6ead05509ce062/Sources/airtraffic_host.m): examined for public high-level parameter models and message order. No Objective-C implementation or Apple framework is copied.
- [libimobiledevice 1.4.0](https://github.com/libimobiledevice/libimobiledevice/tree/1.4.0): public installed headers define the Linux adapter ABI. `src/lockdown.c` and `src/service.c` were inspected for pairing and TLS semantics.

No `MobileDevice.dll`, `AirTrafficHost.dll`, framework, or Apple private library is loaded, bundled, modified, or reverse engineered. Linux native libraries are dynamically linked; their upstream LGPL licenses apply to those system components.

The subsequent native handshake references public wire behavior in
[shinkuan/AirCard-Linux](https://github.com/shinkuan/AirCard-Linux/blob/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/atc.rs)
(MIT). The client is independently implemented with strict session/order validation,
resource bounds and terminal rejection handling. The complete pinned source record,
including inspected but unused Grappa wrappers, is in [READY-FOR-SYNC.md](READY-FOR-SYNC.md).
No authentication-token table or private-library helper is shipped. The owner explicitly
authorized a controlled experiment with index 0 of the pinned public Grappa table; that
temporary token was deleted afterward. The CLI accepts only caller-supplied token files.
Core assets.rs and passthm.rs adapt the upstream MIT image_skin.rs, passthm.rs and
card resource names; filesystem and protected-path write mechanisms are excluded.

## Adapter choice

The old [libimobiledevice-sys](https://github.com/aspenluxxxy/libimobiledevice-rs) binding uses an older bindgen toolchain; [rusty_libimobiledevice](https://github.com/jkcoxson/rusty_libimobiledevice) points users to a separate pure-Rust replacement. For this bounded PoC we need explicit existing-pair session control, extended transport enumeration, TLS error propagation, and small ownership scope. A header-compiled C adapter with a safe Rust RAII wrapper was selected rather than importing either complete stack. This is a project-specific maintenance decision, not a claim that other bindings are unusable.

`crates/linux-adapter` is the only unsafe crate. Constructors/destructors use installed public headers so libplist 2.2 and newer ABI differences do not leak into handwritten Rust declarations. All pointers stay behind owned, non-Send native handles; child clients outlive their sessions only through ownership. `plist` is pinned to 1.10.1 because its opt-in event API is used to enforce limits before constructing nested values.

The normal AFC service on the tested phone requests no service TLS. If AFC requests TLS on another device, this PoC fails closed: libimobiledevice 1.4.0's public `service_client_new` ignores its TLS-enable return value, and AFC exposes no public connection-injection API. Raw syslog/ATC services instead use public `idevice_connect` and explicitly checked `idevice_connection_enable_ssl`.
