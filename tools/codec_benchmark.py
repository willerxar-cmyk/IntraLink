#!/usr/bin/env python3
"""Measure local FFmpeg codecs on generated images, never on a user's screen."""
import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# Presets are not quality-matched. This is an availability and throughput probe,
# not a visual-quality comparison or an implementation of IntraLink's codec.
PROFILES = {
    "mjpeg": ("mjpeg", ["-q:v", "5", "-pix_fmt", "yuvj420p"]),
    "h264": ("libx264", ["-preset", "ultrafast", "-tune", "zerolatency", "-crf", "23", "-g", "30", "-bf", "0", "-pix_fmt", "yuv420p"]),
    "vp9": ("libvpx-vp9", ["-deadline", "realtime", "-cpu-used", "8", "-lag-in-frames", "0", "-b:v", "0", "-crf", "32", "-g", "30", "-pix_fmt", "yuv420p"]),
    "av1": ("libaom-av1", ["-usage", "realtime", "-cpu-used", "8", "-lag-in-frames", "0", "-b:v", "0", "-crf", "35", "-g", "30", "-pix_fmt", "yuv420p"]),
    "h264-nvenc": ("h264_nvenc", ["-preset", "p1", "-tune", "ull", "-g", "30", "-bf", "0", "-pix_fmt", "yuv420p"]),
    "h264-qsv": ("h264_qsv", ["-preset", "veryfast", "-async_depth", "1", "-g", "30", "-bf", "0", "-pix_fmt", "nv12"]),
    "h264-amf": ("h264_amf", ["-usage", "ultralowlatency", "-quality", "speed", "-g", "30", "-bf", "0", "-pix_fmt", "nv12"]),
}


def run(args, deadline=90):
    start = time.perf_counter()
    result = subprocess.run(args, stdin=subprocess.DEVNULL, capture_output=True,
                            text=True, encoding="utf-8", errors="replace", timeout=deadline, check=False)
    return result, time.perf_counter() - start


def benchmark(ffmpeg, ffprobe, name, source, output, frames, fps, threads, width, height):
    encoder, options = PROFILES[name]
    cmd = [ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
           "-threads", str(threads), "-i", str(source), "-an", "-frames:v", str(frames),
           "-c:v", encoder, "-threads", str(threads), *options, str(output)]
    result = {"profile": name, "encoder": encoder, "encoder_options": options}
    try:
        encoded, encode_s = run(cmd)
        if encoded.returncode != 0 or not output.exists():
            return {**result, "status": "encode_failed", "error": encoded.stderr[-1600:]}
        # Require actual decoding of generated media, not just encoder enumeration.
        decoded, decode_s = run([ffmpeg, "-hide_banner", "-loglevel", "error", "-xerror",
                                 "-nostdin", "-threads", str(threads), "-i", str(output),
                                 "-map", "0:v:0", "-an", "-f", "null", "-"])
        if decoded.returncode != 0:
            return {**result, "status": "decode_failed", "error": decoded.stderr[-1600:]}
        probe, _ = run([ffprobe, "-v", "error", "-count_frames", "-select_streams", "v:0",
                        "-show_entries", "stream=nb_read_frames,codec_name,width,height", "-of", "json", str(output)])
        if probe.returncode != 0:
            return {**result, "status": "probe_failed", "error": probe.stderr[-1600:]}
        stream = json.loads(probe.stdout)["streams"][0]
        if (int(stream.get("nb_read_frames", 0)) != frames
                or stream.get("width") != width or stream.get("height") != height):
            return {**result, "status": "stream_mismatch", "stream": stream}
        size = output.stat().st_size
        return {**result, "status": "ok", "stream": stream, "frames": frames,
                "container_bytes": size, "encode_wall_seconds": round(encode_s, 4),
                "decode_wall_seconds": round(decode_s, 4), "encode_wall_fps": round(frames / encode_s, 2),
                "decode_wall_fps": round(frames / decode_s, 2),
                "container_mbit_per_second": round(size * 8 * fps / frames / 1e6, 4)}
    except (subprocess.TimeoutExpired, ValueError, KeyError, IndexError, OSError) as error:
        return {**result, "status": "error", "error": str(error)[-1600:]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--height", type=int, choices=[540, 720, 1080], default=720)
    parser.add_argument("--fps", type=int, choices=[15, 30, 60], default=30)
    parser.add_argument("--seconds", type=int, choices=range(1, 6), default=2)
    parser.add_argument("--threads", type=int, choices=range(1, 9), default=2)
    parser.add_argument("--codecs", nargs="+", choices=PROFILES, default=["mjpeg", "h264", "vp9", "av1"])
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    ffmpeg, ffprobe = shutil.which("ffmpeg"), shutil.which("ffprobe")
    if not ffmpeg or not ffprobe:
        parser.error("ffmpeg and ffprobe are required on PATH")
    width = args.height * 16 // 9
    frames = args.fps * args.seconds
    version, _ = run([ffmpeg, "-version"])
    report = {"method": "synthetic testsrc2 motion; separate lossless source; process wall time includes startup and disk I/O; profiles not quality-matched; no network, native capture or display", "platform": platform.platform(), "machine": platform.machine(), "logical_cpus": os.cpu_count(), "ffmpeg": version.stdout.splitlines()[0], "width": width, "height": args.height, "fps": args.fps, "frames": frames, "threads": args.threads, "results": []}
    with tempfile.TemporaryDirectory(prefix="intralink-codec-") as temp:
        source = Path(temp) / "source.mkv"
        made, _ = run([ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-f", "lavfi", "-i", f"testsrc2=size={width}x{args.height}:rate={args.fps}", "-frames:v", str(frames), "-c:v", "ffv1", "-threads", str(args.threads), str(source)])
        if made.returncode != 0:
            parser.error(f"Cannot generate test source: {made.stderr[-1200:]}")
        for name in dict.fromkeys(args.codecs):
            report["results"].append(benchmark(ffmpeg, ffprobe, name, source, Path(temp) / f"{name}.mkv", frames, args.fps, args.threads, width, args.height))
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if all(row["status"] == "ok" for row in report["results"]) else 1


if __name__ == "__main__":
    sys.exit(main())
