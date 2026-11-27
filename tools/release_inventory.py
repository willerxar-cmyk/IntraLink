"""Generate the Windows portable release dependency inventory and notices.

Requires the locked, already-resolved Cargo registry and GitHub CLI for exact
upstream notices absent from certain published crate archives. No package code
is executed, no credentials are written, and output must not already exist.
"""
import argparse
import base64
import json
import pathlib
import re
import subprocess
import zipfile

FEATURES = "intralink/wgc-preview,intralink/jpeg-simd,intralink/jpeg-parallel,intralink/resource-monitor"
TARGET = "x86_64-pc-windows-msvc"
NOTICE = re.compile(r"^(licen[cs]es?|copying|copyright|notice|authors)([_.-]|$)", re.I)


def capture(*command):
    return subprocess.check_output(command, encoding="utf-8")


def upstream_notices(package, root):
    provenance = json.loads((root / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
    commit = provenance["git"]["sha1"]
    match = re.fullmatch(r"https://github\.com/([\w.-]+/[\w.-]+)", package["repository"])
    if not match or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError(f"Missing exact upstream provenance: {package['name']}")
    repo = match.group(1)
    tree = json.loads(capture("gh", "api", f"repos/{repo}/git/trees/{commit}"))
    result = {}
    for entry in tree["tree"]:
        name = entry["path"]
        if entry["type"] == "blob" and NOTICE.match(name):
            blob = json.loads(capture("gh", "api", f"repos/{repo}/git/blobs/{entry['sha']}"))
            result[name] = base64.b64decode(blob["content"])
    return result, f"https://github.com/{repo}/tree/{commit}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    flags = ["--locked", "--offline", "--no-default-features", "--features", FEATURES]
    metadata = json.loads(capture("cargo", "metadata", "--format-version", "1",
                                  "--filter-platform", TARGET, *flags))
    tree = capture("cargo", "tree", "-p", "intralink", "--target", TARGET, "-e",
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
    for package in packages:
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
        explicit = package.get("license_file")
        if explicit:
            entry = root / explicit
            files[pathlib.Path(explicit).name] = entry.read_bytes()
        source = "Published Cargo package"
        if not files:
            files, source = upstream_notices(package, root)
        if not files:
            # enum-assoc publishes an explicit dual-license declaration, but
            # its exact upstream commit has no separate license/notice file.
            # Preserve original attribution and use the declared Apache option.
            if prefix != "enum-assoc-1.4.2" or package["license"] != "MIT OR Apache-2.0":
                raise ValueError(f"No notice material for {prefix}")
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
        })
    if apache is None:
        raise ValueError("Apache-2.0 text unavailable")
    notices["licenses/enum-assoc-1.4.2/APACHE-2.0.txt"] = apache
    for row in rows:
        if row["name"] == "enum-assoc" and row["version"] == "1.4.2":
            row["notice_files"].append("APACHE-2.0.txt")
    if any(not row["license"] and not row["license_file"] for row in rows):
        raise ValueError("Undeclared license in resolved dependency graph")
    text = [
        "IntraLink Windows x64 portable preview — third-party inventory",
        f"Target: {TARGET}", f"Cargo features: {FEATURES}",
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
    with zipfile.ZipFile(args.output, "x", zipfile.ZIP_DEFLATED) as output:
        output.writestr("THIRD-PARTY-NOTICES.txt", "\n".join(text) + "\n")
        output.writestr("dependency-inventory.json", json.dumps({
            "target": TARGET, "features": FEATURES, "scope": "normal and build dependencies",
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
