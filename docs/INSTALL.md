# Linux installation and binary builds

AirCard-Linux distributes native Linux executables. No Flatpak, AppImage or container
runtime is required to run the program. Hardware support remains experimental; consult
the compatibility table in README before attempting device operations.

CI release archives include a `lib/` directory containing matching libimobiledevice,
libusbmuxd, libplist and any required libimobiledevice-glue. Keep this directory beside
`aircard` when extracting or moving the application. The CLI uses relative ELF RUNPATHs
to load this set, including indirect device-library dependencies. No system library is
replaced and no compatibility symlink is needed. glibc, the dynamic loader, TLS/graphics
libraries and the usbmuxd service remain system dependencies. See `BUILD-INFO.txt` for
the build environment and glibc requirements.

## Debian and Ubuntu

Build dependencies (Debian 12/13, Ubuntu 22.04/24.04):

```sh
sudo apt-get update
sudo apt-get install build-essential pkg-config curl ca-certificates \
  libimobiledevice-dev libplist-dev libusbmuxd-dev \
  libimobiledevice-utils usbmuxd avahi-utils \
  libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxkbcommon-dev \
  libwayland-dev libegl1-mesa-dev libgl1-mesa-dev
```

For compatible native CLI and GUI binaries:

```sh
sudo apt-get install libimobiledevice-utils libusbmuxd-tools usbmuxd avahi-utils \
  libx11-6 libxi6 libxcursor1 libxrandr2 libxkbcommon0 libxkbcommon-x11-0 \
  libwayland-client0 libegl1 libgl1 xdg-desktop-portal
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
  libimobiledevice libplist libusbmuxd usbmuxd avahi \
  libx11 libxi libxcursor libxrandr libxkbcommon libxkbcommon-x11 wayland mesa xdg-desktop-portal
```

Runtime-only installation:

```sh
sudo pacman -S --needed libimobiledevice libplist libusbmuxd usbmuxd avahi \
  libx11 libxi libxcursor libxrandr libxkbcommon libxkbcommon-x11 wayland mesa xdg-desktop-portal
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
  libimobiledevice-utils usbmuxd avahi-tools \
  libX11-devel libXi-devel libXcursor-devel libXrandr-devel libxkbcommon-devel \
  wayland-devel mesa-libEGL-devel mesa-libGL-devel
```

Runtime-only installation:

```sh
sudo dnf install libimobiledevice libplist libusbmuxd \
  libimobiledevice-utils usbmuxd avahi-tools \
  libX11 libXi libXcursor libXrandr libxkbcommon libxkbcommon-x11 \
  wayland-libs mesa-libEGL mesa-libGL xdg-desktop-portal
```

Fedora splits the command-line diagnostics into `libimobiledevice-utils` and development
headers into `-devel` packages. Use packages from the same Fedora release.

