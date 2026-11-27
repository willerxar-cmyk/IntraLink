# Target matrix and release status

Windows x64 preview.3 is public and its generated-window native video diagnostics passed; physical multi-device KVM is unqualified. Linux X11 desktop source exists, but the native build and physical acceptance are pending. Android has no app yet. The manual Portable desktop candidates workflow builds and packages Windows/Linux independently; hosted artifacts are not automatically released. The table below remains a target matrix, not a list of supported release claims. See [DESKTOP.md](DESKTOP.md) and [RELEASE-PREVIEW.md](RELEASE-PREVIEW.md).

| Platform | Controller | Host approach | Targets |
|---|---|---|---|
| Windows | GUI / CLI | Windows.Graphics.Capture or DXGI; documented input APIs; user-session process | x86_64, i686, aarch64 MSVC |
| Linux X11 | GUI / CLI | X11 capture and authorized input | x86_64, aarch64, i686, armv7 GNU |
| Linux Wayland | GUI / CLI | XDG ScreenCast/RemoteDesktop portals and PipeWire; compositor consent | same, subject to desktop support |
| macOS | GUI / CLI | ScreenCaptureKit, accessibility permissions and authorized event APIs | x86_64, aarch64 Apple Darwin |
| Android | GUI | MediaProjection with foreground service and local consent, permitted accessibility gestures if approved | aarch64, armv7, x86_64, i686 Android |
| iOS/iPadOS | GUI | OS-provided user-approved screen-sharing APIs only; no general OS-wide input injection | aarch64 Apple iOS |
| Headless Linux / Raspberry Pi | CLI, GUI where available | permitted resource and terminal modules; screen only if an appropriate graphical session exists | x86_64, aarch64, armv7 |

32-bit targets require independent dependency and GUI audits; modern macOS/iOS do not support 32-bit application distribution. Some mobile devices do not support all native ABIs. Android capture consent and foreground service behavior depend on the Android version; Google Play requires accurate accessibility declaration, disclosure and consent where applicable. iOS app sandbox/background restrictions prevent offering desktop-equivalent arbitrary remote control or system-wide unattended access via ordinary App Store APIs. Do not bypass protections.

Desktop packaging plan: signed MSI/MSIX or EXE, signed/notarized DMG, DEB/RPM/AppImage where supported. Android signed AAB and optional APK; iOS signed IPA through Apple's approved distribution mechanisms. App store developer accounts, signing infrastructure and optional relay hosting may incur costs even though the IntraLink protocol has no subscription. Cross-compilation alone is not a test of installation, capture, permissions or runtime behavior. Use native runners and real devices for every supported combination.
