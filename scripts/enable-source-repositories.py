#!/usr/bin/env python3
"""Enable deb-src alongside Ubuntu runner's existing official binary repositories."""
from pathlib import Path
import re

def enable_sources(text):
    count = 0
    def enable(match):
        nonlocal count
        kinds = match[1].split()
        if "deb" in kinds:
            count += 1
            if "deb-src" not in kinds:
                kinds.append("deb-src")
        return "Types: " + " ".join(kinds)
    # Preserve URIs, suites, components and Signed-By for the same repositories.
    updated = re.sub(r"^Types:[ \t]*(.+)$", enable, text, flags=re.MULTILINE)
    if count == 0:
        raise ValueError("No deb822 binary repositories found; cannot fetch matching sources")
    return updated


if __name__ == "__main__":
    # Do not enable source indexes for unrelated third-party runner repositories.
    path = Path("/etc/apt/sources.list.d/ubuntu.sources")
    path.write_text(enable_sources(path.read_text()))
