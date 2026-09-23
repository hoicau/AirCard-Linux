#!/usr/bin/env python3
"""Bundle the matching device-library family, with exact Debian/Ubuntu source packages.

Only operates on a private packaging stage. Never patches installed or build-tree files.
The host still supplies glibc, the loader, TLS and desktop libraries, and usbmuxd.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess


DEVICE_LIBRARY = re.compile(r"lib(?:imobiledevice(?:-glue)?|usbmuxd|plist)-[0-9.]+\.so\.[0-9.]+")


def command(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def needed(path):
    return command("patchelf", "--print-needed", str(path)).splitlines()


def host_libraries():
    result = {}
    for line in command("ldconfig", "-p").splitlines():
        fields = line.split()
        if len(fields) >= 4 and DEVICE_LIBRARY.fullmatch(fields[0]):
            # Require one architecture per build environment; pkg-config below selects
            # the primary library directory over any multiarch cache entries.
            result.setdefault(fields[0], Path(fields[-1]))
    for pkg in ["libimobiledevice-1.0", "libusbmuxd-2.0", "libplist-2.0"]:
        directory = Path(command("pkg-config", "--variable=libdir", pkg))
        for path in directory.glob("*.so.*"):
            if DEVICE_LIBRARY.fullmatch(path.name):
                result[path.name] = path
    return result


def bundle(stage, available):
    """Copy direct and transitive device libraries and set relocatable RUNPATHs."""
    stage = stage.resolve()
    libdir = stage / "lib"
    libdir.mkdir()
    pending = needed(stage / "aircard")
    copied = {}
    while pending:
        name = pending.pop()
        if not DEVICE_LIBRARY.fullmatch(name) or name in copied:
            continue
        if name not in available:
            raise RuntimeError(f"Missing matching native library: {name}")
        source = available[name].resolve(strict=True)
        if command("patchelf", "--print-soname", str(source)) != name:
            raise RuntimeError(f"Native library SONAME mismatch: {name}")
        destination = libdir / name
        shutil.copy2(source, destination)
        destination.chmod(0o755)
        pending.extend(needed(destination))
        # RUNPATH on every object is necessary for indirect dependencies too.
        subprocess.run(["patchelf", "--set-rpath", "$ORIGIN", str(destination)], check=True)
        copied[name] = source
    if not all(any(name.startswith(prefix) for name in copied) for prefix in
               ["libimobiledevice-", "libusbmuxd-", "libplist-"]):
        raise RuntimeError("CLI does not contain the expected device-library dependencies")
    subprocess.run(["patchelf", "--set-rpath", "$ORIGIN/lib", str(stage / "aircard")], check=True)
    return copied


def package_for(path):
    # Ubuntu merged-/usr systems may register the pre-merge path in dpkg.
    candidates = [str(path)]
    if str(path).startswith("/usr/lib/"):
        candidates.append(str(path)[4:])
    for candidate in candidates:
        result = subprocess.run(["dpkg-query", "-S", candidate], text=True, capture_output=True)
        if result.returncode == 0:
            for line in result.stdout.splitlines():
                owner, _, installed = line.partition(": ")
                if installed == candidate:
                    return owner
    raise RuntimeError(f"Cannot identify the distribution package for {path}")


def provenance(stage, copied):
    licenses = stage / "docs/native-licenses"
    sources = stage / "native-sources"
    licenses.mkdir(parents=True)
    sources.mkdir()
    # Debian copyright files refer to these installed license texts.
    shutil.copytree("/usr/share/common-licenses", licenses / "common-licenses")
    packages = {}
    manifest = []
    for name, original in sorted(copied.items()):
        package = package_for(original)
        fields = command("dpkg-query", "-W", "-f=${Version}\t${source:Package}\t${source:Version}", package).split("\t")
        if len(fields) != 3 or not all(fields):
            raise RuntimeError(f"Missing exact source metadata for {package}")
        version, source_package, source_version = fields
        if package not in packages:
            copyright_file = Path("/usr/share/doc") / package.split(":")[0] / "copyright"
            shutil.copyfile(copyright_file, licenses / f"{package.replace(':', '_')}.copyright")
            packages[package] = (source_package, source_version)
        manifest.append({
            "library": name, "package": package, "version": version,
            "source_package": source_package, "source_version": source_version,
            "original_sha256": hashlib.sha256(original.read_bytes()).hexdigest(),
            "bundled_sha256": hashlib.sha256((stage / "lib" / name).read_bytes()).hexdigest(),
        })
    for package, version in sorted(set(packages.values())):
        # Source indexes must be enabled in CI. A missing exact source fails packaging.
        subprocess.run(["apt-get", "source", "--download-only", f"{package}={version}"],
                       cwd=sources, check=True)
    if not list(sources.glob("*.dsc")):
        raise RuntimeError("Native source packages were not downloaded")
    # apt download-only may leave its sandbox directory behind; ship files only.
    partial = sources / "partial"
    if partial.is_dir():
        partial.rmdir()
    for path in sources.iterdir():
        if not path.is_file() or not (path.name.endswith(".dsc") or ".tar." in path.name or path.name.endswith(".diff.gz")):
            raise RuntimeError(f"Unexpected source-package output: {path.name}")
    (stage / "NATIVE-LIBRARIES.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (sources / "README.txt").write_text(
        "Exact distribution sources for the bundled device libraries are included here.\n"
        "Use dpkg-source -x <package>.dsc to extract the source and distribution patches.\n"
        "Build using debian/rules and the Build-Depends recorded in each .dsc, on the\n"
        "distribution recorded in BUILD-INFO.txt (for example dpkg-buildpackage -b -uc -us).\n"
        "Libraries are dynamically linked and may be replaced with ABI-compatible builds.\n"
        "Packaging only changed ELF RUNPATH to $ORIGIN; scripts/bundle-native.py in the\n"
        "AirCard-Linux source implements that change with patchelf --set-rpath.\n"
        "Upstream copyright notices and license texts are in docs/native-licenses.\n"
    )


def verify(stage):
    """Fail if any device library resolves from the host or a dependency is missing."""
    env = dict(os.environ, LC_ALL="C")
    for key in ["LD_LIBRARY_PATH", "LD_PRELOAD"]:
        env.pop(key, None)
    output = command("ldd", str(stage / "aircard"), env=env)
    if "not found" in output:
        raise RuntimeError(f"Unresolved bundled CLI dependencies:\n{output}")
    resolved = set()
    for line in output.splitlines():
        fields = line.split()
        if fields and DEVICE_LIBRARY.fullmatch(fields[0]):
            if len(fields) < 3 or fields[1] != "=>" or Path(fields[2]).resolve().parent != (stage / "lib").resolve():
                raise RuntimeError(f"Device library escaped the bundle: {line.strip()}")
            resolved.add(fields[0])
    expected = {p.name for p in (stage / "lib").iterdir()}
    if resolved != expected:
        raise RuntimeError("Bundled device dependency set does not match loader resolution")
    command(str(stage / "aircard"), "--version", env=env)
    command(str(stage / "aircard-gui"), "--version", env=env)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", type=Path)
    args = parser.parse_args()
    stage = args.stage.resolve()
    copied = bundle(stage, host_libraries())
    provenance(stage, copied)
    verify(stage)
    print(f"Bundled and verified {len(copied)} matching device libraries with licenses and sources.")


if __name__ == "__main__":
    main()
