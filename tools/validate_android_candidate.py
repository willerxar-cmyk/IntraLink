"""Validate and retain an Android diagnostic candidate, not a public release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import tempfile
import shutil
import struct
import subprocess
import xml.etree.ElementTree as ET
import zipfile

from build_android_native import ndk_bin

ABIS = {"arm64-v8a": 183, "x86_64": 62}
EXPORTS = {"createProfile", "openProfile", "changeRole", "renameDevice", "pairDevice", "revokeDevice",
           "initializeNetwork", "reserveProbe", "cancelProbe", "runProbe", "inspectAvc", "inspectAvcSps", "runScreen"}
PERMISSIONS = {
    "android.permission.INTERNET", "android.permission.ACCESS_NETWORK_STATE",
    "android.permission.FOREGROUND_SERVICE",
    "android.permission.FOREGROUND_SERVICE_MEDIA_PROJECTION",
    "android.permission.POST_NOTIFICATIONS",
}
ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "apps/android/app/build"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(*command):
    return subprocess.check_output([str(x) for x in command], encoding="utf-8", errors="replace", stderr=subprocess.STDOUT)


def sdk_tools():
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if not sdk and platform.system() == "Windows":
        sdk = str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
    if not sdk:
        raise RuntimeError("Configure ANDROID_HOME")
    base = Path(sdk) / "build-tools" / "36.0.0"
    suffix = ".exe" if platform.system() == "Windows" else ""
    return base / f"aapt{suffix}", base / f"zipalign{suffix}", base / "lib/apksigner.jar"


def verify_elf(data, abi):
    if data[:6] != b"\x7fELF\x02\x01":
        raise ValueError(f"Not a little-endian ELF64 library: {abi}")
    header = struct.unpack_from("<16sHHIQQQIHHHHHH", data)
    if header[1] != 3 or header[2] != ABIS[abi] or header[9] != 56:
        raise ValueError(f"Wrong shared-object architecture: {abi}")
    phoff, phnum = header[5], header[10]
    alignments = []
    for index in range(phnum):
        kind, _, offset, vaddr, _, file_size, _, align = struct.unpack_from(
            "<IIQQQQQQ", data, phoff + index * 56
        )
        if kind == 1:
            if align < 16384 or offset % align != vaddr % align or offset + file_size > len(data):
                raise ValueError(f"ELF load alignment is not 16 KiB compatible: {abi}")
            alignments.append(align)
    if not alignments:
        raise ValueError("No ELF load segment")
    return min(alignments)


def check_exports(path):
    suffix = ".exe" if platform.system() == "Windows" else ""
    symbols = run(ndk_bin() / f"llvm-readelf{suffix}", "--dyn-syms", "--wide", path)
    actual = set(re.findall(r"Java_org_intralink_android_NativeBridge_([A-Za-z0-9_]+)\b", symbols))
    if actual != EXPORTS:
        raise ValueError(f"Unexpected JNI export set: {path.name}: {actual}")
    if "Java_org_intralink_android_NativeFixtureBridge_" in symbols:
        raise ValueError(f"Instrumentation-only LAN fixture leaked into candidate: {path.name}")


def verify_archive(path, prefix):
    with zipfile.ZipFile(path) as z:
        if z.testzip() is not None or len(z.namelist()) != len(set(z.namelist())):
            raise ValueError(f"Invalid archive: {path.name}")
        expected = {f"{prefix}lib/{abi}/libintralink_android.so" for abi in ABIS}
        actual = {n for n in z.namelist() if n.startswith(prefix + "lib/") and n.endswith(".so")}
        if actual != expected:
            raise ValueError(f"Unexpected packaged native library set: {path.name}")
        for abi in ABIS:
            name = f"{prefix}lib/{abi}/libintralink_android.so"
            data = z.read(name)
            align = verify_elf(data, abi)
            with tempfile.TemporaryDirectory() as directory:
                library = Path(directory) / "libintralink_android.so"
                library.write_bytes(data)
                check_exports(library)
            print(f"{path.name}: {abi} {len(data)} bytes, ELF LOAD alignment {align}, SHA256 {sha(data)}")
        return any(n.startswith("META-INF/") and n.upper().endswith((".RSA", ".DSA", ".EC")) for n in z.namelist())


def verify_exports():
    for abi in ABIS:
        native = BUILD / "generated/jniLibs" / abi / "libintralink_android.so"
        verify_elf(native.read_bytes(), abi)
        check_exports(native)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--notices", nargs=3, required=True, type=Path)
    parser.add_argument("--require-signed", action="store_true")
    args = parser.parse_args()
    aapt, zipalign, apksigner = sdk_tools()
    gradle = (ROOT / "apps/android/app/build.gradle").read_text(encoding="utf-8")
    version_code = int(re.search(r"\bversionCode\s*=\s*(\d+)", gradle).group(1))
    version = re.search(r"\bversionName\s*=\s*'([^']+)'", gradle).group(1)
    apk_dir = BUILD / "outputs/apk"
    debug = apk_dir / "debug/app-debug.apk"
    test = apk_dir / "androidTest/debug/app-debug-androidTest.apk"
    signed_apk = apk_dir / "release/app-release.apk"
    unsigned_apk = apk_dir / "release/app-release-unsigned.apk"
    release = signed_apk if signed_apk.is_file() else unsigned_apk
    aab = BUILD / "outputs/bundle/release/app-release.aab"
    verify_exports()
    verify_archive(debug, "")
    verify_archive(release, "")
    bundle_signed = verify_archive(aab, "base/")
    # The known decoder fixture belongs only to the instrumentation APK.
    asset = "assets/headless-bands-1080p.h264"
    expected_fixture = "283ab72056ae1b9edf57a6bae51edd255db01ffd72593e1b0113e6914db0cda0"
    with zipfile.ZipFile(test) as archive:
        if archive.namelist().count(asset) != 1 or sha(archive.read(asset)) != expected_fixture:
            raise ValueError("Missing or changed instrumentation-only AVC fixture")
    for path in (debug, release, aab):
        with zipfile.ZipFile(path) as archive:
            if any(name.endswith("/headless-bands-1080p.h264") for name in archive.namelist()):
                raise ValueError("Test AVC fixture leaked into runtime package")
    for apk in (debug, release, test):
        run(zipalign, "-c", "-P", "16", "4", apk)
        badging = run(aapt, "dump", "badging", apk)
        if "org.intralink.android" not in badging:
            raise ValueError("Unexpected Android application identity")
        if apk != test:
            if f"versionCode='{version_code}'" not in badging or f"versionName='{version}'" not in badging or "sdkVersion:'26'" not in badging or "targetSdkVersion:'36'" not in badging:
                raise ValueError("Unexpected application version or SDK")
            permissions = set(re.findall(r"uses-permission:\s+name='([^']+)'", run(aapt, "dump", "permissions", apk)))
            if permissions != PERMISSIONS:
                raise ValueError(f"Unexpected Android permission set: {permissions}")
            xml = run(aapt, "dump", "xmltree", apk, "AndroidManifest.xml")
            for component, kind in ((".CaptureActivity", "activity"), (".CaptureService", "service"),
                                    (".ScreenShareActivity", "activity"), (".ScreenShareService", "service")):
                blocks = re.split(r"(?m)^\s*E: (?:activity|service|receiver|provider) \(line=\d+\)", xml)
                matching = [block for block in blocks
                            if re.search(r'android:name[^=]*="(?:org\.intralink\.android)?' + re.escape(component) + '"', block)]
                if len(matching) != 1:
                    raise ValueError(f"Missing or duplicate capture component: {component}")
                block = matching[0]
                if not re.search(r"android:exported[^=]*=\(type 0x12\)0x0(?:\s|$)", block):
                    raise ValueError(f"Capture {kind} is exported: {component}")
                if kind == "service" and not re.search(r"android:foregroundServiceType[^=]*=\(type 0x11\)0x20(?:\s|$)", block):
                    raise ValueError("Capture service is missing the exact mediaProjection type")
    run("java", "-jar", apksigner, "verify", "--verbose", debug)
    run("java", "-jar", apksigner, "verify", "--verbose", test)
    release_signed = subprocess.run(
        ["java", "-jar", str(apksigner), "verify", "--verbose", str(release)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
    ).returncode == 0
    if bundle_signed:
        run("jarsigner", "-verify", aab)
    if release_signed != bundle_signed:
        raise ValueError("Inconsistent Release APK/AAB signing")
    lint = ET.parse(BUILD / "reports/lint-results-debug.xml")
    issues = [(e.get("severity"), e.get("id")) for e in lint.findall("issue")]
    unexpected = [i for i in issues if i != ("Warning", "AndroidGradlePluginVersion")]
    if unexpected:
        raise ValueError(f"Unresolved Android lint diagnostics: {unexpected}")
    if args.require_signed and (not release_signed or issues):
        raise ValueError("Distribution gate requires authorized signatures and no lint diagnostics")
    for notice in args.notices:
        with zipfile.ZipFile(notice) as z:
            if z.testzip() is not None:
                raise ValueError("Invalid notice archive")
    args.output.mkdir(parents=True, exist_ok=False)
    files = {
        "intralink-android-capture-debug.apk": debug,
        "intralink-android-capture-instrumentation.apk": test,
        "intralink-android-capture-release" + (".apk" if release_signed else "-unsigned.apk"): release,
        "intralink-android-capture-release" + (".aab" if bundle_signed else "-unsigned.aab"): aab,
        "LICENSE": ROOT / "LICENSE",
        "ANDROID.md": ROOT / "docs/ANDROID.md",
    }
    for notice in args.notices:
        files[notice.name] = notice
    for name, source in files.items():
        shutil.copy2(source, args.output / name)
    info = {
        "version": version, "version_code": version_code,
        "scope": "encrypted onboarding, manual pairing, foreground private IPv4 diagnostics, local AVC diagnostic, experimental separately consented v12 read-only screen preview (Android 12L+); no remote input", "abis": list(ABIS),
        "permissions": sorted(PERMISSIONS),
        "debug_and_test_signed": True, "release_signed": release_signed,
        "lint_diagnostics": issues, "physical_runtime_qualified": False,
        "public_release": False,
    }
    (args.output / "CANDIDATE.json").write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
    rows = []
    for path in sorted(args.output.iterdir()):
        rows.append(f"{sha(path.read_bytes())}  {path.name}")
    (args.output / "SHA256SUMS.txt").write_text("\n".join(rows) + "\n", encoding="utf-8")
    print(json.dumps(info))
    print(f"Verified local Android candidate: {args.output}")


if __name__ == "__main__":
    main()
