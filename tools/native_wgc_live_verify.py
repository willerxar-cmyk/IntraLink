#!/usr/bin/env python3
"""Require actual process exits for the continuous owned-window GPU/QUIC diagnostic."""
import argparse
import json
import re
from pathlib import Path
import subprocess
import time


def verify(exe, adapter, height, frames, cancel, isolated):
    command = [str(exe), "desktop-wgc-live-check", "--height", str(height),
               "--samples", str(frames), "--adapter-index", str(adapter)]
    if cancel is not None:
        command += ["--cancel-after", str(cancel)]
    if isolated:
        command.append("--isolated-decoder")
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
                "cancel_after": cancel, "status": "timeout",
                "exit": child.returncode, "output": output}
    if cancel is None:
        required = [
            f"wgc_stream_source=passed, frames={frames}",
            f"wgc_stream_viewer=passed, submitted={frames}",
            f"gpu_preview_check=passed, presented={frames}",
            f"nvdec_check=passed, submitted={frames}, decoded={frames}, displayed={frames}",
            "wgc_live_loopback=passed", "exact_native_origin=true",
            "authenticated_direct_quic=true", "independent_gpu_workers=true",
            "encoder_pixel_readback=0",
        ]
    else:
        required = [f"wgc_live_cancel=passed, requested={cancel}",
                    f"wgc_stream_viewer=cancelled, submitted={cancel}",
                    "wgc_stream_source=cancelled", "no_more_decode=true"]
    required += ["selected_wgc_resources_released",
                 "selected_d3d11_device_released",
                 "selected_winrt_apartment_released",
                 "worker_resources_released=true", "joined_workers=true"]
    child_pid = None
    if isolated:
        required.append("isolated_decoder=true")
        starts = re.findall(r"^decoder_process_started, pid=(\d+), private_job=true$", output, re.MULTILINE)
        exits = re.findall(r"^decoder_process_exit=passed, pid=(\d+), exit=0, ipc_joined=true$", output, re.MULTILINE)
        if len(starts) == 1 and starts == exits and int(starts[0]) != child.pid:
            child_pid = int(starts[0])
    status = "ok" if child.returncode == 0 and all(
        marker in output for marker in required
    ) and (not isolated or child_pid is not None) else "failed"
    return {"pid": child.pid, "height": height, "frames": frames,
            "cancel_after": cancel, "isolated_decoder": isolated, "decoder_pid": child_pid,
            "status": status, "exit": child.returncode,
            "elapsed_s": round(time.perf_counter() - started, 3), "output": output}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--adapter-index", required=True, type=int)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--isolated-decoder", action="store_true")
    args = parser.parse_args()
    if not args.executable.is_file() or not 0 <= args.adapter_index < 64:
        parser.error("Specify an existing executable and current NVIDIA DXGI index")
    if args.output.exists():
        parser.error("Refusing to overwrite existing evidence")
    profiles = [(540, 60, None), (720, 60, None), (1080, 180, None),
                (1080, 180, None), (540, 60, 1), (540, 60, 10),
                (1080, 180, 30)]
    results = []
    for height, frames, cancel in profiles:
        result = verify(args.executable.resolve(), args.adapter_index,
                        height, frames, cancel, args.isolated_decoder)
        results.append(result)
        print(result["output"], end="", flush=True)
        print(f"pid={result['pid']} actual_exit={result['exit']} status={result['status']}",
              flush=True)
        if result["status"] != "ok":
            break
    with args.output.open("x", encoding="utf-8", newline="\n") as handle:
        json.dump(results, handle, indent=2)
        handle.write("\n")
    return 0 if len(results) == len(profiles) and all(
        r["status"] == "ok" for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
