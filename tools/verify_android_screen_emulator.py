"""Real system screen consent and v12 window acceptance on a disposable AVD.

Never use a physical device, personal profile or arbitrary screen source.
Only the named IntraLink AVD, its generated preview Activity and a local
loopback viewer are supported. No screenshots, recordings or persistent grant.
"""
import argparse
import ipaddress
from pathlib import Path
import re
import secrets
import time
from verify_android_capture_emulator import Emulator, PACKAGE, SYSTEM_PACKAGES
from verify_android_lan_probe import Process

APPROVE = "Approve peer and request screen consent"


def text(emu, value):
    emu.shell("input", "text", value)


def hide_keyboard(emu):
    emu.shell("input", "keyevent", "111")


def field(emu, label, value):
    # Preview.13 may prefill the exact active private route. Do not append
    # a second copy of an IP address to an already populated field.
    if label == "This device private IPv4 address":
        for node in emu.ui().iter("node"):
            if (node.get("package") == PACKAGE and node.get("class") == "android.widget.EditText"
                    and node.get("text") == value):
                return
    emu.click(label)
    text(emu, value)
    hide_keyboard(emu)


def create_pair(emu, password, base):
    emu.shell("am", "start", "-n", PACKAGE + "/.MainActivity")
    emu.find("How will you use this device?", {PACKAGE})
    emu.click("CLIENT - Receive control")
    emu.click("Device name")
    text(emu, "IntraLinkTest")
    emu.shell("input", "keyevent", "61")
    text(emu, password)
    emu.shell("input", "keyevent", "61")
    text(emu, password)
    hide_keyboard(emu)
    emu.click("Create local identity")
    emu.find("Public identity", {PACKAGE})
    ids = [n.get("text", "") for n in emu.ui().iter("node")
           if n.get("package") == PACKAGE and re.fullmatch(r"[0-9a-f]{64}", n.get("text", ""))]
    if len(ids) != 1:
        raise RuntimeError("Expected one local public identity")
    identity = ids[0]
    emu.click("Pair a BASE")
    field(emu, "Peer label (letters, digits, underscore or hyphen)", "local-base")
    field(emu, "Complete peer public identity", base)
    field(emu, "Vault passphrase", password)
    emu.click("Review pairing")
    emu.click("APPLY")
    emu.click("Share screen preview")
    return identity


def request(emu, password, guest, port, accept):
    emu.find("Share screen preview", {PACKAGE})
    # Set values before opening OS consent. No password is printed or saved.
    field(emu, "This device private IPv4 address", guest)
    emu.click("45100")
    emu.shell("input", "keyevent", "123")
    for _ in range(5):
        emu.shell("input", "keyevent", "67")
    text(emu, str(port))
    hide_keyboard(emu)
    field(emu, "Vault passphrase", password)
    emu.click(APPROVE)
    tree = emu.ui()
    if any(n.get("text") == "ALLOW" and n.get("package") in SYSTEM_PACKAGES
           for n in tree.iter("node")):
        emu.click("ALLOW", SYSTEM_PACKAGES)
        field(emu, "Vault passphrase", password)
        emu.click(APPROVE)
    emu.find("Share your screen with IntraLink?", SYSTEM_PACKAGES)
    emu.find("Share entire screen", SYSTEM_PACKAGES)
    emu.click("SHARE SCREEN" if accept else "CANCEL", SYSTEM_PACKAGES)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--avd", required=True)
    parser.add_argument("--apk", type=Path, required=True)
    parser.add_argument("--viewer", type=Path, required=True)
    parser.add_argument("--host-port", type=int, required=True)
    parser.add_argument("--guest-port", type=int, required=True)
    parser.add_argument("--guest-ip", required=True)
    parser.add_argument("--fresh-test-profile", action="store_true", required=True)
    args = parser.parse_args()
    emu = Emulator(args.serial, args.avd)
    guest = ipaddress.IPv4Address(args.guest_ip)
    if not guest.is_private or guest.is_loopback or str(guest) not in emu.shell("ip", "-4", "addr"):
        parser.error("Guest IP must be an assigned private AVD address")
    if not all(1024 <= n <= 65535 for n in (args.host_port, args.guest_port)):
        parser.error("Use unprivileged ports")
    if not args.apk.is_file() or not args.viewer.is_file():
        parser.error("Exact normal candidate APK and built test viewer required")
    if "no active redirections" not in emu.run("emu", "redir", "list").lower():
        parser.error("Preserve existing emulator redirections")
    forward = f"udp:{args.host_port}:{args.guest_port}"
    viewer = None
    added = False
    try:
        emu.run("install", "-r", str(args.apk.resolve()))
        # Verified disposable emulator only; reset the project's own test app.
        if emu.shell("pm", "clear", PACKAGE).strip() != "Success":
            raise RuntimeError("Could not reset dedicated test profile")
        viewer = Process([str(args.viewer.resolve()), f"127.0.0.1:{args.host_port}"])
        base = viewer.until(r"EMULATOR_BASE_ID ([a-zA-Z0-9]+)$", time.monotonic()+15, "Viewer")[1]
        password = secrets.token_hex(16)
        peer = create_pair(emu, password, base)
        request(emu, password, str(guest), args.guest_port, False)
        emu.find("Screen capture was not authorized.", {PACKAGE})
        emu.no_active_capture()
        print("Real OS denial and no active projection: PASSED", flush=True)
        # The previous form retains the route fields; clear the address and
        # reset the port before the next invocation.
        for label in (str(guest), str(args.guest_port)):
            node = emu.find(label, {PACKAGE})
            emu.tap_node(node)
            emu.shell("input", "keyevent", "123")
            for _ in range(len(label)):
                emu.shell("input", "keyevent", "67")
            if label == str(args.guest_port):
                text(emu, "45100")
            hide_keyboard(emu)
        if "OK" not in emu.run("emu", "redir", "add", forward):
            raise RuntimeError("Emulator UDP forward failed")
        added = True
        request(emu, password, str(guest), args.guest_port, True)
        del password
        deadline = time.monotonic()+8
        while time.monotonic() < deadline:
            projection = emu.shell("dumpsys", "media_projection")
            activity = emu.shell("dumpsys", "activity", "activities")
            if PACKAGE in projection and re.search(
                r"topResumedActivity=.*org\.intralink\.android/\.ScreenShareActivity", activity
            ):
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("No active authorized foreground projection")
        print("Real OS consent and foreground projection observed", flush=True)
        viewer.proc.stdin.write(peer+"\n")
        viewer.proc.stdin.flush()
        match = viewer.until(r"EMULATOR_VIEW_PASSED (.+)$", time.monotonic()+35, "Viewer")
        if viewer.proc.wait(timeout=5) != 0:
            raise RuntimeError("Viewer failed")
        print("Actual MediaProjection / v12 / Windows window: PASSED; "+match[1], flush=True)
        deadline = time.monotonic()+10
        while time.monotonic()<deadline:
            try:
                emu.no_active_capture()
                break
            except RuntimeError:
                time.sleep(0.5)
        else:
            raise RuntimeError("Projection/service not released")
        print("Projection and foreground service closure: PASSED", flush=True)
    finally:
        if viewer:
            viewer.close()
        if added:
            emu.run("emu", "redir", "del", f"udp:{args.host_port}")
        # Only the dedicated AVD app and our temporary XML are affected.
        emu.shell("am", "force-stop", PACKAGE)
        emu.shell("rm", "-f", "/data/local/tmp/intralink-acceptance.xml")


if __name__ == "__main__":
    main()
