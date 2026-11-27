#!/usr/bin/env python3
"""Decode the exact generated fixture and Rust-recovered stream on CPU.

Optional development gate, not an application decoder or a sandbox for arbitrary
input. The Rust example accepts no input and only emits the embedded fixture.
No display capture, network, GPU codec or persistent video file is involved.
"""
import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "crates/intralink-d3d11/examples/fixtures/headless-bands-1080p.h264"
SOURCE_SHA256 = "283ab72056ae1b9edf57a6bae51edd255db01ffd72593e1b0113e6914db0cda0"
WIDTH, HEIGHT = 1920, 1080
FRAME_BYTES = WIDTH * HEIGHT * 3 // 2
KEPT = [0, 3, 4, 5]
COLORS = [(16, 128, 128), (235, 128, 128), (63, 102, 240),
          (173, 42, 26), (32, 240, 118), (126, 128, 128)]


def checked(args, source=None, timeout=45):
    result = subprocess.run(args, input=source, capture_output=True,
                            timeout=timeout, check=False, cwd=ROOT)
    if result.returncode:
        raise RuntimeError(f"{args[0]} exit {result.returncode}: {result.stderr[-1500:]!r}")
    return result.stdout


def digest(data):
    return hashlib.sha256(data).hexdigest()


def decode(ffmpeg, data, frames):
    start = time.perf_counter()
    raw = checked([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-xerror", "-nostdin",
        "-hwaccel", "none", "-c:v", "h264", "-threads", "2",
        "-protocol_whitelist", "pipe", "-f", "h264", "-i", "pipe:0",
        "-an", "-fps_mode", "passthrough", "-frames:v", str(frames + 1),
        "-pix_fmt", "yuv420p", "-f", "rawvideo", "pipe:1",
    ], data)
    elapsed = time.perf_counter() - start
    if len(raw) != frames * FRAME_BYTES:
        raise ValueError(f"Wrong decoded length: {len(raw)}")
    return raw, elapsed


def check_color(data, original_phase):
    max_error = 0
    y_size = WIDTH * HEIGHT
    uv_size = y_size // 4
    for band in range(len(COLORS)):
        x = ((2 * band + 1) * WIDTH // (2 * len(COLORS))) & ~1
        for y in (HEIGHT // 4, HEIGHT // 2, HEIGHT * 3 // 4):
            uv_offset = (y // 2) * (WIDTH // 2) + x // 2
            actual = (data[y * WIDTH + x], data[y_size + uv_offset],
                      data[y_size + uv_size + uv_offset])
            expected = COLORS[(band + original_phase) % len(COLORS)]
            error = max(abs(a - b) for a, b in zip(actual, expected))
            if error > 12:
                raise ValueError(f"Color error at frame {original_phase}, band {band}")
            max_error = max(max_error, error)
    return max_error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()
    if args.output and args.output.exists():
        parser.error("Refusing to overwrite a previous report")
    ffmpeg = shutil.which("ffmpeg")
    ffprobe = shutil.which("ffprobe")
    cargo = shutil.which("cargo")
    if not all((ffmpeg, ffprobe, cargo)):
        parser.error("FFmpeg, ffprobe and Cargo must be available")
    source = FIXTURE.read_bytes()
    if digest(source) != SOURCE_SHA256:
        raise ValueError("Generated fixture hash mismatch")
    recovered = checked([cargo, "run", "--locked", "--quiet", "-p",
                         "intralink-core", "--example", "avc_recovery_fixture"])
    if not 0 < len(recovered) <= len(source) + 64:
        raise ValueError("Unexpected recovered fixture size")

    expected_metadata = {"codec_name": "h264", "profile": "High",
                         "width": WIDTH, "height": HEIGHT, "pix_fmt": "yuv420p",
                         "color_range": "tv", "color_space": "bt709",
                         "color_transfer": "bt709", "color_primaries": "bt709",
                         "has_b_frames": 0}
    for data in (source, recovered):
        stream = json.loads(checked([
            ffprobe, "-v", "error", "-protocol_whitelist", "pipe", "-f", "h264",
            "-select_streams", "v:0", "-show_entries",
            "stream=" + ",".join(expected_metadata), "-of", "json", "pipe:0",
        ], data))["streams"][0]
        if stream != expected_metadata:
            raise ValueError(f"Unexpected stream metadata: {stream}")

    original, original_s = decode(ffmpeg, source, 6)
    actual, recovered_s = decode(ffmpeg, recovered, len(KEPT))
    hashes = []
    max_error = 0
    for index, phase in enumerate(KEPT):
        before = original[phase * FRAME_BYTES:(phase + 1) * FRAME_BYTES]
        after = actual[index * FRAME_BYTES:(index + 1) * FRAME_BYTES]
        if before != after:
            raise ValueError(f"Recovery changed decoded pixels at original frame {phase}")
        max_error = max(max_error, check_color(after, phase))
        hashes.append(digest(after))
    report = {
        "scope": "known generated fixture, CPU decoder, no remote or display path",
        "ffmpeg": checked([ffmpeg, "-version"]).decode("utf-8", "replace").splitlines()[0],
        "decoder": "h264; hwaccel none; 2 configured threads",
        "source_sha256": SOURCE_SHA256, "source_bytes": len(source),
        "recovered_sha256": digest(recovered), "recovered_bytes": len(recovered),
        "original_frames": 6, "recovered_original_indices": KEPT,
        "exact_recovered_frame_hashes": hashes, "exact_decoded_bytes": len(actual),
        "max_sampled_yuv_error": max_error,
        "original_decode_wall_seconds": original_s,
        "recovered_decode_wall_seconds": recovered_s,
        "stream": expected_metadata,
    }
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        with args.output.open("x", encoding="utf-8", newline="\n") as handle:
            handle.write(rendered)
    print(rendered, end="")


if __name__ == "__main__":
    main()
