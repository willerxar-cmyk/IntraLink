"""Build the actual shared Rust Android JNI library for both supported ABIs."""
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess

NDK_VERSION = "28.2.13676358"
API = 26
TARGETS = {
    "arm64-v8a": ("aarch64-linux-android", "aarch64-linux-android"),
    "x86_64": ("x86_64-linux-android", "x86_64-linux-android"),
}


def ndk_bin():
    explicit = os.environ.get("ANDROID_NDK_HOME")
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if not sdk and platform.system() == "Windows":
        sdk = str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
    if not explicit and not sdk:
        raise RuntimeError("Configure ANDROID_HOME or ANDROID_NDK_HOME")
    ndk = Path(explicit) if explicit else Path(sdk) / "ndk" / NDK_VERSION
    host = {"Windows": "windows-x86_64", "Linux": "linux-x86_64",
            "Darwin": "darwin-x86_64"}.get(platform.system())
    if host is None:
        raise RuntimeError("Unsupported Android build host")
    result = ndk / "toolchains" / "llvm" / "prebuilt" / host / "bin"
    if not result.is_dir():
        raise RuntimeError(f"Missing Android NDK toolchain: {result}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--clippy", action="store_true", help="Run target-specific strict Clippy before the Release build")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")).resolve()
    bin_dir = ndk_bin()
    suffix = ".exe" if platform.system() == "Windows" else ""
    for abi, (target, clang_target) in TARGETS.items():
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(target_dir)
        cargo_target = target.upper().replace("-", "_")
        cc_target = target.replace("-", "_")
        clang = str(bin_dir / f"clang{suffix}")
        env[f"CARGO_TARGET_{cargo_target}_LINKER"] = clang
        env[f"CARGO_TARGET_{cargo_target}_RUSTFLAGS"] = f"-C link-arg=--target={clang_target}{API}"
        env[f"CC_{cc_target}"] = clang
        env[f"CXX_{cc_target}"] = str(bin_dir / f"clang++{suffix}")
        env[f"CFLAGS_{cc_target}"] = f"--target={clang_target}{API}"
        env[f"CXXFLAGS_{cc_target}"] = f"--target={clang_target}{API}"
        env[f"AR_{cc_target}"] = str(bin_dir / f"llvm-ar{suffix}")
        if args.clippy:
            subprocess.run(
                ["cargo", "clippy", "--locked", "--release", "-p", "intralink-android", "--target", target, "--", "-D", "warnings"],
                cwd=root, env=env, check=True,
            )
        subprocess.run(
            ["cargo", "build", "--locked", "--release", "-p", "intralink-android", "--target", target],
            cwd=root, env=env, check=True,
        )
        source = target_dir / target / "release" / "libintralink_android.so"
        if not source.is_file() or source.read_bytes()[:4] != b"\x7fELF":
            raise RuntimeError(f"Missing genuine Android native library: {source}")
        destination = args.output / abi / source.name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        print(f"Built {abi}: {destination} ({destination.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
