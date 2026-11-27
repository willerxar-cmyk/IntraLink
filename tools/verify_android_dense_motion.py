"""Compare the real portable encoder on a generated 210-frame Surface.
No projection, personal pixels, system permission or input. Uses the normal
JNI path and an explicitly pinned, short-lived private-LAN graphical viewer.
Both sender and viewer must agree on record counts and bytes.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import time
from verify_android_lan_probe import Process, private_ip, RUNNER

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--serial", required=True)
    p.add_argument("--android-ip", required=True)
    p.add_argument("--port", type=int, default=45232)
    p.add_argument("--viewer", type=Path, required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--quality", action="store_true", help="Require the generated-pixel quality mode")
    a = p.parse_args()
    ip = private_ip(a.android_ip)
    if a.serial.rsplit(":", 1)[0] != ip or not 1024 <= a.port <= 65535:
        p.error("Explicit selected device and private address required")
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if not sdk and os.name == "nt":
        sdk = str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
    if not sdk:
        p.error("Configure Android SDK")
    adb = str(Path(sdk) / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb"))
    if subprocess.check_output([adb, "-s", a.serial, "get-state"], text=True, timeout=10).strip() != "device":
        raise RuntimeError("Selected ADB device unavailable")
    viewer = Process([str(a.viewer.resolve()), f"{ip}:{a.port}", "--dense-motion" if a.quality else "--approved-navigation"])
    android = None
    try:
        wid = viewer.until(r"EMULATOR_BASE_ID ([a-zA-Z0-9]+)$", time.monotonic()+15, "Windows")[1]
        android = Process([adb, "-s", a.serial, "shell", "am", "instrument", "-w", "-r",
            "-e", "lanProbe", "inbound", "-e", "host", ip, "-e", "port", str(a.port),
            "-e", "peerId", wid, "-e", "lanMediaFixture", "dense", RUNNER])
        aid = android.until(r"LAN_ANDROID_ID ([a-zA-Z0-9]+)$", time.monotonic()+30, "Android")[1]
        viewer.proc.stdin.write(aid+"\n")
        viewer.proc.stdin.flush()
        deadline = time.monotonic()+100
        remote = android.until(r"LAN_ANDROID_PASSED inbound (\d+)ms frames=(\d+) bytes=(\d+); revocation and lease cleanup passed", deadline, "Android")
        quality = viewer.until(r"DENSE_QUALITY (.+)$", deadline, "Windows")[1] if a.quality else ""
        if a.quality:
            print(f"QUALITY_OBSERVED label={a.label} {quality}", flush=True)
        tags = viewer.until(r"NAVIGATION_FINAL (.+)$", deadline, "Windows")[1]
        local = viewer.until(r"EMULATOR_VIEW_PASSED (.+)$", deadline, "Windows")[1]
        stats = dict(re.findall(r"(\w+)=(\d+)", local))
        if remote[2] != "210" or stats.get("frames") != remote[2] or stats.get("bytes") != remote[3]:
            raise RuntimeError("Frame or byte count mismatch")
        if stats.get("presented") != "210" or stats.get("withheld") != "0" or stats.get("changed") != "210":
            raise RuntimeError("Incomplete generated fixture presentation")
        if android.proc.wait(timeout=5) != 0 or viewer.proc.wait(timeout=5) != 0:
            raise RuntimeError("Nonzero acceptance exit")
        print(f"DENSE_MOTION_PASS label={a.label} android_ms={remote[1]} {local} {tags} {quality}", flush=True)
    finally:
        if android:
            android.close()
        viewer.close()

if __name__ == "__main__":
    main()
