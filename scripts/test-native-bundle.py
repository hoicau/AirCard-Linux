#!/usr/bin/env python3
"""Exercise real ELF loading with a device-library ABI absent from the host."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


bundler = module("bundle-native")
checker = module("check-native-bundle")
repositories = module("enable-source-repositories")


class BundleTest(unittest.TestCase):
    def test_missing_abi_and_transitive_dependencies_survive_relocation(self):
        with tempfile.TemporaryDirectory(prefix="aircard-elf-test-") as temporary:
            base = Path(temporary)
            upstream = base / "upstream"
            upstream.mkdir()
            stage = base / "release"
            stage.mkdir()
            plist = "libplist-2.0.so.987"
            mux = "libusbmuxd-2.0.so.987"
            mobile = "libimobiledevice-1.0.so.987"
            for name, code, dependencies in [
                (plist, "int plist_fixture(void) { return 1; }", []),
                (mux, "extern int plist_fixture(void); int mux_fixture(void) { return plist_fixture(); }", [plist]),
                (mobile, "extern int mux_fixture(void); int mobile_fixture(void) { return mux_fixture(); }", [mux]),
            ]:
                source = base / "fixture.c"
                source.write_text(code)
                subprocess.run(["cc", "-shared", "-fPIC", str(source), "-o", str(upstream / name),
                                f"-Wl,-soname,{name}", f"-L{upstream}",
                                *(f"-l:{dep}" for dep in dependencies)], check=True)
            source = base / "fixture.c"
            source.write_text('#include <stdio.h>\nextern int mobile_fixture(void); int main(void) { puts("aircard 0.0.0"); return mobile_fixture() != 1; }')
            subprocess.run(["cc", str(source), "-o", str(stage / "aircard"),
                            f"-L{upstream}", f"-Wl,-rpath-link,{upstream}",
                            f"-l:{mobile}"], check=True)
            shutil.copy2(stage / "aircard", stage / "aircard-gui")
            # GUI test fixture needs the same RUNPATH; real GUI has no device linkage.
            subprocess.run(["patchelf", "--set-rpath", "$ORIGIN/lib", str(stage / "aircard-gui")], check=True)
            failure = subprocess.run([str(stage / "aircard"), "--version"], capture_output=True)
            self.assertNotEqual(failure.returncode, 0)
            self.assertIn(b"error while loading shared libraries", failure.stderr)
            available = {name: upstream / name for name in [mobile, mux, plist]}
            copied = bundler.bundle(stage, available)
            self.assertEqual(set(copied), set(available))
            # Remove the build inputs so no accidental absolute run path can pass.
            shutil.rmtree(upstream)
            moved = base / "relocated"
            stage.rename(moved)
            bundler.verify(moved)
            licenses = moved / "docs/native-licenses"
            licenses.mkdir(parents=True)
            (licenses / "fixture.copyright").write_text("Synthetic test fixtures only\n")
            sources = moved / "native-sources"
            sources.mkdir()
            data = b"synthetic source archive"
            (sources / "fixture.tar.gz").write_bytes(data)
            (sources / "fixture.dsc").write_text(
                f"Source: fixture\nVersion: 1\nChecksums-Sha256:\n {hashlib.sha256(data).hexdigest()} {len(data)} fixture.tar.gz\n")
            manifest = [{"library": name, "package": "fixture", "source_package": "fixture",
                         "source_version": "1", "bundled_sha256": hashlib.sha256((moved / "lib" / name).read_bytes()).hexdigest()}
                        for name in copied]
            (moved / "NATIVE-LIBRARIES.json").write_text(json.dumps(manifest))
            checker.verify(moved)
            (sources / "fixture.tar.gz").write_bytes(b"changed")
            with self.assertRaisesRegex(RuntimeError, "Source archive checksum mismatch"):
                checker.verify(moved)
            (moved / "lib" / mux).unlink()
            with self.assertRaisesRegex(RuntimeError, "Unresolved"):
                bundler.verify(moved)

    def test_source_repository_setup_preserves_trust_and_is_idempotent(self):
        original = "Types: deb\nURIs: http://archive.ubuntu.com/ubuntu/\nSuites: noble\nComponents: main\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"
        expected = original.replace("Types: deb\n", "Types: deb deb-src\n")
        self.assertEqual(repositories.enable_sources(original), expected)
        self.assertEqual(repositories.enable_sources(expected), expected)
        with self.assertRaises(ValueError):
            repositories.enable_sources("# no repositories\n")


if __name__ == "__main__":
    unittest.main()
