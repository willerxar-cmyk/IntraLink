"""Owner-assisted entire-display navigation acceptance on one authorized tablet.

The normal Android app requires fresh system consent; only the owner may
confirm it. ADB manipulates only the generated fixture and calculator after
the receiver presents the approved preview. No personal browsing, account,
file, notification or message inspection, screen recording, remote input
grant, or persistent test pairing. Requires an absent IntraLink test profile.
"""
import argparse
from pathlib import Path
import queue
import re
import secrets
import time
from verify_android_screen_physical import Device, PACKAGE, APPROVE, UI_PATH
from verify_android_screen_emulator import create_pair, field
from verify_android_lan_probe import Process, private_ip

FIXTURE = "org.intralink.navigationfixture"
VIDEO = FIXTURE + "/.MotionActivity"
READING = FIXTURE + "/.ReadingActivity"
CALCULATOR = "com.google.android.calculator/com.android.calculator2.Calculator"
SCOPE = "Share entire display across apps for up to 90 seconds (including visible notifications and private content). Stop from notification or Android system. Screen lock, rotation or task removal ends sharing."


def top_is(device, component):
    info = device.shell("dumpsys", "activity", "activities")
    # Accept either compact or full component notation.
    pkg, cls = component.split("/", 1)
    full = pkg + "/" + (pkg + cls if cls.startswith(".") else cls)
    return any("topResumedActivity=" in line and (component in line or full in line)
               for line in info.splitlines())


def start(device, component):
    if not device.projection_active():
        raise RuntimeError("Sharing stopped before navigation")
    # CLEAR_TOP applies only to the independent test fixture task. Android
    # otherwise may bring its last Activity (reading) forward instead of video.
    flags = ("-f", "0x04000000") if component.startswith(FIXTURE + "/") else ()
    result = device.shell("am", "start", "-W", *flags, "-n", component)
    if "Error" in result:
        raise RuntimeError("Test Activity launch failed")
    deadline = time.monotonic() + 4
    while time.monotonic() < deadline and device.projection_active():
        if top_is(device, component):
            return
        time.sleep(0.15)
    raise RuntimeError("Could not establish expected test Activity")


