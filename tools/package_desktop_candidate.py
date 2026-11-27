"""Package a verified Windows/Linux candidate, never publish automatically."""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tomllib
import zipfile

TARGETS = {
    "windows": "x86_64-pc-windows-msvc",
    "linux": "x86_64-unknown-linux-gnu",
}
DOCS = (
    "README.md", "CHANGELOG.md", "LICENSE", "docs/SECURITY.md",
    "docs/PLATFORMS.md", "docs/INPUT-SHARING.md", "docs/INPUT-QUEUE.md",
    "docs/DESKTOP.md", "docs/ENROLLMENT.md", "docs/WORKSPACE.md",
    "docs/RELEASE-PREVIEW.md", "docs/ONBOARDING.md",
)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def contents(args, revision):
    name = "intralink.exe" if args.platform == "windows" else "intralink"
    files = {name: args.binary.read_bytes()}
    if not files[name].startswith(b"MZ" if args.platform == "windows" else b"\x7fELF"):
        raise ValueError("Binary is not a native executable for this platform")
    for path in DOCS:
        files[path] = Path(path).read_bytes()
    with zipfile.ZipFile(args.notices) as notice:
        if notice.testzip() is not None:
            raise ValueError("Corrupt notice archive")
        for path in notice.namelist():
            item = PurePosixPath(path)
            if item.is_absolute() or ".." in item.parts or "\\" in path or path in files:
                raise ValueError(f"Unexpected notice archive path: {path}")
            files[path] = notice.read(path)
    inventory = json.loads(files["dependency-inventory.json"])
    if (inventory["target"] != TARGETS[args.platform]
            or inventory.get("build_host") != TARGETS[args.platform]
            or not inventory["packages"]):
        raise ValueError("Inventory target or package graph mismatch")
    required = ("intralink/jpeg-simd", "intralink/jpeg-parallel",
                "intralink/resource-monitor")
    features = set(inventory["features"].split(","))
    if not set(required) <= features:
        raise ValueError("Inventory is missing portable release features")
    platform_features = (
        {"intralink/wgc-preview", "intralink/input-preview"}
        if args.platform == "windows" else {"intralink/desktop-preview"}
    )
    if args.x11_input_preview:
        if args.platform != "linux":
            raise ValueError("X11 input is Linux-only")
        platform_features.add("intralink/input-x11-preview")
    if features != set(required) | platform_features:
        raise ValueError("Inventory features do not match the candidate")
    if not files["THIRD-PARTY-NOTICES.txt"]:
        raise ValueError("Missing original notices")
    note = [
        "IntraLink desktop build candidate",
        f"Source revision: {revision}",
        f"Target: {TARGETS[args.platform]}",
        "Not a production release. Native consent, capture and physical KVM",
        "must be qualified separately on real authorized devices.",
    ]
    review = [f"{row['name']} {row['version']}" for row in inventory["packages"]
              if row.get("review_required")]
    if review:
        note.append("Original notice provenance requires independent review: " + ", ".join(review))
    if args.platform == "linux":
        note.extend([
            "Run ./intralink --help in an X11 session with the required system",
            "libraries installed.",
            ("Opt-in X11 receiving host; no Linux controller or physical KVM qualification."
             if args.x11_input_preview else "Input KVM is not enabled in this Linux candidate."),
            "Wayland and physical Linux capture remain unqualified.",
        ])
        files["linux-runtime-libraries.txt"] = Path("linux-runtime-libraries.txt").read_bytes()
        if b"not found" in files["linux-runtime-libraries.txt"]:
            raise ValueError("Missing Linux runtime library")
    else:
        note.extend([
            "Run .\\intralink.exe --help. This is unsigned.",
            "The input preview is foreground-only and requires local consent.",
        ])
    files["CANDIDATE.txt"] = ("\n".join(note) + "\n").encode()
    files["SHA256SUMS.txt"] = "".join(
        f"{sha(data)}  {path}\n" for path, data in sorted(files.items())
    ).encode()
    return files, name


def write_archive(path, platform, files, binary):
    if platform == "windows":
        with zipfile.ZipFile(path, "x", zipfile.ZIP_DEFLATED) as archive:
            for name, data in sorted(files.items()):
                archive.writestr(name, data)
        with zipfile.ZipFile(path) as archive:
            if archive.testzip() is not None or set(archive.namelist()) != files.keys():
                raise ValueError("ZIP verification failed")
            for name, data in files.items():
                if archive.read(name) != data:
                    raise ValueError(f"ZIP entry differs: {name}")
    else:
        with tarfile.open(path, "x:gz") as archive:
            for name, data in sorted(files.items()):
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o755 if name == binary else 0o644
                archive.addfile(info, io.BytesIO(data))
        with tarfile.open(path, "r:gz") as archive:
            if set(archive.getnames()) != files.keys():
                raise ValueError("TAR verification failed")
            for name, data in files.items():
                stream = archive.extractfile(name)
                if stream is None or stream.read() != data:
                    raise ValueError(f"TAR entry differs: {name}")
            if archive.getmember(binary).mode != 0o755:
                raise ValueError("Linux executable mode missing")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", required=True, choices=TARGETS)
    parser.add_argument("--x11-input-preview", action="store_true")
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--notices", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    version = tomllib.loads(Path("Cargo.toml").read_text(encoding="utf-8"))[
        "workspace"
    ]["package"]["version"]
    files, binary = contents(args, revision)
    args.output.mkdir(parents=True, exist_ok=False)
    extension = "zip" if args.platform == "windows" else "tar.gz"
    variant = "-x11-input" if args.x11_input_preview else ""
    archive = args.output / (
        f"intralink-{version}-{args.platform}-x64{variant}-{revision[:12]}-candidate.{extension}"
    )
    write_archive(archive, args.platform, files, binary)
    checksum = sha(archive.read_bytes())
    (args.output / "SHA256SUMS.txt").write_text(
        f"{checksum}  {archive.name}\n", encoding="utf-8", newline="\n"
    )
    print(f"{archive}: {checksum}")


if __name__ == "__main__":
    main()