Sources: [libimobiledevice packages](https://packages.fedoraproject.org/pkgs/libimobiledevice/),
[CLI utilities](https://packages.fedoraproject.org/pkgs/libimobiledevice/libimobiledevice-utils/),
[libplist-devel](https://packages.fedoraproject.org/pkgs/libplist/libplist-devel/),
[libusbmuxd-devel](https://packages.fedoraproject.org/pkgs/libusbmuxd/libusbmuxd-devel/).

## Desktop session and file pickers

The GUI uses eframe 0.33, OpenGL and X11/Wayland. Run it inside your normal desktop
session, with its working graphics driver. `aircard` remains usable without a display.
The Browse buttons use the XDG desktop portal; a Flatpak runtime is not involved.
Install your desktop's portal backend if missing: GNOME commonly uses
`xdg-desktop-portal-gnome`, KDE uses its KDE backend (named `xdg-desktop-portal-kde`
on Arch/Fedora and `xdg-desktop-portal-kde` on Debian), and other desktops can use
`xdg-desktop-portal-gtk`. These backend packages are available through apt/pacman/dnf.
Use the backend recommended by your desktop; entering paths directly also works.

```sh
systemctl --user status xdg-desktop-portal --no-pager
```

Graphics/file-picker package references:
[Debian libxkbcommon](https://packages.debian.org/trixie/libxkbcommon0),
[Debian portal backend](https://packages.debian.org/trixie/xdg-desktop-portal-gtk),
[Arch libxkbcommon](https://archlinux.org/packages/extra/x86_64/libxkbcommon/),
[Fedora Mesa](https://packages.fedoraproject.org/pkgs/mesa/).

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
./target/release/aircard-gui
```

Required native minimums are libimobiledevice 1.3.0, libplist 2.2.0 and libusbmuxd 2.0.0.
A minimum build version does not guarantee compatibility with a particular iOS version.
The runtime must match the binary's CPU architecture and glibc requirement. Unbundled local
builds also require matching device-library SONAMEs. A binary built on current Arch is not
automatically compatible with older Debian.
Prefer the build for your distribution/release, or compile on the target distribution.
For a binary you built or otherwise trust, `ldd ./aircard` lists missing runtime libraries.

Optional per-user installation for a local build:

```sh
install -Dm755 target/release/aircard "$HOME/.local/bin/aircard"
install -Dm755 target/release/aircard-gui "$HOME/.local/bin/aircard-gui"
```

Keep both executables together: the GUI starts the adjacent CLI worker. If they must
live in different directories, pass `aircard-gui --cli /absolute/path/to/aircard`.
Before starting a task, the GUI checks `aircard --version` with a three-second deadline
and requires an exact version match. Replace both files from the same release if that
check fails; consult native library dependencies if the CLI cannot run at all.
Do not launch the GUI as root. It does not install a daemon or persist device identifiers.

For a bundled release, retain the entire extracted directory in a user-owned location and
launch `aircard-gui` there. Copying only the two binaries into `~/.local/bin` loses the
bundled libraries. If the GUI reports a specific missing `.so`, first check that `lib/`
was extracted beside `aircard`. A glibc version error requires a compatible build or a
local build; bundling device libraries does not change the minimum glibc version.

## Native archives

```sh
./scripts/package-native.sh
# Produces dist/aircard-linux-0.1.2-<architecture>.tar.gz and .sha256
```

Pass an optional label, such as `debian-13`, to distinguish local builds.

CI uses `./scripts/package-native.sh --bundle-native`. This mode requires Debian/Ubuntu,
`patchelf`, matching binary/development packages, and `deb-src` indexes for those exact
versions. It copies the complete device-library family and sets `$ORIGIN/lib` on the CLI
and `$ORIGIN` on each bundled library. It also includes:

- `NATIVE-LIBRARIES.json`: package versions and original/bundled library SHA256 hashes.
- `docs/native-licenses/`: distribution copyright files and referenced license texts.
- `native-sources/`: exact `.dsc` and source archives, including distribution patches and
  build instructions, for rebuilding or replacing the dynamic libraries.

Packaging fails if a matching source is unavailable or a device library resolves outside
the archive. CI checks relocated startup on Ubuntu and in an Arch container without
system libimobiledevice/libusbmuxd/libplist packages. The `ubuntu-latest` runner and latest
stable Rust remain in use. Containers are used only for CI verification.

Packaging additionally needs Python 3, GNU tar, gzip and binutils/readelf. The script
builds with Cargo.lock, includes the CLI, GUI, license notices, docs and BUILD-INFO, and
refuses to overwrite an existing archive. It explicitly excludes private device data and
private JSON evidence. Verify the adjacent checksum before extracting, then run `./aircard-gui`
from the extracted directory. Never mix binaries from different releases.
CI builds branch pushes and pull requests on `ubuntu-latest` with latest stable Rust;
archive names contain the version and CPU architecture. Tag pushes do not trigger Linux CI.
Publishing a release downloads the existing successful branch artifact for the exact tagged
commit, verifies checksums and `BUILD-METADATA.json` (commit, version and clean tree), then
uploads the archive and SHA256 file. It does not compile or call `linux.yml` again.

Commit the version and lockfile updates, push the branch, wait for its CI to pass, then
publish the matching tag, such as `v0.1.2`. If the artifact is missing, pending or expired,
the release upload fails with guidance. Wait or rerun the original branch CI, then rerun
the release workflow. Rerunning a release uses its original commit; it does not pick up
later branch changes. Successful uploads replace assets with the same names.

## Automatic backup and recovery locations

The GUI creates a unique private folder for each confirmed Apply or Restore under
`$XDG_DATA_HOME/aircard/operations` (default `~/.local/share/aircard/operations`). The
`backup.json` file keeps the original card artwork for Restore. The `recovery/` directory
records transaction progress and original data for Recover after an interruption.
Planning or cancelling a confirmation does not create these files. The CLI creates the
backup and journal when it runs; successful cleanup removes the journal, keeping the backup.

Apply uses `operations/<device-digest>/cards/<card-digest>/<operation>/backup.json`,
with its recovery journal beside the backup. Repeated changes to the same card keep separate
operation folders under that card. Restore uses `operations/<device-digest>/restores/<operation>`
because a manually selected backup can refer to a different card from the detected one.
The digests are SHA-256 hashes, so raw device/card identifiers never become path names.

**Advanced save locations** displays the generated paths and allows manual overrides.
**Restore or recover** offers the latest backup for the selected iPhone and card, refreshed
when either selection changes. Unfinished operations are discovered across all cards on
the iPhone and remain visible while switching cards. Both are rediscovered after restart.
Backups from the previous layout remain in place and can be selected with Browse; their
unfinished recovery directories are still detected automatically. Use Browse for custom locations.
Explanations are shown as wrapping text outside path fields. Restore requires an existing
backup from Apply; a new filename, image, exported ZIP or recovery folder is not a backup.
Private files must have no group/other access; `chmod 600` sets the expected file permissions.

## USB and Wi-Fi diagnostics

Run AirCard as your regular user. Do not start a second usbmuxd manually to work around
permissions. Distribution usbmuxd packages normally install the required udev rules and
may start the daemon on device attachment rather than enabling a persistent service.

The GUI's **Check device** button checks pairing and AFC file access without writes.
**Automatic** chooses USB first only when one paired physical iPhone is available;
multiple phones still need a choice. Explicit USB/Wi-Fi selections are preserved.
Pairing and read-only service startup retry one timeout/disconnection on the same route.
These checks do not establish ATC authentication or artwork compatibility; those are
validated by the operation itself. No artwork write is automatically retried.

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
rm "$HOME/.local/bin/aircard" "$HOME/.local/bin/aircard-gui"
cargo clean
```

Automatic token setup stores a private cache in `$XDG_DATA_HOME/aircard`, or
`~/.local/share/aircard` by default. This directory also holds automatically saved backups and recovery data. Keep it while
you need Restore or have an unfinished operation; do not remove it just to clear the token.

AirCard does not install a system service. Delete your own exported previews/card backups and
redirected logs when no longer needed. Native library and usbmuxd packages may be shared
with other software; uninstall those only through your package manager if no longer used.
