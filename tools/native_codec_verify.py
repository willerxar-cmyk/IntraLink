#!/usr/bin/env python3
"""Independently verify a generated IntraLink NVENC clip using external CUVID.

Optional development diagnostic; FFmpeg/ffprobe are not application dependencies.
Only creates owned temporary synthetic video files and an explicitly requested report.
"""
import argparse
import json
import pathlib
import shutil
import subprocess
import tempfile
import time

COLORS = [(16, 128, 128), (235, 128, 128), (63, 102, 240),
          (173, 42, 26), (32, 240, 118), (126, 128, 128)]


def run(args):
    result = subprocess.run(
        args, capture_output=True, text=True, encoding="utf-8",
        errors="replace", timeout=90, check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"{args[0]} exit {result.returncode}: {result.stderr[-2000:]}")
    return result.stdout


def verify(exe, ffmpeg, ffprobe, adapter, height, frames, directory):
    width = height * 16 // 9
    clip = directory / f"{height}.h264"
    raw = directory / f"{height}.nv12"
    native_log = run([
        str(exe), "desktop-nvenc-check", "--height", str(height),
        "--samples", str(frames), "--adapter-index", str(adapter),
        "--output", str(clip),
    ])
    stream = json.loads(run([
        ffprobe, "-v", "error", "-select_streams", "v:0", "-show_entries",
        "stream=codec_name,profile,width,height,pix_fmt,color_range,color_space,"
        "color_transfer,color_primaries,has_b_frames", "-of", "json", str(clip),
    ]))["streams"][0]
    expected = {
        "codec_name": "h264", "profile": "High", "width": width, "height": height,
        "pix_fmt": "yuv420p", "color_range": "tv", "color_space": "bt709",
        "color_transfer": "bt709", "color_primaries": "bt709", "has_b_frames": 0,
    }
    if any(stream.get(k) != v for k, v in expected.items()):
        raise RuntimeError(f"Unexpected encoded stream metadata: {stream}")
    start = time.perf_counter()
    run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-xerror", "-nostdin",
        "-hwaccel", "cuda", "-hwaccel_output_format", "cuda", "-c:v", "h264_cuvid",
        "-i", str(clip), "-an", "-vf", "hwdownload,format=nv12",
        "-fps_mode", "passthrough", "-f", "rawvideo", str(raw),
    ])
    elapsed = time.perf_counter() - start
    length = width * height * 3 // 2
    if raw.stat().st_size != frames * length:
        raise RuntimeError("Wrong decoded byte/frame count")
    max_error = 0
    with raw.open("rb") as handle:
        for phase in range(frames):
            data = handle.read(length)
            if len(data) != length:
                raise RuntimeError("Truncated decoded frame")
            for band in range(len(COLORS)):
                x = ((2 * band + 1) * width // (2 * len(COLORS))) & ~1
                for y in (height // 4, height // 2, height * 3 // 4):
                    offset = width * height + (y // 2) * width + x
                    actual = (data[y * width + x], data[offset], data[offset + 1])
                    reference = COLORS[(band + phase) % len(COLORS)]
                    error = max(abs(a - b) for a, b in zip(actual, reference))
                    max_error = max(max_error, error)
                    if error > 12:
                        raise RuntimeError(f"Color mismatch frame={phase} band={band}: {actual} != {reference}")
        if handle.read(1):
            raise RuntimeError("Unexpected trailing decoded bytes")
    return {
        "resolution": f"{width}x{height}", "native_encoder_stdout": native_log,
        "frames_verified": frames, "max_sampled_yuv_error": max_error,
        "encoded_bytes": clip.stat().st_size, "external_named_decoder": "h264_cuvid",
        "decode_wall_s": elapsed, "stream": stream,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=pathlib.Path, required=True)
    parser.add_argument("--adapter-index", type=int, required=True)
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()
    ffmpeg, ffprobe = shutil.which("ffmpeg"), shutil.which("ffprobe")
    if not ffmpeg or not ffprobe:
        parser.error("The optional diagnostic requires FFmpeg and ffprobe on PATH")
    if not args.executable.is_file() or not 0 <= args.adapter_index < 64:
        parser.error("Specify an existing NVENC-enabled IntraLink executable and current DXGI index")
    if args.output and args.output.exists():
        parser.error("Refusing to overwrite an existing report")
    with tempfile.TemporaryDirectory(prefix="intralink-native-codec-") as temp:
        report = [
            verify(args.executable.resolve(), ffmpeg, ffprobe, args.adapter_index,
                   height, frames, pathlib.Path(temp))
            for height, frames in ((540, 6), (1080, 60))
        ]
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        with args.output.open("x", encoding="utf-8", newline="\n") as handle:
            handle.write(rendered)
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
