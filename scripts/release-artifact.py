#!/usr/bin/env python3
"""Publish only the tested branch artifact for the exact tagged source commit."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tomllib


def api(path):
    return json.loads(subprocess.check_output(["gh", "api", path], text=True))


def find_run(repository, commit, request=api):
    response = request(f"repos/{repository}/actions/workflows/linux.yml/runs?head_sha={commit}&event=push&status=success&per_page=100")
    for run in response["workflow_runs"]:
        # PR merge builds and tag builds must never substitute for the branch commit.
        if run["head_sha"] != commit or run["event"] != "push" or run["conclusion"] != "success":
            continue
        artifacts = request(f"repos/{repository}/actions/runs/{run['id']}/artifacts?per_page=100")
        if any(a["name"] == "aircard-linux" and not a["expired"] for a in artifacts["artifacts"]):
            return run["id"]
    raise ValueError("No successful Linux branch build with an available artifact exists for this release commit. Wait for its branch CI to pass (or rerun that branch build if its artifact expired), then rerun this release workflow. No rebuild was started.")


def verify(directory, commit, tag, version):
    if tag.removeprefix("v") != version:
        raise ValueError(f"Release tag {tag!r} does not match source version {version!r}. Update Cargo.toml and Cargo.lock before tagging.")
    archives = list(directory.glob("*.tar.gz"))
    if not archives:
        raise ValueError("No release archives found")
    for archive in archives:
        checksum = archive.with_name(archive.name + ".sha256").read_text().split()
        if len(checksum) != 2 or checksum[1] != archive.name:
            raise ValueError("Unexpected archive checksum format")
        with archive.open("rb") as file:
            if hashlib.file_digest(file, "sha256").hexdigest() != checksum[0]:
                raise ValueError("Release archive checksum mismatch")
        with tarfile.open(archive) as bundle:
            members = [m for m in bundle.getmembers() if m.name.endswith("/BUILD-METADATA.json")]
            if len(members) != 1 or not members[0].isfile() or members[0].size > 4096:
                raise ValueError("Release build metadata missing or invalid; rebuild the branch with the current packaging script")
            metadata = json.load(bundle.extractfile(members[0]))
        if metadata != {"commit": commit, "version": version, "dirty": False}:
            raise ValueError("Artifact source commit, version or clean-tree status does not match the release")
    print(f"Verified {len(archives)} existing archive(s) for {tag} at {commit}; no rebuild required.")


if __name__ == "__main__":
    try:
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        if sys.argv[1] == "find":
            run_id = find_run(os.environ["RELEASE_REPOSITORY"], commit)
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                output.write(f"run_id={run_id}\n")
            print(f"Reusing successful Linux branch build {run_id} for {commit}.")
        elif sys.argv[1] == "verify":
            version = tomllib.loads(Path("Cargo.toml").read_text())["workspace"]["package"]["version"]
            verify(Path("dist"), commit, os.environ["RELEASE_TAG"], version)
        else:
            raise ValueError("Expected find or verify")
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
