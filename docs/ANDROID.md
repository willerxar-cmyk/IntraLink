# Android local onboarding candidate

The Android module is a real Kotlin application backed by the shared Rust encrypted vault. It is **not a remote-control client yet**. The current manifest requests no network, accessibility, capture or foreground-service permissions. BASE/CLIENT selection recommends a role from connected input devices but grants no authority. App-private identity, role, name and compatible grants use the existing Rust vault. Passwords are entered locally, copied to bounded byte arrays and cleared after use; the UI does not save password state. Screen capture of sensitive UI is blocked, and cloud/device-transfer backups exclude app data. This is not a complete security audit.

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
    python -B tools/validate_android_candidate.py dist/android-onboarding-candidate --notices dist/android-arm64-notices.zip dist/android-x86_64-notices.zip dist/android-jvm-notices.zip

The validator checks the archive contents, actual JNI exports, ELF architecture and 16 KiB LOAD alignment, APK package/version, permission absence, zipalign, test/debug signatures and release signature consistency. It retains hashes and the license inventories. The candidate gate records the remaining Gradle version advisory; no lint suppression. Use --require-signed for an authorized release candidate with no outstanding lint diagnostics; a native device acceptance gate and license review are still mandatory. The independent review markers in license manifests are not a declaration of legal compliance.

## Physical acceptance

Use an owner-authorized Android device or an emulator with a clean test profile. The debug and instrumentation APKs are for local testing only. The instrumentation runner creates its own disposable app-private vault, exercises JNI identity creation, rejection of duplicate and wrong password operations, rename, role transition and reopening, then removes its test files. Run only after confirming the selected target:

    adb -s DEVICE_SERIAL install path/to/intralink-android-onboarding-debug.apk
    adb -s DEVICE_SERIAL install path/to/intralink-android-onboarding-instrumentation.apk
    adb -s DEVICE_SERIAL shell am instrument -w org.intralink.android.test/org.intralink.android.OnboardingInstrumentation

Require the PASSED marker and successful result. Separately exercise first-run UI, app stop/restart, device rotation, wrong passphrase, role change and rename on a disposable local profile. Verify no unexpected permission requests, access outside the app or automatic connections. Do not use a real user's vault for tests. Native tests compiled but not yet executed on Android.

## Next implementation

Implement explicit pairing and authenticated Rust transport integration, Android lifecycle-aware session management, visible consent and local stop, MediaProjection/MediaCodec capture and receiving, foreground input sharing only through supported authorized APIs, and a dashboard of independently granted capabilities. Linux X11/Wayland input remains a separate native adapter project. No unattended control, permission bypass, or physical KVM qualification claim.
