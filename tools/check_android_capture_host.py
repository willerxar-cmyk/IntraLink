"""Run the actual compiled Android pure-policy classes on the host JVM."""
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "apps/android/app/build"


def main():
    rows = json.loads((BUILD / "reports/runtime-dependencies.json").read_text(encoding="utf-8"))
    stdlib = [Path(row["artifact"]) for row in rows
              if row["group"] == "org.jetbrains.kotlin" and row["name"] == "kotlin-stdlib"]
    if len(stdlib) != 1:
        raise ValueError("Expected exact resolved Kotlin runtime")
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if not sdk and os.name == "nt":
        sdk = str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
    if not sdk:
        raise RuntimeError("Configure ANDROID_HOME")
    classes = BUILD / "intermediates/built_in_kotlinc/debug/compileDebugKotlin/classes"
    android = Path(sdk) / "platforms/android-36/android.jar"
    paths = [classes, stdlib[0], android]
    if not all(p.exists() for p in paths):
        raise FileNotFoundError("Build actual Kotlin classes and resolved runtime first")
    subprocess.run(["java", "--class-path", os.pathsep.join(map(str, paths)),
                    str(ROOT / "tools/AndroidCaptureHostCheck.java")], check=True)


if __name__ == "__main__":
    main()
