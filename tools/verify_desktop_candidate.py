"""Verify a downloaded desktop candidate without extracting or executing it."""
from __future__ import annotations

import argparse
import ast
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tarfile
import zipfile

from package_desktop_candidate import TARGETS

MAX_FILES = 4096
MAX_ENTRY_BYTES = 256 * 1024 * 1024
MAX_TOTAL_BYTES = 512 * 1024 * 1024
MAX_METADATA_BYTES = 4 * 1024 * 1024
HEX = re.compile(r"[0-9a-f]{64}")
REVISION = re.compile(r"[0-9a-f]{40}")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def valid_name(name):
    path = PurePosixPath(name)
    return (
        bool(path.parts) and not path.is_absolute() and path.as_posix() == name
        and all(part not in (".", "..") for part in path.parts)
        and "\\" not in name and ":" not in name
        and all(ord(char) >= 32 and ord(char) != 127 for char in name)
    )


def manifest(text):
    entries = {}
    for line in text.splitlines():
        digest, separator, name = line.partition("  ")
        require(separator and HEX.fullmatch(digest) and valid_name(name),
                "Invalid SHA-256 manifest entry")
        require(name not in entries, f"Duplicate manifest entry: {name}")
        entries[name] = digest
        require(len(entries) <= MAX_FILES, "Manifest entry limit exceeded")
    require(entries, "Empty manifest")
    return entries


def stream_hash(stream):
    digest = hashlib.sha256()
    while chunk := stream.read(1024 * 1024):
        digest.update(chunk)
    return digest.hexdigest()


def source_blob(revision, name):
    return subprocess.check_output(["git", "show", f"{revision}:{name}"])


def source_docs(revision):
    # Historical candidates carry the documentation list of their own source
    # commit, not the list from whichever newer checkout verifies them.
    tree = ast.parse(source_blob(revision, "tools/package_desktop_candidate.py"))
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id == "DOCS"
            for target in node.targets
        ):
            docs = ast.literal_eval(node.value)
            require(isinstance(docs, tuple) and 0 < len(docs) <= 100
                    and all(isinstance(name, str) and valid_name(name)
                            for name in docs), "Invalid source documentation list")
            return docs
    raise ValueError("Missing source documentation list")


