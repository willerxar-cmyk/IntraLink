"""Generate a locked native dependency inventory and notices.

Requires the locked, already-resolved Cargo registry and GitHub CLI for exact
upstream notices absent from certain published crate archives. No package code
is executed, no credentials are written, and output must not already exist.
"""
import argparse
import base64
import json
import hashlib
import pathlib
import re
import subprocess
import zipfile

FEATURES = "intralink/wgc-preview,intralink/jpeg-simd,intralink/jpeg-parallel,intralink/resource-monitor"
PLATFORMS = {
    "windows": ("x86_64-pc-windows-msvc", FEATURES),
    "linux": ("x86_64-unknown-linux-gnu", FEATURES.replace("wgc-preview", "desktop-preview")),
    "android-arm64": ("aarch64-linux-android", ""),
    "android-x86_64": ("x86_64-linux-android", ""),
}
NOTICE = re.compile(r"^(licen[cs]es?|copying|copyright|notice|authors)([_.-]|$)", re.I)


def capture(*command):
    return subprocess.check_output(command, encoding="utf-8")


def upstream_notices(package, root):
    provenance = json.loads((root / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
    commit = provenance["git"]["sha1"]
    match = re.fullmatch(r"https://github\.com/([\w.-]+/[\w.-]+)(?:/tree/[\w./-]+)?/?", package["repository"] or "")
    if not match or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError(f"Missing exact upstream provenance: {package['name']}")
    repo = match.group(1)
    tree = json.loads(capture("gh", "api", f"repos/{repo}/git/trees/{commit}?recursive=1"))
    if tree.get("truncated"):
        raise ValueError(f"Incomplete upstream tree: {package['name']}")
    package_path = provenance.get("path_in_vcs", "").strip("/")
    result = {}
    for entry in tree["tree"]:
        name = entry["path"]
        if entry["type"] != "blob":
            continue
        # Include notices in root/package LICENSES directories and REUSE
        # metadata, retaining their exact upstream paths and copyright data.
        relative = name
        if package_path and name.startswith(package_path + "/"):
            relative = name[len(package_path) + 1:]
        first = relative.split("/", 1)[0]
        if NOTICE.match(first) or relative in (".reuse/dep5", "REUSE.toml"):
            blob = json.loads(capture("gh", "api", f"repos/{repo}/git/blobs/{entry['sha']}"))
            result[name] = base64.b64decode(blob["content"])
    return result, f"https://github.com/{repo}/tree/{commit}"


def reviewed_missing_notice(package, root):
    """Only exact, reviewed package omissions; never guess a copyright holder."""
    provenance = json.loads((root / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
    if (package["name"], package["version"], package["license"],
            provenance["git"]["sha1"]) != (
        "drm-fourcc", "2.2.0", "MIT", "7b9f80060ffd4b35cd236437e65a17fb517d163c"
    ):
        return {}, ""
    # The published crate and original tree contain no license file. The
    # upstream repository's later MIT permission notice is pinned by blob.
    # Preserve the original author/contributor declarations alongside it;
    # do not represent the later blob as present in the original commit.
    repo = "danielzfranklin/drm-fourcc-rs"
    sha = "31aa79387f27e730e33d871925e152e35e428031"
    blob = json.loads(capture("gh", "api", f"repos/{repo}/git/blobs/{sha}"))
    if blob["sha"] != sha:
        raise ValueError("Unexpected upstream MIT blob")
    license_text = base64.b64decode(blob["content"])
    git_blob = b"blob " + str(len(license_text)).encode() + b"\0" + license_text
    if hashlib.sha1(git_blob).hexdigest() != sha:
        raise ValueError("Invalid upstream MIT blob contents")
    files = {
        "Cargo.toml.orig": (root / "Cargo.toml.orig").read_bytes(),
        "README.md": (root / "README.md").read_bytes(),
        "LICENSE.upstream-later.txt": license_text,
        "NOTICE.txt": (
            "Original package and pinned commit declare MIT but contain no separate "
            "license file. A later upstream MIT permission notice is included "
            "unchanged, pinned by Git blob 31aa79387f27e730e33d871925e152e35e428031. "
            "Original author and contributor declarations are retained in the "
            "manifest and README. No copyright statement is invented. "
            "Original license provenance requires independent review.\n"
        ).encode(),
    }
    return files, f"https://github.com/{repo}/git/blobs/{sha} (later MIT notice)"


def has_notice(files):
    return any(NOTICE.match(part) for name in files for part in name.split("/"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--platform", choices=PLATFORMS, default="windows")
    parser.add_argument("--input-preview", action="store_true", help="Inventory the explicit Windows input build")
    parser.add_argument("--x11-input-preview", action="store_true", help="Inventory the Linux X11 receiving-host build")
    args = parser.parse_args()
    target, features = PLATFORMS[args.platform]
    if args.input_preview:
        if args.platform != "windows":
            parser.error("--input-preview requires Windows")
        features += ",intralink/input-preview"
    if args.x11_input_preview:
        if args.platform != "linux":
            parser.error("--x11-input-preview requires Linux")
        features += ",intralink/input-x11-preview"
    if args.output.exists():
        raise FileExistsError(args.output)
    package_name = "intralink-android" if args.platform.startswith("android-") else "intralink"
    flags = ["--locked", "--offline", "--no-default-features"]
    if features:
        flags += ["--features", features]
    host = next(line.removeprefix("host: ") for line in capture("rustc", "-vV").splitlines()
                if line.startswith("host: "))
    filters = ["--filter-platform", target]
    if host != target:
        filters += ["--filter-platform", host]
    metadata = json.loads(capture("cargo", "metadata", "--format-version", "1",
                                  *filters, *flags))
    tree = capture("cargo", "tree", "-p", package_name, "--target", target, "-e",
                   "normal,build", "--prefix", "none", "--format", "{p}",
                   "--charset", "ascii", *flags)
    selected = {tuple(m.groups()) for line in tree.splitlines()
                if (m := re.match(r"^([\w.-]+) v([^\s]+)", line))}
    packages = sorted((p for p in metadata["packages"]
                       if (p["name"], p["version"]) in selected and p.get("source")),
                      key=lambda p: (p["name"], p["version"]))
    actual = {(p["name"], p["version"]) for p in metadata["packages"]}
    if not selected.issubset(actual) or not packages:
        raise ValueError("Incomplete dependency inventory")
    notices = {}
    rows = []
    apache = None
    needs_apache_fallback = False
    for package in packages:
        review_required = False
        root = pathlib.Path(package["manifest_path"]).parent
        prefix = f"{package['name']}-{package['version']}"
        files = {}
        for entry in root.iterdir():
            if NOTICE.match(entry.name):
                if entry.is_file():
                    files[entry.name] = entry.read_bytes()
                elif entry.is_dir():
                    for item in entry.rglob("*"):
                        if item.is_file():
                            files[item.relative_to(root).as_posix()] = item.read_bytes()
        for name in (".reuse/dep5", "REUSE.toml"):
            entry = root / name
            if entry.is_file():
                files[name] = entry.read_bytes()
        explicit = package.get("license_file")
        if explicit:
            entry = root / explicit
            files[pathlib.Path(explicit).name] = entry.read_bytes()
        source = "Published Cargo package"
        if not explicit and not has_notice(files):
            recovered, source = upstream_notices(package, root)
            files.update(recovered)
        if not explicit and not has_notice(files):
            recovered, reviewed_source = reviewed_missing_notice(package, root)
            if recovered:
                files.update(recovered)
                source = reviewed_source
                review_required = True
        if not explicit and not has_notice(files):
            # enum-assoc publishes an explicit dual-license declaration, but
            # its exact upstream commit has no separate license/notice file.
            # Preserve original attribution and use the declared Apache option.
            if prefix != "enum-assoc-1.4.2" or package["license"] != "MIT OR Apache-2.0":
                raise ValueError(f"No notice material for {prefix}")
            needs_apache_fallback = True
            review_required = True
            files["Cargo.toml.orig"] = (root / "Cargo.toml.orig").read_bytes()
            files["README.md"] = (root / "README.md").read_bytes()
            files["NOTICE.txt"] = (
                "Upstream declares MIT OR Apache-2.0. No standalone license or NOTICE "
                "file was found in the exact package/source root. Apache-2.0 text "
                "is provided separately as the selected license option. Original "
                "package attribution is retained in Cargo.toml.orig.\n"
            ).encode()
        for name, content in files.items():
            notices[f"licenses/{prefix}/{name}"] = content
            if name == "LICENSE-APACHE" and b"Apache License" in content and apache is None:
                apache = content
        rows.append({
            "name": package["name"], "version": package["version"],
            "license": package.get("license"), "license_file": explicit,
            "source": package["source"], "repository": package.get("repository"),
            "notice_source": source, "notice_files": sorted(files),
            "review_required": review_required,
        })
    for row in rows:
        if row["name"] == "enum-assoc" and row["version"] == "1.4.2" and needs_apache_fallback:
            if apache is None:
                raise ValueError("Apache-2.0 text unavailable")
            notices["licenses/enum-assoc-1.4.2/APACHE-2.0.txt"] = apache
            row["notice_files"].append("APACHE-2.0.txt")
    if any(not row["license"] and not row["license_file"] for row in rows):
        raise ValueError("Undeclared license in resolved dependency graph")
    text = [
        f"IntraLink {args.platform} preview - native third-party inventory",
        f"Target: {target}", f"Build host: {host}", f"Cargo features: {features}",
        "Includes the resolved normal/build dependency graph; build-only packages "
        "may not be linked into the executable.",
        "License declarations and original notices are preserved; this is not a "
        "legal opinion, complete source audit, or proof of production compliance.",
        "",
    ]
    for row in rows:
        text.append(f"{row['name']} {row['version']} — {row['license'] or row['license_file']}")
        text.append(f"  {row['repository'] or row['source']}")
        text.append(f"  Notice source: {row['notice_source']}")
        if row["review_required"]:
            text.append("  Original notice provenance requires independent review.")
    with zipfile.ZipFile(args.output, "x", zipfile.ZIP_DEFLATED) as output:
        output.writestr("THIRD-PARTY-NOTICES.txt", "\n".join(text) + "\n")
        output.writestr("dependency-inventory.json", json.dumps({
            "target": target, "build_host": host, "features": features, "package": package_name, "scope": "normal and build dependencies",
            "packages": rows,
        }, indent=2) + "\n")
        for name, content in sorted(notices.items()):
            output.writestr(name, content)
    with zipfile.ZipFile(args.output) as output:
        if output.testzip() is not None:
            raise ValueError("Invalid notices archive")
    print(f"Inventory: {len(rows)} dependencies; {len(notices)} notice files; {args.output}")


if __name__ == "__main__":
    main()
