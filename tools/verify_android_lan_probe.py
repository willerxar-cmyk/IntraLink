#!/usr/bin/env python3
"""Run explicit Windows/Android private-LAN JNI/QUIC acceptance.

Requires the owner-selected ADB serial, already-installed compatible debug and
instrumentation APKs, and the freshly built Rust example. No user vault,
screen, input, firewall modification, credentials or persistent grant.
"""
import argparse
import ipaddress
import os
from pathlib import Path
import queue
import re
import subprocess
import threading
import time

RUNNER = "org.intralink.android.test/org.intralink.android.OnboardingInstrumentation"


class Process:
    def __init__(self, command):
        self.proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                                     errors="replace", bufsize=1)
        self.lines = queue.Queue()
        self.thread = threading.Thread(target=self.read, daemon=True)
        self.thread.start()

    def read(self):
        for line in self.proc.stdout:
            self.lines.put(line.rstrip())
        self.lines.put(None)

    def until(self, pattern, deadline, label):
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=max(0.01, deadline - time.monotonic()))
            except queue.Empty:
                break
            if line is None:
                raise RuntimeError(f"{label} exited before expected result (exit={self.proc.poll()})")
            match = re.search(pattern, line)
            if match:
                return match
            if "LAN_ANDROID_FAILED" in line:
                raise RuntimeError(line)
            if line.startswith("Error:") or "Error type" in line:
                print(f"{label}: {line}")
        raise TimeoutError(f"{label} deadline waiting for {pattern}")

    def close(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=5)
        for stream in (self.proc.stdin, self.proc.stdout):
            stream.close()


def private_ip(value):
    ip = ipaddress.IPv4Address(value)
    if not any(ip in ipaddress.IPv4Network(block) for block in
               ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")):
        raise ValueError("Use an explicit RFC1918 address")
    return str(ip)


def run_direction(adb, serial, executable, windows, android, windows_port, android_port, direction, media):
    role = "host" if direction == "outbound" else "base"
    command = [str(executable), role, f"{windows}:{windows_port}", f"{android}:{android_port}"]
    if media:
        command.append("media")
    wp = Process(command)
    ap = None
    try:
        wid = wp.until(r"LAN_WINDOWS_ID ([a-zA-Z0-9]+)$", time.monotonic() + 15, "Windows")[1]
        target = windows if direction == "outbound" else android
        port = windows_port if direction == "outbound" else android_port
        ap = Process([str(adb), "-s", serial, "shell", "am", "instrument", "-w", "-r",
                      "-e", "lanProbe", direction, "-e", "host", target,
                      "-e", "port", str(port), "-e", "peerId", wid,
                      "-e", "lanMediaFixture", "true" if media else "false", RUNNER])
        aid = ap.until(r"LAN_ANDROID_ID ([a-zA-Z0-9]+)$", time.monotonic() + 30, "Android")[1]
        wp.proc.stdin.write(aid + "\n")
        wp.proc.stdin.flush()
        deadline = time.monotonic() + 50
        detail = r" frames=(\d+) bytes=(\d+)" if media else ""
        a = ap.until(r"LAN_ANDROID_PASSED (inbound|outbound) (\d+)ms" + detail + r"; revocation and lease cleanup passed",
                     deadline, "Android")
        w = wp.until(r"LAN_WINDOWS_PASSED (host|base) (\d+)ms" + detail, deadline, "Windows")
        if a[1] != direction or w[1] != role:
            raise RuntimeError("Direction mismatch")
        if media:
            if a[3] != w[3] or a[4] != w[4] or int(a[3]) != 60 or int(a[4]) <= 0:
                raise RuntimeError("Media statistics mismatch")
            print(f"MEDIA PASS: {a[3]} 1920x1080 frames; {a[4]} encoded payload bytes; every RGB pixel verified", flush=True)
        if ap.proc.wait(timeout=5) != 0 or wp.proc.wait(timeout=5) != 0:
            raise RuntimeError("Nonzero process exit")
        print(f"PASS Android {direction}: Android session {a[2]} ms, Windows session {w[2]} ms; "
              "fresh pinned identities, actual JNI/QUIC, revocation and lease cleanup", flush=True)
    finally:
        if ap:
            ap.close()
        wp.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--windows-ip", required=True)
    parser.add_argument("--android-ip", required=True)
    parser.add_argument("--windows-port", type=int, required=True)
    parser.add_argument("--android-port", type=int, required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--direction", choices=("outbound", "inbound", "both"), default="both")
    parser.add_argument("--media", action="store_true", help="Requires explicit lan-fixture debug builds; generated pixels only")
    args = parser.parse_args()
    windows, android = private_ip(args.windows_ip), private_ip(args.android_ip)
    if windows == android or not all(1024 <= p <= 65535 for p in (args.windows_port, args.android_port)):
        parser.error("Distinct private addresses and unprivileged ports required")
    if args.serial.rsplit(":", 1)[0] != android:
        parser.error("ADB serial must match the selected Android IPv4 address")
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if not sdk and os.name == "nt":
        sdk = str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
    if not sdk:
        parser.error("Configure ANDROID_HOME")
    adb = Path(sdk) / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb")
    state = subprocess.check_output([str(adb), "-s", args.serial, "get-state"], text=True, timeout=10)
    if state.strip() != "device" or not args.executable.is_file():
        parser.error("Selected device and built example are required")
    for direction in (("outbound", "inbound") if args.direction == "both" else (args.direction,)):
        run_direction(adb, args.serial, args.executable.resolve(), windows, android,
                      args.windows_port, args.android_port, direction, args.media)


if __name__ == "__main__":
    main()
