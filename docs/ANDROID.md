# Android pairing and diagnostic candidate

The Android module is a Kotlin application backed by the shared Rust encrypted vault and the existing authenticated QUIC probe protocol. Preview.5 source adds manual bilateral pairing, a local saved-peer dashboard, revocation, and explicitly authorized, foreground one-shot diagnostics. It is **not a screen or remote-input client yet**. The manifest requests only INTERNET and ACCESS_NETWORK_STATE for private-network transport and Android network configuration. No discovery, public relay, automatic connection, screen capture, accessibility service or background listener is enabled.

BASE/CLIENT setup recommends a role from connected input devices but grants no authority. App-private identity, role, name and directional grants use the existing Rust vault. Passwords are entered locally, copied to bounded byte arrays and cleared after use; the UI does not save password state. Screen capture of sensitive UI is blocked and cloud/device-transfer backups exclude app data. This is not a complete security audit.

## Pair and test a connection

1. Independently exchange the complete public identities shown by IntraLink on the two devices.
2. On the BASE, select Pair a CLIENT, enter a label, the verified CLIENT identity and its numeric private IPv4 address and UDP port. On the CLIENT, select Pair a BASE and enter the verified BASE identity and a label. A directional pairing does not imply reciprocal authorization.
3. Review the complete identity and route, enter the local passphrase and explicitly approve each side. There is no network traffic during pairing. Self-pairing, duplicate peer identities/labels, invalid role directions, public addresses, DNS names, loopback addresses and invalid ports are rejected by Rust for this Android UI.
4. On the CLIENT, open the saved BASE and select Accept a diagnostic. Enter the CLIENT device's private IPv4 address and UDP port, unlock the vault, then approve the single foreground session. On the BASE, select the saved CLIENT and Test connection; approve. Existing desktop listen/connect commands use the same intralink/probe/1 protocol and can be used on the other end. The listener waits up to 120 seconds; bind and exchange have separate bounded deadlines. The displayed elapsed time is diagnostic session duration, not input or network latency.
5. Stop diagnostic or leaving the resumed Activity cancels the reservation/session; the listener is closed and the vault lock released. No auto-reconnect. A process-wide unique reservation prevents concurrent Android probes and prevents stale cancellation from affecting a newer session. UI locks on return from background. Revocation removes the local grant only. Do not expose the listener publicly or change system firewall rules merely for testing.

Only manually supplied private IPv4 routes are accepted on Android. Desktop pairing may contain other routes; Android refuses to probe such a route. A new Android application context is installed once through the documented Iroh JNI bridge and retained for process lifetime. There are no new wire messages or privileges; diagnostic grants never authorize screen or input. The current Android connection implementation is a one-shot diagnostic, not persistent multi-host workspace or Hybrid multiplexing.
## Build

Use JDK 21, Gradle wrapper 9.3.1, AGP 9.1.0, Android SDK 36, build tools 36.0.0, NDK 28.2.13676358 and Rust 1.96.0 with targets aarch64-linux-android and x86_64-linux-android. Configure ANDROID_HOME and an isolated CARGO_TARGET_DIR if needed. From the repository root on Windows:

    python tools/build_android_native.py apps/android/app/build/generated/jniLibs --clippy
    apps\android\gradlew.bat -p apps/android :app:assembleDebug :app:assembleRelease :app:bundleRelease :app:assembleDebugAndroidTest :app:lintDebug :app:writeRuntimeDependencyInventory

On Linux use python3 and apps/android/gradlew. The Gradle variant API wires the actual native outputs into packaging. The build-only Android bridge is included in the locked Rust workspace; existing registry versions were not changed. Both ABIs use API 26 and NDK r28's 16 KiB-compatible ELF segments.

Release signing is optional for building candidates; never use the debug key as a release key. Configure all four environment variables with an existing authorized keystore and its real credentials: INTRALINK_ANDROID_KEYSTORE, INTRALINK_ANDROID_STORE_PASSWORD, INTRALINK_ANDROID_KEY_ALIAS, INTRALINK_ANDROID_KEY_PASSWORD. Partial configuration fails. Do not commit any secrets, paste passwords in logs, or overwrite an existing key. Without them, release APK and AAB remain unsigned.

Generate the exact native and JVM runtime notices from the resolved build:

    python tools/release_inventory.py dist/android-arm64-notices.zip --platform android-arm64
    python tools/release_inventory.py dist/android-x86_64-notices.zip --platform android-x86_64
    python tools/android_jvm_notices.py apps/android/app/build/reports/runtime-dependencies.json dist/android-jvm-notices.zip
    python -B tools/validate_android_candidate.py dist/android-pairing-candidate --notices dist/android-arm64-notices.zip dist/android-x86_64-notices.zip dist/android-jvm-notices.zip

The validator checks the archive contents, actual generated and packaged JNI export sets, ELF architecture and 16 KiB LOAD alignment, APK package/version, exact two network permissions, zipalign, test/debug signatures and release signature consistency. It retains hashes and the license inventories. The candidate gate records the remaining Gradle version advisory; no lint suppression. Use --require-signed for an authorized release candidate with no outstanding lint diagnostics; a native device acceptance gate and license review are still mandatory. The independent review markers in license manifests are not a declaration of legal compliance.

## Physical acceptance

Use an owner-authorized Android device or an emulator with a clean test profile. The debug and instrumentation APKs are for local testing only. The instrumentation runner creates disposable app-private vaults, exercises JNI identity creation, rejection of duplicate and wrong password operations, rename, role transition, reopening, manual pairing/revocation and cancellation before a diagnostic can start, then removes its test files. It does not open a real network listener. Run only after confirming the selected target:

    adb -s DEVICE_SERIAL install path/to/intralink-android-pairing-debug.apk
    adb -s DEVICE_SERIAL install path/to/intralink-android-pairing-instrumentation.apk
    adb -s DEVICE_SERIAL shell am instrument -w org.intralink.android.test/org.intralink.android.OnboardingInstrumentation

Require the PASSED marker and successful result. Separately exercise first-run UI, app stop/restart, device rotation, wrong passphrase, role change and rename on a disposable local profile. Verify no unexpected permission requests, access outside the app or automatic connections. With two distinct authorized devices on a private LAN, test actual Windows/Linux to Android and Android to Windows/Linux probe directions, pinned-ID mismatch, revoked pairing, unavailable route, Stop, activity pause and process exit. Inspect actual endpoint closure and network egress; do not infer physical acceptance from host loopback tests. Do not use a real user's vault for tests. Native tests compiled but not yet executed on Android.

## Next implementation

Qualify the manual pairing and one-shot authenticated transport on Android. Then implement signed offline invitation UI, independently authorized capability session management, MediaProjection/MediaCodec capture and receiving, foreground input sharing only through supported authorized APIs, and a dashboard of independently granted capabilities. Linux X11/Wayland input remains a separate native adapter project. No unattended control, permission bypass, or physical KVM qualification claim.
