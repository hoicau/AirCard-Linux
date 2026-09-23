#!/usr/bin/env python3
"""Check release reuse and rejection of stale or untested artifacts without network access."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release", Path(__file__).with_name("release-artifact.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTest(unittest.TestCase):
    def test_only_successful_matching_push_with_unexpired_artifact_is_used(self):
        runs = [
            {"id": 1, "head_sha": "wrong", "event": "push", "conclusion": "success"},
            {"id": 2, "head_sha": "commit", "event": "pull_request", "conclusion": "success"},
            {"id": 3, "head_sha": "commit", "event": "push", "conclusion": "failure"},
            {"id": 4, "head_sha": "commit", "event": "push", "conclusion": "success"},
            {"id": 5, "head_sha": "commit", "event": "push", "conclusion": "success"},
        ]
        def api(path):
            if "/workflows/" in path:
                return {"workflow_runs": runs}
            self.assertTrue("/runs/4/" in path or "/runs/5/" in path)
            return {"artifacts": [{"name": "aircard-linux", "expired": "/runs/4/" in path}]}
        self.assertEqual(release.find_run("owner/repo", "commit", api), 5)
        runs.pop()
        with self.assertRaisesRegex(ValueError, "No successful"):
            release.find_run("owner/repo", "commit", api)

    def test_checksum_version_commit_and_dirty_tree_are_verified(self):
        with tempfile.TemporaryDirectory(prefix="aircard-release-test-") as temporary:
            directory = Path(temporary)
            archive = directory / "aircard-linux-0.1.2-x86_64.tar.gz"
            def package(commit="commit", version="0.1.2", dirty=False):
                data = json.dumps(dict(commit=commit, version=version, dirty=dirty)).encode()
                with tarfile.open(archive, "w:gz") as bundle:
                    entry = tarfile.TarInfo("release/BUILD-METADATA.json")
                    entry.size = len(data)
                    bundle.addfile(entry, io.BytesIO(data))
                archive.with_name(archive.name + ".sha256").write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
            package()
            for tag in ["0.1.2", "v0.1.2"]:
                release.verify(directory, "commit", tag, "0.1.2")
            with self.assertRaisesRegex(ValueError, "tag"):
                release.verify(directory, "commit", "0.1.1", "0.1.2")
            for options in [dict(commit="old"), dict(version="0.1.1"), dict(dirty=True)]:
                package(**options)
                with self.assertRaisesRegex(ValueError, "Artifact source"):
                    release.verify(directory, "commit", "0.1.2", "0.1.2")
            package()
            with archive.open("ab") as file:
                file.write(b"changed")
            with self.assertRaisesRegex(ValueError, "checksum"):
                release.verify(directory, "commit", "0.1.2", "0.1.2")


if __name__ == "__main__":
    unittest.main()
