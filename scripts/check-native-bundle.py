#!/usr/bin/env python3
"""Verify a release after extraction to a different directory, without an iPhone."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile


def verify(root):
    manifest = json.loads((root / "NATIVE-LIBRARIES.json").read_text())
    if not manifest:
        raise RuntimeError("Missing native-library manifest")
    for entry in manifest:
        path = root / "lib" / entry["library"]
        if hashlib.sha256(path.read_bytes()).hexdigest() != entry["bundled_sha256"]:
            raise RuntimeError(f"Native library checksum mismatch: {path.name}")
        notice = root / "docs/native-licenses" / f"{entry['package'].replace(':', '_')}.copyright"
        if not notice.is_file():
            raise RuntimeError(f"Native library notice missing: {notice.name}")
    sources = root / "native-sources"
    descriptors = list(sources.glob("*.dsc"))
    if not descriptors:
        raise RuntimeError("Native source descriptors missing")
    source_versions = set()
    for descriptor in descriptors:
        text = descriptor.read_text()
        source = re.search(r"^Source: (.+)$", text, re.MULTILINE)
        version = re.search(r"^Version: (.+)$", text, re.MULTILINE)
        if not source or not version:
            raise RuntimeError("Incomplete source descriptor")
        source_versions.add((source[1], version[1]))
        section = re.search(r"^Checksums-Sha256:\n((?: .+\n)+)", text, re.MULTILINE)
        if not section:
            raise RuntimeError("Source descriptor has no SHA256 checksums")
        for line in section[1].splitlines():
            digest, size, name = line.split()
            if Path(name).name != name:
                raise RuntimeError("Unexpected source archive path")
            data = (sources / name).read_bytes()
            if len(data) != int(size) or hashlib.sha256(data).hexdigest() != digest:
                raise RuntimeError(f"Source archive checksum mismatch: {name}")
    if not {(e["source_package"], e["source_version"]) for e in manifest} <= source_versions:
        raise RuntimeError("Bundled libraries lack their exact source versions")
    env = dict(os.environ, LC_ALL="C")
    for key in ["LD_LIBRARY_PATH", "LD_PRELOAD"]:
        env.pop(key, None)
    output = subprocess.check_output(["ldd", str(root / "aircard")], env=env, text=True)
    if "not found" in output:
        raise RuntimeError(output)
    resolved = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) >= 3 and fields[1] == "=>":
            resolved[fields[0]] = Path(fields[2]).resolve()
    for entry in manifest:
        name = entry["library"]
        if resolved.get(name) != (root / "lib" / name).resolve():
            raise RuntimeError(f"{name} was loaded from the host instead of the bundle")
    versions = []
    for binary in ["aircard", "aircard-gui"]:
        output = subprocess.check_output([str(root / binary), "--version"], env=env, text=True, timeout=5)
        versions.append(output.split()[-1])
    if versions[0] != versions[1]:
        raise RuntimeError("CLI/GUI versions differ")
    print(f"Relocated bundle {versions[0]}: native libraries, exact sources, notices and startup verified.")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="aircard-bundle-check-") as temporary:
        destination = Path(temporary)
        with tarfile.open(sys.argv[1]) as archive:
            archive.extractall(destination, filter="data")
        roots = list(destination.iterdir())
        if len(roots) != 1 or not roots[0].is_dir():
            raise SystemExit("Expected one release directory")
        verify(roots[0])
