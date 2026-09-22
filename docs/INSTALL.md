# Linux installation and binary builds

AirCard-Linux distributes native Linux executables. No Flatpak, AppImage or container
runtime is required to run the program. Hardware support remains experimental; consult
the compatibility table in README before attempting device operations.

## Debian and Ubuntu

Build dependencies (Debian 12/13, Ubuntu 22.04/24.04):

```sh
sudo apt-get update
sudo apt-get install build-essential pkg-config curl ca-certificates \
  libimobiledevice-dev libplist-dev libusbmuxd-dev \
  libimobiledevice-utils usbmuxd avahi-utils
```

For a compatible prebuilt CLI binary, the runtime tools pull the native library packages:

```sh
sudo apt-get install libimobiledevice-utils libusbmuxd-tools usbmuxd avahi-utils
```

Library package names differ across releases: for example Debian 12 uses
`libimobiledevice6`, while Debian 13 uses `libimobiledevice-1.0-6`. Install through the
package manager, which resolves its matching libplist/libusbmuxd dependencies.
Do not create compatibility symlinks between different SONAMEs.

Sources: [Debian 12 runtime](https://packages.debian.org/bookworm/libimobiledevice6),
[Debian 13 runtime](https://packages.debian.org/trixie/libimobiledevice-1.0-6),
[development package](https://packages.debian.org/trixie/libimobiledevice-dev).

## Arch Linux and derivatives

On an up-to-date Arch installation:

```sh
sudo pacman -S --needed base-devel pkgconf curl ca-certificates \
  libimobiledevice libplist libusbmuxd usbmuxd avahi
```

Runtime-only installation:

```sh
sudo pacman -S --needed libimobiledevice libplist libusbmuxd usbmuxd avahi
```

Arch includes development headers in the library packages; there are no separate `-dev`
packages. If repository metadata is stale, follow your distribution's normal complete
upgrade procedure first. Do not use `pacman -Sy` followed by selective installation.
Manjaro uses its own repository snapshots; build locally if an Arch binary requires a
newer library or glibc than that snapshot provides.

Sources: [libimobiledevice](https://archlinux.org/packages/extra/x86_64/libimobiledevice/),
[libusbmuxd](https://archlinux.org/packages/extra/x86_64/libusbmuxd/).

## Fedora

Build dependencies:

```sh
sudo dnf install gcc gcc-c++ make pkgconf-pkg-config curl ca-certificates \
  libimobiledevice-devel libplist-devel libusbmuxd-devel \
  libimobiledevice-utils usbmuxd avahi-tools
```

Runtime-only installation:

```sh
sudo dnf install libimobiledevice libplist libusbmuxd \
  libimobiledevice-utils usbmuxd avahi-tools
```

Fedora splits the command-line diagnostics into `libimobiledevice-utils` and development
headers into `-devel` packages. Use packages from the same Fedora release.

Sources: [libimobiledevice packages](https://packages.fedoraproject.org/pkgs/libimobiledevice/),
[CLI utilities](https://packages.fedoraproject.org/pkgs/libimobiledevice/libimobiledevice-utils/),
[libplist-devel](https://packages.fedoraproject.org/pkgs/libplist/libplist-devel/),
[libusbmuxd-devel](https://packages.fedoraproject.org/pkgs/libusbmuxd/libusbmuxd-devel/).

## Rust and build

Use Rust stable with edition 2024 support. The workspace requires Rust 1.88 or newer;
use the checked-in Cargo.lock. Install Rust through [rustup](https://rustup.rs/) if the
system compiler is older, then:

```sh
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
rustup override set stable
pkg-config --modversion libimobiledevice-1.0 libplist-2.0 libusbmuxd-2.0
cargo build --locked --workspace --release
./target/release/aircard --help
```

Required native minimums are libimobiledevice 1.3.0, libplist 2.2.0 and libusbmuxd 2.0.0.
A minimum build version does not guarantee compatibility with a particular iOS version.
The runtime must match the binary's CPU architecture, glibc requirement and native library
SONAMEs. A binary built on current Arch is not automatically compatible with older Debian.
Prefer the build for your distribution/release, or compile on the target distribution.
For a binary you built or otherwise trust, `ldd ./aircard` lists missing runtime libraries.

Optional per-user installation:

```sh
install -Dm755 target/release/aircard "$HOME/.local/bin/aircard"
```

## USB and Wi-Fi diagnostics

Run AirCard as your regular user. Do not start a second usbmuxd manually to work around
permissions. Distribution usbmuxd packages normally install the required udev rules and
may start the daemon on device attachment rather than enabling a persistent service.

```sh
systemctl status usbmuxd --no-pager
ls -l /run/usbmuxd
idevice_id -l
idevice_id -n
avahi-browse --resolve --terminate _apple-mobdev2._tcp
./aircard devices
./aircard probe --transport usb
./aircard probe --transport wifi
```

The diagnostic tools can print private UDIDs/network identifiers; keep that output local.
Connect and unlock your own device and confirm that this host is already paired. AirCard
never initiates pairing, edits pair records, changes trust settings or disables service TLS.
If pairing is needed, establish it separately through the normal system trust flow.

Wi-Fi requires a paired reachable phone advertising Wi-Fi sync and a mux backend exposing
network entries. Discovery in Avahi alone does not add such entries to Linux usbmuxd.
`--transport wifi` never falls back to USB. Do not disable a firewall or install a proxy
as a blind workaround; inspect discovery/backend support first.

## Uninstall

Remove only the executables you installed, for example:

```sh
rm "$HOME/.local/bin/aircard"
cargo clean
```

AirCard does not install a system service. Delete your own exported previews/snapshots and
redirected logs when no longer needed. Native library and usbmuxd packages may be shared
with other software; uninstall those only through your package manager if no longer used.
