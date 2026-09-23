#!/usr/bin/env python3
"""Check all required native screenshot captures without third-party Python modules."""
import pathlib
import struct
import sys

root = pathlib.Path(sys.argv[1])
for name in ["artwork-light", "artwork-dark", "help-light", "device-light", "confirmation-dark", "compact-light", "listening-light", "no-card-dark", "device-compact-light", "save-locations-dark", "restore-light", "save-locations-compact"]:
    data = (root / f"{name}.png").read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", name
    width, height = struct.unpack(">II", data[16:24])
    assert width >= 680 and height >= 520 and len(data) > 5000, (name, width, height)
assert (root / "artwork-light.png").read_bytes() != (root / "artwork-dark.png").read_bytes()
print("Twelve native GUI views captured, including automatic save locations and restore/recovery guidance.")
