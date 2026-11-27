"""Collect only the current generated Android AVC fixture and assess it offline.

The instrumentation owns the source pixels and emits a bounded local fixture.
ADB collection verifies the declared digest before any decoding. No projection,
arbitrary network AVC, general decoder worker, or production admission.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import time
from verify_android_lan_probe import RUNNER, private_ip

CACHE = "cache/intralink-generated-dense-avc.ilavc"
MAX = 16 * 1024 * 1024

def checked(command, source=None, timeout=40):
    result = subprocess.run(command, input=source, capture_output=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{Path(command[0]).name} failed: {result.stderr[-1200:]!r}")
    return result.stdout

def parse(data):
    if data[:8] != b"ILAVC001" or struct.unpack(">HHII", data[8:20]) != (1920,1080,60,6_000_000):
        raise ValueError("Wrong generated fixture header")
    pos = 20
    sps = pps = None
    pictures = []
    last_pts = -1
    ended = False
    while pos < len(data):
        kind, flags, reserved, length, pts = struct.unpack_from(">BBHIq", data, pos)
        pos += 16
        if reserved or flags > 1 or length > 2*1024*1024-8 or pos+length > len(data):
            raise ValueError("Invalid bounded record")
        value = data[pos:pos+length]
        pos += length
        if kind == 255:
            if flags or length or pts or pos != len(data):
                raise ValueError("Invalid terminator")
            ended = True
            break
        if not value:
            raise ValueError("Empty AVC record")
        if kind == 1 and sps is None and not pictures:
            sps = value
        elif kind == 2 and pps is None and sps is not None and not pictures:
            pps = value
        elif kind == 3 and sps is not None and pps is not None and not pictures:
            pass # MediaCodec may repeat configuration already in CSD.
        elif kind == 4 and sps is not None and pps is not None and pts > last_pts and len(pictures)<120:
            pictures.append((pts, flags, value))
            last_pts = pts
        else:
            raise ValueError("Unexpected record order")
    if not ended or not 100 <= len(pictures) <= 120 or not pictures[0][1]:
        raise ValueError("Incomplete generated AVC stream")
    return sps, pps, pictures

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--serial", required=True)
    p.add_argument("--admission", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    if ":" not in a.serial or private_ip(a.serial.rsplit(":",1)[0]) != a.serial.rsplit(":",1)[0]:
        p.error("Explicit private ADB Wi-Fi serial required")
    if a.output.exists() or not a.admission.is_file():
        p.error("New output path and built Rust admission example required")
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if not sdk and os.name == "nt":
        sdk = str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
    if not sdk:
        p.error("Configure Android SDK")
    adb = str(Path(sdk)/"platform-tools"/("adb.exe" if os.name=="nt" else "adb"))
    base = [adb, "-s", a.serial]
    if checked(base+["get-state"]).strip()!=b"device":
        raise RuntimeError("Selected device unavailable")
    ffmpeg, ffprobe = shutil.which("ffmpeg"), shutil.which("ffprobe")
    if not ffmpeg or not ffprobe:
        raise RuntimeError("Local FFmpeg tools required")
    present = subprocess.run(base+["shell","run-as","org.intralink.android","ls",CACHE],
                             capture_output=True,timeout=15)
    if present.returncode==0:
        raise RuntimeError("Preserve previous generated fixture; collect separately")
    report = checked(base+["shell","am","instrument","-w","-e","denseAvc","true",RUNNER],timeout=30).decode()
    match = re.search(r"DENSE_AVC_PASS codec=(\S+) submitted=(\d+) pictures=(\d+) idrs=(\d+) picture_bytes=(\d+) elapsed_ms=(\d+) target_bps=(\d+) file_bytes=(\d+) sha256=([0-9a-f]{64})",report)
    if not match:
        raise RuntimeError("Generated encoder fixture failed: "+report[-1600:])
    try:
        size = int(match[8])
        if not 36<=size<=MAX:
            raise ValueError("Invalid generated file budget")
        data = checked(base+["exec-out","run-as","org.intralink.android","cat",CACHE])
        if len(data)!=size or hashlib.sha256(data).hexdigest()!=match[9]:
            raise ValueError("Fixture digest differs from actual instrumentation")
        admission = checked([str(a.admission.resolve())],data).decode()
        sps,pps,pictures = parse(data)
        if len(pictures)!=int(match[3]) or sum(len(v) for _,_,v in pictures)!=int(match[5]):
            raise ValueError("Encoder counters differ")
        annex = sps+pps+b"".join(v for _,_,v in pictures)
        stream=json.loads(checked([ffprobe,"-v","error","-protocol_whitelist","pipe","-f","h264","-select_streams","v:0",
            "-show_entries","stream=codec_name,profile,width,height,pix_fmt,color_range,color_space,color_transfer,color_primaries,has_b_frames,level",
            "-of","json","pipe:0"],annex))["streams"][0]
        if (stream.get("width"),stream.get("height"),stream.get("pix_fmt"),stream.get("has_b_frames")) != (1920,1080,"yuv420p",0):
            raise ValueError("Unexpected decoded mode: "+str(stream))
        start=time.perf_counter()
        hashes=checked([ffmpeg,"-hide_banner","-loglevel","error","-xerror","-nostdin","-hwaccel","none","-c:v","h264","-threads","2",
            "-protocol_whitelist","pipe","-f","h264","-i","pipe:0","-an","-fps_mode","passthrough","-frames:v","121",
            "-pix_fmt","yuv420p","-f","framemd5","pipe:1"],annex).decode()
        elapsed=time.perf_counter()-start
        rows=[r.split(",") for r in hashes.splitlines() if r and not r.startswith("#")]
        if len(rows)!=len(pictures) or any(int(r[4])!=1920*1080*3//2 for r in rows):
            raise ValueError("CPU decoder output count/geometry mismatch")
        unique=len({r[-1].strip() for r in rows})
        if unique<100:
            raise ValueError("Insufficient unique decoded motion frames")
        result={"scope":"owner-selected device, generated pixels, local ADB and offline CPU decode only",
            "codec":match[1],"submitted":int(match[2]),"pictures":int(match[3]),"idrs":int(match[4]),
            "picture_bytes":int(match[5]),"elapsed_ms":int(match[6]),"target_bps":int(match[7]),
            "file_sha256":match[9],"file_bytes":size,"stream":stream,"strict_v9_assessment":admission.strip(),
            "pts_span_us":pictures[-1][0]-pictures[0][0],"decoded_frames":len(rows),
            "unique_decoded_frames":unique,"cpu_decode_and_hash_seconds":elapsed}
        a.output.mkdir(parents=True,exist_ok=False)
        (a.output/"generated-android-dense.ilavc").write_bytes(data)
        (a.output/"generated-android-dense.h264").write_bytes(annex)
        (a.output/"decoded.framemd5").write_text(hashes,encoding="utf-8")
        (a.output/"report.json").write_text(json.dumps(result,indent=2)+"\n",encoding="utf-8")
        print(json.dumps(result,indent=2))
    finally:
        checked(base+["shell","run-as","org.intralink.android","rm","-f",CACHE])
if __name__=="__main__":
    main()
