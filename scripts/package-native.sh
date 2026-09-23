#!/usr/bin/env bash
# Explicit allowlist: no device evidence, token, snapshot, source tree or Apple binary.
set -euo pipefail
cd "$(dirname "$0")/.."
label="${1:-}"
if [[ -n "$label" && ! "$label" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]]; then
  echo 'Use a simple distribution label, e.g. debian-13 or arch-x86_64.' >&2
  exit 2
fi
cargo build --locked --workspace --release
arch="$(uname -m)"
version="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="gui"))')"
name="aircard-linux-${version}${label:+-${label}}-${arch}"
mkdir -p dist
if [[ -e "dist/$name.tar.gz" || -e "dist/$name.tar.gz.sha256" ]]; then
  echo 'Output already exists; choose another label or remove your previous artifact.' >&2
  exit 1
fi
stage="$(mktemp -d)"
trap 'rm -rf -- "$stage"' EXIT
mkdir "$stage/$name"
install -m755 target/release/aircard target/release/aircard-gui "$stage/$name/"
install -m644 LICENSE README.md "$stage/$name/"
mkdir "$stage/$name/docs"
install -m644 docs/INSTALL.md docs/PROVENANCE.md docs/STATUS.md docs/SYNC-TOKEN.md docs/WALLET-TRANSACTIONS.md docs/AIRTRAFFIC-RESEARCH.md "$stage/$name/docs/"
python3 scripts/license-notices.py "$stage/$name/THIRD-PARTY-NOTICES.txt"
{
  git describe --always --dirty
  rustc --version
  uname -m
  if [[ -f /etc/os-release ]]; then cat /etc/os-release; fi
  pkg-config --modversion libimobiledevice-1.0 libplist-2.0 libusbmuxd-2.0
  for binary in aircard aircard-gui; do
    echo "$binary dependencies:"
    readelf -d "target/release/$binary" | sed -n '/NEEDED/p'
    echo "$binary glibc requirements:"
    readelf --version-info "target/release/$binary" | sed -n '/Name: GLIBC_/p'
  done
} > "$stage/$name/BUILD-INFO.txt"
# Stable archive ordering/time for a fixed set of input binaries and build metadata.
archive_epoch="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}"
tar --sort=name --mtime="@$archive_epoch" --owner=0 --group=0 --numeric-owner -C "$stage" -cf - "$name" | gzip -n > "$stage/$name.tar.gz"
mv -- "$stage/$name.tar.gz" "dist/$name.tar.gz"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
printf 'Created dist/%s.tar.gz\n' "$name"
