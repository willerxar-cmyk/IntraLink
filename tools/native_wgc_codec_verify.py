#!/usr/bin/env python3
"""Run the real generated-window NVIDIA capture/codec/preview executable.

Require actual process exit zero as well as native count/teardown evidence.
No physical display sharing, remote address, or external codec is used.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time


def verify(exe, adapter, height, frames, present):
    command = [
        str(exe), "desktop-wgc-codec-check", "--height", str(height),
        "--samples", str(frames), "--adapter-index", str(adapter),
    ]
    if present:
        command.append("--present")
    started = time.perf_counter()
    child = subprocess.Popen(
        command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, encoding="utf-8", errors="replace",
    )
    try:
        output, _ = child.communicate(timeout=45)
    except subprocess.TimeoutExpired:
        child.kill()
        output, _ = child.communicate()
        return {"pid": child.pid, "height": height, "frames": frames,
                "present": present, "status": "timeout", "exit": child.returncode,
                "output": output}
    required = [
        f"nvdec_check=passed, submitted={frames}, decoded={frames}, displayed={frames}",
        "wgc_codec_check=passed", "same_device=true",
        "encoder_pixel_readback=0", "closed_capture_rejected=true",
        "selected_wgc_resources_released", "selected_d3d11_device_released",
        "selected_winrt_apartment_released",
    ]
    if present:
        required.extend([f"gpu_preview_check=passed, presented={frames}",
                         f"diagnostic_readback_bytes={frames * 16}", "gpu_preview=true"])
    status = "ok" if child.returncode == 0 and all(
        marker in output for marker in required
    ) else "failed"
    return {
        "pid": child.pid, "height": height, "frames": frames,
        "present": present, "status": status, "exit": child.returncode,
        "elapsed_s": round(time.perf_counter() - started, 3),
        "output": output,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--adapter-index", required=True, type=int)
    parser.add_argument("--present", action="store_true")
    parser.add_argument("--repeat-1080", action="store_true")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if not args.executable.is_file() or not 0 <= args.adapter_index < 64:
        parser.error("Specify an existing executable and current NVIDIA DXGI index")
    if args.output.exists():
        parser.error("Refusing to overwrite existing evidence")
    profiles = [(540, 60), (720, 60), (1080, 180)]
    if args.repeat_1080:
        profiles.append((1080, 180))
    results = []
    for height, frames in profiles:
        result = verify(args.executable.resolve(), args.adapter_index, height, frames, args.present)
        results.append(result)
        print(result["output"], end="", flush=True)
        print(f"pid={result['pid']} actual_exit={result['exit']} status={result['status']}", flush=True)
        if result["status"] != "ok":
            break
    with args.output.open("x", encoding="utf-8", newline="\n") as handle:
        json.dump(results, handle, indent=2)
        handle.write("\n")
    return 0 if len(results) == len(profiles) and all(r["status"] == "ok" for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
