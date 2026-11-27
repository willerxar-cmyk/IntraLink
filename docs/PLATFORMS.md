# Target matrix and release status

Preview.4 desktop candidates: Windows x64 passed local native generated-window diagnostics; Linux x86_64 passed native Ubuntu 24.04 CI strict lint, tests, Release, CLI execution, ldd, inventory and archive verification. Physical multi-device KVM, Linux real X11/Wayland capture and Linux input injection remain unqualified. Android local setup candidate is built; next source implements manual pairing and one-shot diagnostics. Release signing, physical Android acceptance, screen and input capabilities remain pending. See [DESKTOP.md](DESKTOP.md), [ONBOARDING.md](ONBOARDING.md) and [RELEASE-PREVIEW.md](RELEASE-PREVIEW.md).

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