def check_archive(archive, platform, revision, x11):
    binary = "intralink.exe" if platform == "windows" else "intralink"
    docs = source_docs(revision)
    required = set(docs) | {
        binary, "CANDIDATE.txt", "SHA256SUMS.txt",
        "dependency-inventory.json", "THIRD-PARTY-NOTICES.txt",
    }
    if platform == "linux":
        required.add("linux-runtime-libraries.txt")

    members = {}
    total = 0
    iterable = archive.infolist() if platform == "windows" else archive
    for item in iterable:
        if platform == "windows":
            name, size = item.filename, item.file_size
            mode = item.external_attr >> 16
            require(not item.is_dir() and not stat.S_ISLNK(mode),
                    f"Non-regular ZIP entry: {name}")
        else:
            name, size = item.name, item.size
            require(item.isfile(), f"Non-regular TAR entry: {name}")
            if name == binary:
                require(item.mode == 0o755, "Linux executable mode missing")
        require(valid_name(name) and name not in members,
                f"Unsafe or duplicate archive path: {name}")
        require(0 <= size <= MAX_ENTRY_BYTES, f"Entry too large: {name}")
        total += size
        require(total <= MAX_TOTAL_BYTES and len(members) < MAX_FILES,
                "Archive limits exceeded")
        members[name] = (item, size)
    require(required <= members.keys(), "Required package files missing")

    def open_entry(name):
        item, _ = members[name]
        return archive.open(item) if platform == "windows" else archive.extractfile(item)

    def metadata(name):
        require(members[name][1] <= MAX_METADATA_BYTES, f"Metadata too large: {name}")
        with open_entry(name) as stream:
            return stream.read(MAX_METADATA_BYTES + 1)

    entries = manifest(metadata("SHA256SUMS.txt").decode("utf-8"))
    require(entries.keys() == members.keys() - {"SHA256SUMS.txt"},
            "Archive manifest coverage differs from actual entries")
    for name, expected in entries.items():
        with open_entry(name) as stream:
            require(stream_hash(stream) == expected, f"SHA-256 mismatch: {name}")

    for name in docs:
        actual = metadata(name)
        expected = source_blob(revision, name)
        if platform == "windows":
            # Windows checkout may use core.autocrlf. Only newline
            # representation may differ, never actual document content.
            actual = actual.replace(b"\r\n", b"\n")
            expected = expected.replace(b"\r\n", b"\n")
        require(actual == expected, f"Source documentation mismatch: {name}")

    note = metadata("CANDIDATE.txt").decode("utf-8").splitlines()
    require([line for line in note if line.startswith("Source revision: ")]
            == [f"Source revision: {revision}"], "Source revision mismatch")
    require([line for line in note if line.startswith("Target: ")]
            == [f"Target: {TARGETS[platform]}"], "Candidate target mismatch")

    inventory = json.loads(metadata("dependency-inventory.json"))
    require(inventory.get("target") == TARGETS[platform]
            and inventory.get("build_host") == TARGETS[platform]
            and inventory.get("packages"), "Native dependency inventory mismatch")
    features = {"intralink/jpeg-simd", "intralink/jpeg-parallel",
                "intralink/resource-monitor"}
    if platform == "windows":
        features.update({"intralink/wgc-preview", "intralink/input-preview"})
    else:
        features.add("intralink/desktop-preview")
        if x11:
            features.add("intralink/input-x11-preview")
        require(b"not found" not in metadata("linux-runtime-libraries.txt"),
                "Missing Linux runtime library")
    require(set(inventory["features"].split(",")) == features,
            "Candidate feature graph mismatch")
    require(members["THIRD-PARTY-NOTICES.txt"][1] > 0, "Missing original notices")
    with open_entry(binary) as stream:
        require(stream.read(4).startswith(b"MZ" if platform == "windows" else b"\x7fELF"),
                "Invalid native executable magic")
    review = [f"{p['name']} {p['version']}" for p in inventory["packages"]
              if p.get("review_required")]
    return {
        "source_revision": revision,
        "target": TARGETS[platform],
        "verified_entries": len(entries),
        "archive_entries": len(members),
        "binary_bytes": members[binary][1],
        "binary_sha256": entries[binary],
        "dependency_packages": len(inventory["packages"]),
        "review_required": review,
    }


def verify(path, revision, platform, x11=False):
    require(REVISION.fullmatch(revision), "Use the exact 40-character Git commit")
    require(not x11 or platform == "linux", "X11 candidate is Linux-only")
    require(path.stat().st_size <= MAX_TOTAL_BYTES, "Candidate archive too large")
    expected_name = f"-{revision[:12]}-candidate." + (
        "zip" if platform == "windows" else "tar.gz")
    require(path.name.endswith(expected_name), "Archive filename revision mismatch")
    outer = path.with_name("SHA256SUMS.txt")
    require(outer.stat().st_size <= MAX_METADATA_BYTES, "Outer manifest too large")
    expected = manifest(outer.read_text(encoding="utf-8"))
    require(expected.keys() == {path.name}, "Unexpected outer manifest entries")
    with path.open("rb") as stream:
        digest = stream_hash(stream)
    require(expected[path.name] == digest, "Outer archive SHA-256 mismatch")
    opener = zipfile.ZipFile if platform == "windows" else tarfile.open
    mode = "r" if platform == "windows" else "r:gz"
    with opener(path, mode) as archive:
        result = check_archive(archive, platform, revision, x11)
    result["archive_sha256"] = digest
    result["archive_bytes"] = path.stat().st_size
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--platform", choices=TARGETS, required=True)
    parser.add_argument("--x11-input-preview", action="store_true")
    args = parser.parse_args()
    print(json.dumps(verify(args.archive, args.revision, args.platform,
                            args.x11_input_preview), indent=2))


if __name__ == "__main__":
    main()
