# Physical Android / desktop LAN acceptance

This is an explicit developer/owner-operated acceptance harness for the existing authenticated transport and generated media only. It never accesses personal display content, input, a user's vault, or a distribution signing key. Use the owner-selected ADB Wi-Fi serial and distinct private IPv4 addresses. The Android app and instrumentation must be compatible and signed by the same authorized debug key. Never use pm clear, global firewall changes, permission bypasses, or unrelated device discovery.

## One-shot probe

Build the Windows peer example and ordinary Android debug/instrumentation APKs:

    cargo build --locked -p intralink-transport --example lan_probe_acceptance --release
    apps/android/gradlew -p apps/android :app:assembleDebug :app:assembleDebugAndroidTest

Install only with the selected adb -s SERIAL target. Preserve existing app data and do not replace an unrelated signature. The Python coordinator requires an explicit ADB Wi-Fi serial matching the specified Android private address. It starts a short-lived Windows endpoint with a fresh public identity, passes only that identity to Android instrumentation and returns the Android public identity to Windows stdin. No private key or password is passed across ADB or the network. Android creates a temporary encrypted vault, uses actual JNI manual pairing/reservation/probe and revokes the grant after the exchange. The local test vault is deleted by the instrumentation finally block. Windows uses an independent in-memory Profile and grant. Both endpoints close. The harness never invokes MediaProjection or a system capture-consent dialog.

    python -B -u tools/verify_android_lan_probe.py --serial ANDROID_IP:ADB_PORT --windows-ip WINDOWS_IP --android-ip ANDROID_IP --windows-port WINDOWS_TEST_PORT --android-port ANDROID_TEST_PORT --executable PATH_TO_EXAMPLE --direction inbound

Direction is from Android's perspective: inbound = Android CLIENT/host, Windows BASE initiates. Outbound = Android BASE, Windows CLIENT listens. Use --direction both only when inbound UDP on Windows is already appropriately authorized. Windows Public network profiles may prevent unsolicited inbound UDP. Do not disable the firewall; if necessary the operator should explicitly authorize only the named program, local test UDP port and selected private peer, then remove that temporary permission after use. ADB connectivity alone does not establish QUIC connectivity. The reported session durations include binding, TLS, exchange and closure, not RTT.

## Generated full-frame media

The optional Rust lan-fixture feature enables the same bounded v12 Compact software view path with a generated 1920x1080 RGB color-band and moving marker fixture. The shared test producer emits actual lossless keyframe and delta records, the receiver reconstructs and checks every RGB pixel, and the coordinator requires exact matching frame/payload statistics. No native decoder receives arbitrary network data. The feature uses the same encrypted local grant and cancellation registry but a distinct explicit test invocation. No real display capture, input, media recording, presentation FPS or general video throughput claim.

Build the explicit non-distributable debug variant and peer:

    cargo build --locked -p intralink-transport --features lan-fixture --example lan_probe_acceptance --release
    apps/android/gradlew -p apps/android -PintralinkLanFixture=true :app:assembleDebug :app:assembleDebugAndroidTest

The Gradle property disables Release variants and uses a separate generated JNI directory. The candidate validator refuses any native fixture export. Only explicit instrumentation arguments can exercise the extra debug entry; the normal application's JNI has no such symbol. Save these debug test artifacts separately, and restore the ordinary APK after tests. Do not publish fixture-enabled builds. Run the same coordinator with --media and the explicit direction. Both directions are supported by the generated fixture harness; any network failure is recorded rather than silently downgraded.

## Physical checkpoint

On 2026-11-27 the owner authorized a physical TB373FU Android 16/API 36 ARM64 device (1840x2944 display) and the Windows machine. Installed the preserved exact preview.9 debug/instrumentation pair on a previously uninstalled app. Native onboarding, borrowed AVC buffers, SPS and actual software c2.android.avc.decoder fixture passed: six original and four recovered frames, all retained visible planar YUV hashes equal (12,441,600 bytes). Windows BASE to Android CLIENT actual authenticated JNI/QUIC probe passed: Android session 1193 ms, Windows 1295 ms, revocation/lease cleanup passed. Android BASE to Windows CLIENT initially timed out (Android connect deadline); Windows Wi-Fi is classified Public, firewall profiles enabled. Windows inbound UDP restriction is suspected, not proven. No firewall or network profile modifications performed. No physical screen capture or remote input performed. Three further Windows-initiated probes using fresh identities passed (Android 1079/1054/1026 ms, Windows 1186/1150/1132 ms); all four attempts in this direction passed.

Actual generated media gate: physical Android CLIENT/host to Windows BASE/receiver, initiated by Windows on the private Wi-Fi network. Both exact opt-in native ABIs passed strict Clippy/Release and Debug/instrumentation Gradle build. Real v12 Compact session passed 60/60 1920x1080 frames, 45,276 compressed payload bytes, full RGB reconstruction of all 124,416,000 pixels (373,248,000 RGB bytes), sender and receiver statistics agreed, no repeats/no-updates, 3605 ms Android total session and 3689 ms Windows total session. This is an end-to-end synthetic-media integrity result, not 30/60 sustained presentation FPS, typical bandwidth, or H.264 network admission. The test used fresh pinned identities and verified revocation and lease cleanup. APKs and example retained locally under dist/physical-lan-fixture-test-20261127, explicitly NOT-FOR-DISTRIBUTION. Exact ordinary preview.9 APK/test restored on tablet without clearing data; no active projection/service or Windows test UDP listener afterwards. Candidate export guard accepted both ordinary ABI libraries and rejected both fixture-enabled libraries. No permissions or registry dependency changes.

Final repetition after strengthening instrumentation reservation cleanup: 60/60 1080p frames, the same 45,276 encoded bytes and every pixel matched; Android total 3196 ms, Windows total 3299 ms, both exited successfully. Across two real media sessions: 120 frames, 248,832,000 verified RGB pixels. The exact ordinary preview.9 app and test APK were restored again, with no active MediaProjection or app services. Final native fixture binary and instrumentation hashes recorded in benchmarks/physical-android-lan-20261127.txt. The ADB route is deliberately not included in repository documents.
