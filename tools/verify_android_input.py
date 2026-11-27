"""Physical Windows BASE -> Android local app input acceptance.
No system input, accessibility, user credentials or personal screen capture.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import time
from verify_android_lan_probe import Process, RUNNER, private_ip

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--serial",required=True)
    p.add_argument("--android-ip",required=True)
    p.add_argument("--port",type=int,default=45234)
    p.add_argument("--controller",type=Path,required=True)
    a=p.parse_args()
    ip=private_ip(a.android_ip)
    if a.serial.rsplit(":",1)[0]!=ip or not 1024<=a.port<=65535 or not a.controller.is_file():
        p.error("Explicit private Android target and exact built controller required")
    sdk=os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if not sdk and os.name=="nt":
        sdk=str(Path(os.environ["LOCALAPPDATA"])/"Android"/"Sdk")
    if not sdk:
        p.error("Configure Android SDK")
    adb=str(Path(sdk)/"platform-tools"/("adb.exe" if os.name=="nt" else "adb"))
    if subprocess.check_output([adb,"-s",a.serial,"get-state"],text=True,timeout=10).strip()!="device":
        raise RuntimeError("Selected ADB device unavailable")
    controller=Process([str(a.controller.resolve()),f"{ip}:{a.port}"])
    android=None
    try:
        identity=controller.until(r"INPUT_WINDOWS_ID ([a-zA-Z0-9]+)$",time.monotonic()+15,"Windows")[1]
        android=Process([adb,"-s",a.serial,"shell","am","instrument","-w","-r",
            "-e","lanInput","true","-e","host",ip,"-e","port",str(a.port),"-e","peerId",identity,RUNNER])
        peer=android.until(r"INPUT_ANDROID_ID ([a-zA-Z0-9]+)$",time.monotonic()+30,"Android")[1]
        controller.proc.stdin.write(peer+"\n")
        controller.proc.stdin.flush()
        deadline=time.monotonic()+30
        sent=controller.until(r"INPUT_WINDOWS_PASSED records=(\d+) max_ack_us=(\d+)$",deadline,"Windows")
        received=android.until(r"INPUT_ANDROID_PASSED records=(\d+) moves=2 clicks=1 scroll=3 cursor=true text_verified=true released=true revoked=true",deadline,"Android")
        if sent[1]!=received[1] or sent[1]!="16":
            raise RuntimeError("Input record count mismatch")
        if controller.proc.wait(timeout=5)!=0 or android.proc.wait(timeout=5)!=0:
            raise RuntimeError("Nonzero acceptance exit")
        print(f"PHYSICAL_APP_INPUT_PASS records={sent[1]} max_ack_us={sent[2]} cursor=true text_verified=true released=true revoked=true")
    finally:
        if android: android.close()
        controller.close()

if __name__=="__main__":
    main()
