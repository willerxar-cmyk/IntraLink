"""Exercise the real Android consent UI on a dedicated disposable IntraLink AVD.

Never use this test with a physical device or a personal Android profile. It
expects a freshly installed app, does not reset app data, and captures only the
emulator's generated diagnostic fixture. No screenshots or encoded bytes are
exported. All consent actions go through the visible Android system UI.
"""
import argparse
import os
from pathlib import Path
import re
import secrets
import subprocess
import time
import xml.etree.ElementTree as ET

PACKAGE = "org.intralink.android"
UI_PATH = "/data/local/tmp/intralink-acceptance.xml"
SYSTEM_PACKAGES = {"com.android.systemui", "com.android.permissioncontroller",
                   "com.google.android.permissioncontroller"}


class Emulator:
    def __init__(self, serial, avd):
        if not re.fullmatch(r"emulator-\d+", serial) or not avd.startswith("IntraLink_"):
            raise ValueError("Select an explicit dedicated IntraLink emulator")
        sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
        if not sdk and os.name == "nt":
            sdk = str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
        if not sdk:
            raise ValueError("Configure ANDROID_HOME")
        self.adb = str(Path(sdk) / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb"))
        self.serial = serial
        name = self.run("emu", "avd", "name").splitlines()[0].strip()
        if name != avd or self.shell("getprop", "ro.kernel.qemu").strip() != "1":
            raise RuntimeError("Not the explicitly selected disposable emulator")
        if self.shell("getprop", "sys.boot_completed").strip() != "1":
            raise RuntimeError("Emulator is not booted")

    def run(self, *args):
        result = subprocess.run([self.adb, "-s", self.serial, *args],
                                capture_output=True, text=True, encoding="utf-8",
                                errors="replace", timeout=20)
        if result.returncode != 0:
            # Do not include command arguments: setup uses a disposable secret.
            raise RuntimeError(f"ADB operation failed: {args[0]}")
        return result.stdout

    def shell(self, *args):
        return self.run("shell", *args)

    def ui(self):
        for _ in range(3):
            try:
                self.shell("uiautomator", "dump", UI_PATH)
                return ET.fromstring(self.shell("cat", UI_PATH))
            except (ET.ParseError, RuntimeError):
                time.sleep(0.5)
        raise RuntimeError("Could not inspect the emulator UI")

    def find(self, label, packages, timeout=12):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            tree = self.ui()
            matches = [n for n in tree.iter("node")
                       if n.get("text") == label and n.get("package") in packages]
            if len(matches) == 1:
                return matches[0]
            time.sleep(0.25)
        raise RuntimeError(f"Expected visible UI element: {label}")

    def click(self, label, packages={PACKAGE}):
        node = self.find(label, packages)
        if node.get("enabled") != "true":
            raise RuntimeError(f"UI element is disabled: {label}")
        self.tap_node(node)

    def tap_node(self, node):
        match = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
        if not match:
            raise RuntimeError("Invalid UI bounds")
        left, top, right, bottom = map(int, match.groups())
        self.shell("input", "tap", str((left + right) // 2), str((top + bottom) // 2))

    def setup(self):
        self.shell("am", "start", "-n", PACKAGE + "/.MainActivity")
        self.find("How will you use this device?", {PACKAGE})
        self.click("CLIENT - Receive control")
        if self.find("CLIENT - Receive control", {PACKAGE}).get("checked") != "true":
            raise RuntimeError("CLIENT role not selected")
        self.click("Device name")
        self.shell("input", "text", "IntraLinkTest")
        secret = secrets.token_hex(16)
        self.shell("input", "keyevent", "61")
        self.shell("input", "text", secret)
        self.shell("input", "keyevent", "61")
        self.shell("input", "text", secret)
        del secret
        self.shell("input", "keyevent", "111")
        self.click("Create local identity")
        self.click("Advanced tools")
        self.click("Local screen encoder diagnostic")
        self.find("Ready for a new local consent request.", {PACKAGE})

    def consent(self, accept):
        self.click("Start local diagnostic")
        # A fresh API 33+ installation asks for notification permission first.
        tree = self.ui()
        if any(n.get("text") == "ALLOW" and n.get("package") in SYSTEM_PACKAGES
               for n in tree.iter("node")):
            self.click("ALLOW", SYSTEM_PACKAGES)
            self.click("Start local diagnostic")
        self.find("Share your screen with IntraLink?", SYSTEM_PACKAGES)
        self.find("Share entire screen", SYSTEM_PACKAGES)
        self.click("SHARE SCREEN" if accept else "CANCEL", SYSTEM_PACKAGES)

    def no_active_capture(self):
        projection = self.shell("dumpsys", "media_projection")
        services = self.shell("dumpsys", "activity", "services", PACKAGE)
        if not re.search(r"Media Projection:\s*null", projection) or "(nothing)" not in services:
            raise RuntimeError("Projection or foreground service still active")

    def metrics(self):
        tree = self.ui()
        texts = [n.get("text", "") for n in tree.iter("node") if n.get("package") == PACKAGE]
        metrics = next((t for t in texts if t.startswith("Encoder: ")), "")
        if not metrics:
            raise RuntimeError("No actual encoder metrics")
        print(metrics)
        return metrics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--avd", required=True)
    args = parser.parse_args()
    emu = Emulator(args.serial, args.avd)
    try:
        emu.setup()
        emu.consent(False)
        emu.find("Screen capture was not authorized.", {PACKAGE})
        emu.no_active_capture()
        print("Real system consent denial and no active projection: PASSED")
        emu.consent(True)
        emu.find("Local diagnostic completed.", {PACKAGE}, timeout=30)
        result = emu.metrics()
        pictures = re.search(r"AVC picture buffers: (\d+)", result)
        idrs = re.search(r"IDR: (\d+)", result)
        unparsed = re.search(r"Unparsed: (\d+)", result)
        if not (pictures and idrs and unparsed and int(pictures[1]) > 0 and
                int(idrs[1]) > 0 and int(unparsed[1]) == 0):
            raise RuntimeError("No fully observed AVC picture buffers or unsupported framing")
        geometry = re.search(r"Size: (\d+)\D+(\d+)", result)
        sps = re.search(r"SPS profile/level: (\d+)/(\d+)\D+(\d+)\D+(\d+)", result)
        csd = re.search(r"CSD mask: (\d+)\D+Format changes: (\d+)", result)
        pts = re.search(r"PTS delta min/max: (\d+)/(\d+)\D+Span: (\d+)", result)
        if not (geometry and sps and csd and pts
                and sps.group(3, 4) == geometry.group(1, 2)
                and int(sps[1]) in (66, 77, 100) and int(sps[2]) > 0
                and int(csd[1]) & 384 == 384 and int(csd[2]) >= 1
                and 0 < int(pts[1]) <= int(pts[2]) <= int(pts[3])):
            raise RuntimeError("Actual SPS/CSD or observed PTS geometry was not established")
        sync = re.search(r"Sync probe: (\d+)\D+Delay: (\d+) ms\D+PTS: (\d+)", result)
        if not (sync and int(sync[1]) == 2 and 0 <= int(sync[2]) < 2000 and int(sync[3]) > 0):
            raise RuntimeError("No fresh IDR observed after the local sync-frame request")
        emu.no_active_capture()
        print("Actual MediaProjection / MediaCodec / Rust AVC/SPS/PTS and sync-frame observation: PASSED")
        # The running fixture and 250 ms metric refresh may prevent
        # UiAutomator from reaching idle. Read the anchored bottom Stop button
        # bounds from the completed screen, then verify actual active OS
        # projection and the foreground capture Activity before tapping it.
        stop_node = emu.find("Stop capture", {PACKAGE})
        emu.consent(True)
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            projection = emu.shell("dumpsys", "media_projection")
            activity = emu.shell("dumpsys", "activity", "activities")
            if PACKAGE in projection and re.search(
                r"topResumedActivity=.*org\.intralink\.android/\.CaptureActivity", activity
            ):
                break
            time.sleep(0.25)
        else:
            raise RuntimeError("No actual active projection on the foreground fixture")
        emu.tap_node(stop_node)
        emu.find("Capture stopped.", {PACKAGE})
        emu.no_active_capture()
        print("Fresh consent, foreground Stop and resource closure: PASSED")
    finally:
        # The owned emulator may retain the test-only profile. Do not wipe it
        # or collect a screenshot. Remove only this script's temporary XML.
        emu.shell("rm", "-f", UI_PATH)


if __name__ == "__main__":
    main()
