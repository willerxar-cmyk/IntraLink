#!/usr/bin/env python3
"""Terminate only the decoder child of an owned generated-window diagnostic.

Requires psutil in the optional local validation environment. No real desktop
sharing, external process selection, or GPU/driver reset is performed.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import threading
import time

import psutil


def verify(exe, adapter):
    args = [str(exe), "desktop-wgc-live-check", "--height", "1080",
            "--samples", "180", "--adapter-index", str(adapter),
            "--isolated-decoder"]
    parent = subprocess.Popen(args, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True,
                              encoding="utf-8", errors="replace")
    output = queue.Queue(maxsize=256)
    child_pid = None
    captured = []

    def reader():
        for line in parent.stdout:
            output.put(line)
        output.put(None)

    thread = threading.Thread(target=reader, daemon=True)
    thread.start()
    try:
        deadline = time.monotonic() + 15
        while child_pid is None:
            line = output.get(timeout=max(0.001, deadline-time.monotonic()))
            if line is None:
                raise RuntimeError("Diagnostic ended before child announcement")
            captured.append(line)
            match = re.fullmatch(
                r"decoder_process_started, pid=(\d+), private_job=true\s*", line)
            if match:
                child_pid = int(match[1])
        child = psutil.Process(child_pid)
        if (child.ppid() != parent.pid
                or not os.path.samefile(child.exe(), exe)
                or child.cmdline()[1:] != ["desktop-decoder-worker"]):
            raise RuntimeError("Refusing to terminate a non-owned decoder process")
        # Give the native child time to start its generated-window pipeline.
        # The identity-bound psutil Process prevents PID reuse on kill.
        time.sleep(0.7)
        if parent.poll() is not None or not child.is_running():
            raise RuntimeError("Diagnostic ended before interruption")
        child.kill()
        child.wait(timeout=5)
        deadline = time.monotonic() + 15
        while True:
            line = output.get(timeout=max(0.001, deadline-time.monotonic()))
            if line is None:
                break
            captured.append(line)
        status = parent.wait(timeout=5)
        thread.join(timeout=2)
        text = "".join(captured)
        passed = (status != 0 and "wgc_live_loopback=passed" not in text
                  and "selected_wgc_resources_released" in text
                  and "selected_winrt_apartment_released" in text
                  and not child.is_running() and not thread.is_alive())
        return {"parent_pid": parent.pid, "decoder_pid": child_pid,
                "parent_exit": status, "passed": passed, "output": text}
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--adapter-index", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.executable.is_file() or not 0 <= args.adapter_index < 64:
        parser.error("Specify an existing executable and valid NVIDIA index")
    if args.output.exists():
        parser.error("Refusing to overwrite evidence")
    result = verify(args.executable.resolve(), args.adapter_index)
    print(result["output"], end="")
    print("decoder_fault_containment=" + ("passed" if result["passed"] else "failed"))
    with args.output.open("x", encoding="utf-8", newline="\n") as handle:
        json.dump(result, handle, indent=2)
        handle.write("\n")
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
