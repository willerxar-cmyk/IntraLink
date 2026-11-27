"""Owner-assisted physical Android screen preview, using the normal app UI.

This tool NEVER taps a system consent dialog, grants Android permissions,
clears application data or accesses a preexisting user vault. It requires a
fresh test-only profile and the owner physically approving capture. No
screenshots or raw media are retained. The Windows viewer checks the known
visible preview fixture; its output is aggregate telemetry only.
"""
import argparse
import os
from pathlib import Path
import re
import secrets
import subprocess
import time
import xml.etree.ElementTree as ET
from verify_android_capture_emulator import PACKAGE, SYSTEM_PACKAGES
from verify_android_lan_probe import Process, private_ip
from verify_android_screen_emulator import create_pair, field, hide_keyboard, APPROVE

UI_PATH = "/data/local/tmp/intralink-physical-acceptance.xml"


class Device:
    def __init__(self, serial, address):
        if not re.fullmatch(r"\d+\.\d+\.\d+\.\d+:\d+", serial):
            raise ValueError("Explicit physical ADB Wi-Fi serial required")
        if serial.rsplit(":", 1)[0] != address:
            raise ValueError("ADB address must equal selected Android address")
        sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
        if not sdk and os.name == "nt":
            sdk = str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
        if not sdk:
            raise ValueError("Configure ANDROID_HOME")
        self.adb = str(Path(sdk) / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb"))
        self.serial = serial
        if self.run("get-state").strip() != "device" or self.shell("getprop", "ro.kernel.qemu").strip() == "1":
            raise RuntimeError("Selected device is not the available physical tablet")
        # This check uses names only. Never open or replace a user's vault.
        existing = self.exec("shell", "run-as", PACKAGE, "ls", "-a", "files")
        if existing.returncode != 0 and "No such file or directory" not in existing.stderr:
            raise RuntimeError("Could not establish initial app data state")
        if existing.returncode == 0 and any("profile.vault" in name for name in existing.stdout.splitlines()):
            raise RuntimeError("An existing IntraLink profile must be preserved")
        self.created = False

    def exec(self, *args):
        return subprocess.run([self.adb, "-s", self.serial, *args], capture_output=True,
                              text=True, encoding="utf-8", errors="replace", timeout=20)

    def run(self, *args):
        result = self.exec(*args)
        if result.returncode != 0:
            raise RuntimeError(f"Selected ADB operation failed: {args[0]}")
        return result.stdout

    def shell(self, *args):
        return self.run("shell", *args)

    def ui(self):
        for _ in range(3):
            try:
                self.shell("uiautomator", "dump", UI_PATH)
                root = ET.fromstring(self.shell("cat", UI_PATH))
                return root
            except (ET.ParseError, RuntimeError):
                time.sleep(0.3)
        raise RuntimeError("Could not inspect test application UI")

    def find(self, label, packages={PACKAGE}, timeout=16):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            root = self.ui()
            nodes = [n for n in root.iter("node") if n.get("text") == label
                     and n.get("package") in packages]
            if len(nodes) == 1:
                return nodes[0]
            time.sleep(0.2)
        raise RuntimeError(f"Expected test UI element: {label}")

    def tap_node(self, node):
        # No physical system permission/MediaProjection clicks, even if
        # a future caller supplies a node from a system package.
        if node.get("package") != PACKAGE:
            raise RuntimeError("Only the owner may operate system consent")
        if node.get("enabled") != "true":
            raise RuntimeError("Test UI element not enabled")
        match = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
        if not match:
            raise RuntimeError("Invalid test UI bounds")
        left, top, right, bottom = map(int, match.groups())
        self.shell("input", "tap", str((left+right)//2), str((top+bottom)//2))

    def click(self, label, packages={PACKAGE}):
        if packages != {PACKAGE}:
            raise RuntimeError("System dialogs require the physical owner")
        self.tap_node(self.find(label, packages))
        if label == "Create local identity":
            self.created = True

    def notifications(self):
        info = self.shell("dumpsys", "package", PACKAGE)
        return bool(re.search(r"android\.permission\.POST_NOTIFICATIONS:\s*granted=true", info))

    def projection_active(self):
        info = self.shell("dumpsys", "media_projection")
        return PACKAGE in info

    def foreground_preview(self):
        info = self.shell("dumpsys", "activity", "activities")
        return bool(re.search(r"topResumedActivity=.*org\.intralink\.android/\.ScreenShareActivity", info))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--android-ip", required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--viewer", type=Path, required=True)
    args = parser.parse_args()
    address = private_ip(args.android_ip)
    if not 1024 <= args.port <= 65535 or not args.viewer.is_file():
        parser.error("Explicit unprivileged port and test viewer required")
    device = Device(args.serial, address)
    viewer = None
    password = secrets.token_hex(16)
    try:
        if device.projection_active():
            raise RuntimeError("Existing projection must be stopped by owner")
        viewer = Process([str(args.viewer.resolve()), f"{address}:{args.port}", "--approved-preview"])
        base = viewer.until(r"EMULATOR_BASE_ID ([a-zA-Z0-9]+)$", time.monotonic()+15, "Windows")[1]
        print("TEST_SETUP: temporary BASE created; configuring Android UI", flush=True)
        peer = create_pair(device, password, base)
        print("TEST_PAIR: actual UI pairing completed", flush=True)
        field(device, "This device private IPv4 address", address)
        if args.port != 45100:
            device.click("45100")
            device.shell("input", "keyevent", "123")
            for _ in range(5):
                device.shell("input", "keyevent", "67")
            device.shell("input", "text", str(args.port))
            hide_keyboard(device)
        field(device, "Vault passphrase", password)
        notifications = device.notifications()
        device.click(APPROVE)
        if not notifications:
            print("OWNER_NOTIFICATION: please allow IntraLink notifications on the tablet", flush=True)
            deadline = time.monotonic()+75
            while time.monotonic() < deadline and not device.notifications():
                time.sleep(0.5)
            if not device.notifications():
                raise RuntimeError("Owner did not grant notifications")
            time.sleep(0.6)
            field(device, "Vault passphrase", password)
            device.click(APPROVE)
        print("OWNER_CONSENT: please accept the real Android screen-sharing dialog now; remain on the preview", flush=True)
        deadline = time.monotonic()+58
        while time.monotonic()<deadline:
            if device.projection_active() and device.foreground_preview():
                break
            time.sleep(0.15)
        else:
            raise RuntimeError("No owner-authorized foreground projection observed")
        print("OWNER_CONSENT_ACCEPTED: Android system projection active", flush=True)
        viewer.proc.stdin.write(peer+"\n")
        viewer.proc.stdin.flush()
        result = viewer.until(r"EMULATOR_VIEW_PASSED (.+)$", time.monotonic()+35, "Windows")
        if viewer.proc.wait(timeout=5) != 0:
            raise RuntimeError("Viewer failed")
        print("PHYSICAL SCREEN PREVIEW PASS: "+result[1], flush=True)
        deadline = time.monotonic()+10
        while time.monotonic()<deadline:
            services = device.shell("dumpsys", "activity", "services", PACKAGE)
            if not device.projection_active() and "(nothing)" in services:
                break
            time.sleep(0.3)
        else:
            raise RuntimeError("Screen resources not released")
        print("Projection and foreground service closure: PASSED", flush=True)
    finally:
        if viewer:
            viewer.close()
        # Do not force-stop an app that might have contained an existing
        # profile. Only our newly created disposable test profile is affected.
        if device.created:
            device.shell("am", "force-stop", PACKAGE)
            # Preserve all other data. The specific profile did not exist
            # before the test and was generated exclusively for this run.
            device.shell("run-as", PACKAGE, "rm", "-f", "files/profile.vault", "files/profile.vault.lock")
        device.shell("rm", "-f", UI_PATH)
        del password


if __name__ == "__main__":
    main()
