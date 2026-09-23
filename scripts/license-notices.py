#!/usr/bin/env python3
"""Collect upstream license texts from locked Cargo sources for native distribution."""
import json
import pathlib
import subprocess
import sys

metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--format-version", "1",
    "--filter-platform", subprocess.check_output(["rustc", "-vV"], text=True).split("host: ", 1)[1].splitlines()[0],
]))
workspace = set(metadata["workspace_members"])
parts = ["AirCard-Linux third-party notices\nGenerated from Cargo.lock.\n"]
for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    if package["id"] in workspace:
        continue
    root = pathlib.Path(package["manifest_path"]).parent
    parts.append(f'\n=== {package["name"]} {package["version"]} ===\nLicense: {package.get("license") or "see upstream files"}\nRepository: {package.get("repository") or "not specified"}\n')
    candidates = set()
    if package.get("license_file"):
        candidates.add(root / package["license_file"])
    for pattern in ["LICENSE*", "LICENCE*", "COPYING*", "NOTICE*", "UNLICENSE*", "licenses/*", "LICENSES/*", "fonts/*.txt"]:
        candidates.update(root.glob(pattern))
    for path in sorted(candidates):
        if path.is_file() and path.stat().st_size <= 1024 * 1024:
            parts.append(f'\n--- {path.relative_to(root)} ---\n{path.read_text(errors="replace")}\n')
parts.append("\n=== eframe/egui/emath/ecolor/epaint MIT license ===\n")
parts.append(pathlib.Path("docs/licenses/egui-MIT.txt").read_text())
pathlib.Path(sys.argv[1]).write_text("".join(parts))