def samples(line):
    if line.startswith("NAVIGATION_SAMPLE "):
        return {k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", line)}
    return None


def phase(device, viewer, label, component, seconds, action=None):
    start(device, component)
    # Discard buffered samples from the preceding phase, never errors.
    while True:
        try:
            pending = viewer.lines.get_nowait()
        except queue.Empty:
            break
        if pending is None or pending.startswith(("Error:", "EMULATOR_VIEW_PASSED")):
            raise RuntimeError(f"Viewer ended before {label}")
    begin = time.monotonic()
    first = last = None
    count = 0
    next_check = begin
    next_action = begin + 1
    while time.monotonic() - begin < seconds:
        now = time.monotonic()
        if now >= next_check:
            if not device.projection_active() or not top_is(device, component):
                raise RuntimeError(f"Expected foreground and projection lost: {label}")
            next_check = now + 2
        if action and now >= next_action:
            count += int(action() or 0)
            next_action = time.monotonic() + 1
        try:
            line = viewer.lines.get(timeout=0.2)
        except queue.Empty:
            if viewer.proc.poll() is not None:
                raise RuntimeError(f"Viewer stopped during {label}")
            continue
        if line is None or line.startswith("Error:") or line.startswith("EMULATOR_VIEW_PASSED"):
            raise RuntimeError(f"Viewer ended during {label}: {line}")
        point = samples(line)
        if point:
            first = first or point
            last = point
    if not first or not last or last["ms"] <= first["ms"]:
        raise RuntimeError(f"No measured interval for {label}")
    span = last["ms"] - first["ms"]
    delta = {k: last[k] - first[k] for k in ("frames", "presented", "bytes", "jpeg", "tiles", "lossless", "decode_us")}
    if delta["presented"] <= 0:
        raise RuntimeError(f"No changed frames presented during {label}")
    print(f"PHASE {label} duration_ms={span} actions={count} " +
          " ".join(f"{k}={v}" for k,v in delta.items()), flush=True)


def fixture_log(device, tag):
    pid = device.shell("pidof", FIXTURE).strip()
    if not re.fullmatch(r"\d+", pid):
        raise RuntimeError("No isolated fixture process")
    return device.shell("logcat", "-d", "--pid", pid, "-s", tag + ":I", "*:S")


def motion_verified(device):
    pid = device.shell("pidof", FIXTURE).strip()
    if not re.fullmatch(r"\d+", pid):
        raise RuntimeError("No isolated video test process")
    log = device.shell("logcat", "-d", "-v", "epoch", "--pid", pid, "-s", "IntraLinkMotion:I", "*:S")
    if "PREPARED 1920x1080 duration_ms=20000" not in log or "VIDEO_RENDERING_STARTED" not in log:
        raise RuntimeError("Native video playback not verified")
    positions = [(float(t), int(pos)) for t, pos in re.findall(
        r"(?m)^\s*(\d+\.\d+)\s+[^\n]*?PLAYBACK_POSITION (\d+)", log)]
    if len(positions) < 3 or len({pos for _, pos in positions}) < 3:
        raise RuntimeError("Playback position is not advancing")
    print(f"NATIVE_VIDEO: 1920x1080 / 20s fixture prepared and rendering; playing_samples={len(positions)}", flush=True)
    return pid, positions


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--serial", required=True)
    p.add_argument("--android-ip", required=True)
    p.add_argument("--viewer", type=Path, required=True)
    p.add_argument("--port", type=int, default=45100)
    args = p.parse_args()
    address = private_ip(args.android_ip)
    if not 1024 <= args.port <= 65535 or not args.viewer.is_file():
        p.error("Explicit unprivileged port and viewer required")
    device = Device(args.serial, address)
    if not device.notifications():
        raise RuntimeError("Owner must enable IntraLink notifications before navigation test")
    if device.projection_active():
        raise RuntimeError("An existing projection must be stopped by owner")
    if "package:" + FIXTURE not in device.shell("pm", "list", "packages", FIXTURE):
        raise RuntimeError("Install the explicit offline fixture APK first")
    viewer = None
    password = secrets.token_hex(16)
    try:
        viewer = Process([str(args.viewer.resolve()), f"{address}:{args.port}", "--approved-navigation"])
        base = viewer.until(r"EMULATOR_BASE_ID ([a-zA-Z0-9]+)$", time.monotonic()+15, "Windows")[1]
        print("TEST_SETUP: pairing temporary Android/Windows identities", flush=True)
        peer = create_pair(device, password, base)
        field(device, "This device private IPv4 address", address)
        if args.port != 45100:
            raise RuntimeError("Use default port for this bounded acceptance")
        field(device, "Vault passphrase", password)
        device.click(SCOPE)
        if device.find(SCOPE, {PACKAGE}).get("checked") != "true":
            raise RuntimeError("Explicit navigation scope not selected")
        device.click(APPROVE)
        print("OWNER_CONSENT: accept entire-display sharing on the physical tablet now", flush=True)
        deadline = time.monotonic()+58
        while time.monotonic()<deadline:
            if device.projection_active() and device.foreground_preview():
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("Owner-authorized foreground projection not observed")
        print("OWNER_CONSENT_ACCEPTED", flush=True)
        viewer.proc.stdin.write(peer+"\n")
        viewer.proc.stdin.flush()
        viewer.until(r"NAVIGATION_READY$", time.monotonic()+8, "Windows")
        print("NAVIGATION_READY: first recognized preview presented", flush=True)
        # The whole display is now explicitly approved. Operate only known test
        # components, and never use an unrelated user's browser or account.
        phase(device, viewer, "video", VIDEO, 22)
        first_video_pid, first_video_positions = motion_verified(device)
        # wm size reports the unrotated physical panel. Use actual logical
        # display geometry for touch coordinates, including landscape.
        display = device.shell("dumpsys", "window", "displays")
        dims = re.search(r"DisplayFrames w=(\d+) h=(\d+)", display)
        if not dims:
            raise RuntimeError("Unknown current display geometry")
        width, height = map(int, dims.groups())
        def scroll():
            if not top_is(device, READING):
                raise RuntimeError("Unexpected scroll target")
            device.shell("input", "swipe", str(width//2), str(height*3//4),
                         str(width//2), str(height//4), "450")
            return True
        phase(device, viewer, "web_scroll", READING, 17, scroll)
        positions = [int(x) for x in re.findall(r"SCROLL_Y (\d+)", fixture_log(device, "IntraLinkReading"))]
        if not positions or max(positions) < 512:
            raise RuntimeError("Actual WebView scrolling not verified")
        print(f"WEB_SCROLL_VERIFIED: max_scroll_y={max(positions)} events={len(positions)}", flush=True)
        keys = ("KEYCODE_CLEAR", "KEYCODE_1", "KEYCODE_2", "KEYCODE_PLUS",
                "KEYCODE_3", "KEYCODE_4", "KEYCODE_EQUALS")
        sent = False
        def calculator():
            nonlocal sent
            if sent:
                return 0
            if not top_is(device, CALCULATOR):
                raise RuntimeError("Unexpected calculator target")
            # ADB startup latency varies. Dispatch the complete known
            # sequence in one local input command rather than timing out
            # between individual keys.
            device.shell("input", "keyevent", *keys)
            sent = True
            return len(keys)
        phase(device, viewer, "calculator", CALCULATOR, 13, calculator)
        # Inspect only the expected current result, not calculation history.
        root = device.ui()
        if not any(n.get("resource-id") == "com.google.android.calculator:id/result_final"
                   and n.get("text") == "46" for n in root.iter("node")):
            raise RuntimeError("Calculator result 12 + 34 = 46 not verified")
        print("CALCULATOR_VERIFIED: 12 + 34 = 46", flush=True)
        phase(device, viewer, "video_return", VIDEO, 14)
        video_pid, positions = motion_verified(device)
        # A stopped Activity may be resumed or its process may be recreated.
        # Verify fresh advancing native playback in either case by device
        # log timestamps, rather than requiring the earlier log prefix.
        resumed = [pos for timestamp, pos in positions if timestamp > first_video_positions[-1][0]]
        if len(resumed) < 3 or len(set(resumed)) < 3:
            raise RuntimeError("Native video playback did not advance after return")
        print(f"VIDEO_RETURN_VERIFIED: advancing_playback_samples={len(resumed)} process_recreated={video_pid != first_video_pid}", flush=True)
        final = viewer.until(r"NAVIGATION_FINAL (.+)$", time.monotonic()+45, "Windows")
        result = viewer.until(r"EMULATOR_VIEW_PASSED (.+)$", time.monotonic()+5, "Windows")
        if viewer.proc.wait(timeout=5) != 0:
            raise RuntimeError("Viewer exit failed")
        print("NAVIGATION_PASS: "+result[1]+" "+final[1], flush=True)
        deadline = time.monotonic()+10
        while time.monotonic()<deadline:
            if not device.projection_active() and "(nothing)" in device.shell("dumpsys", "activity", "services", PACKAGE):
                break
            time.sleep(0.3)
        else:
            raise RuntimeError("Screen resources not released")
        print("Projection and service closure: PASSED", flush=True)
    finally:
        if viewer:
            viewer.close()
        if device.created:
            device.shell("am", "force-stop", PACKAGE)
            device.shell("run-as", PACKAGE, "rm", "-f", "files/profile.vault", "files/profile.vault.lock")
        device.shell("am", "force-stop", FIXTURE)
        device.shell("rm", "-f", UI_PATH)
        del password


if __name__ == "__main__":
    main()
