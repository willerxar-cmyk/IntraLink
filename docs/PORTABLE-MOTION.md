# Portable motion policy and generated physical acceptance (preview.12)

The existing v12 Compact/JPEG software transport remains the default portable path. No new permission, dependency, JNI entry, media tag, profile negotiation or network listener. Screen sharing remains explicitly consented and independent of input authority.

## Policy

The sender still compares the actual complete RGB canvas before declaring Repeat. The new bounded motion heuristic samples one RGB pixel every 16 pixels in each axis, comparing current/previous and a neighboring RGB value. It selects dense motion only when at least 3/8 of sampled pixels changed and at least 1/4 of samples are changed and detailed. It is not a codec classifier or a substitute for exact tile comparisons. Uniform or sparse changes stay eligible for the previous exact lossless paths. After two measured costly outputs, a dense epoch can skip redundant lossless attempts and use JPEG directly. Before that, dense tile attempts are bounded by twice the per-frame soft payload target. Outside dense motion, the prior tile budget and receipt-driven quality policy still apply.

A local soft target of 24 Mbit/s is divided by the configured FPS to obtain a nominal byte budget per changed frame. Three consecutive dense JPEG outputs larger than 125% of that budget reduce the local JPEG ceiling by ten quality points, down to the existing minimum 35. Forty-five small JPEG outputs below 75% of budget recover five points, never above the negotiated ceiling. Receipt-driven adaptation remains independently active; outside dense motion the payload-specific ceiling does not degrade sparse UI JPEG. The target is not a hard bandwidth cap, actual throughput estimate, dynamic resolution change or a promise of visual fidelity at the minimum quality. If dense content remains large even at quality 35, the sender does not silently drop a required frame or violate the existing bounded wire format.

A JPEG base is lossy at the receiver even when the original source pixels match the previous source canvas. On transition to sparse motion, Compact attempts one exact full keyframe before sending a delta or Repeat. A successful full keyframe clears the lossy-base flag; an unsuccessful attempt remains bounded until a new JPEG or dense epoch. This prevents unchanged original tiles from preserving old JPEG residuals indefinitely when a complete lossless refresh fits the existing payload budget.

The desktop bounded JPEG decoder uses the existing image crate and its JpegDecoder, validates exact negotiated dimensions and expected output bytes, and reuses one decoded byte scratch buffer. Only after successful full decode does it update the existing presentation allocation. RGB, grayscale, RGBA and gray-alpha conversion match the previous supported output semantics. The compressed input and codec internal allocations still exist. A malformed JPEG must not mutate the presentation canvas. This adds no native decoder or external dependency.

## Physical generated A/B

The instrumentation-only DenseMotionFixture generates 210 full-size frames through a real Android Surface/ImageReader: 10 sparse preview frames, 140 textured moving frames and 60 sparse recovery frames. Both revisions use the ordinary runScreen JNI, ephemeral pinned identities, private v12 QUIC, and Windows graphical receiver. The benchmark does not capture actual display or personal data and requires no system MediaProjection approval. It creates and revokes only temporary test grants, never overwrites a user's vault.

The desktop acceptance example supports --dense-motion, requiring exact 210 received/presented changed records, matched sender/receiver byte counters, 140 dense sampled RGB quality measurements (PSNR), and full sparse-pixel comparison. Run identical instrumentation/receiver and same selected device with previous and current signed debug APKs, with no concurrent heavy builds. Report actual frame interval, payload, codec record counts, decode time and measured distortion; do not conflate synthetic results with ordinary video quality or sustained capture.

    python -B -u tools/verify_android_dense_motion.py --serial OWNER_ADB_SERIAL --android-ip OWNER_PRIVATE_IP --viewer PATH_TO_EXAMPLE --label REVISION --quality

Preserve earlier APKs and package hashes. The generated test and quality inspector are not part of production UI.

## Independent AVC admission assessment

The instrumentation-only DenseAvcFixture drives the same generated texture into a native MediaCodec Surface at the explicit 1920x1080/60fps/6Mbit/s operating point, with High profile, level 4.2, no B frames and BT.709 limited requested. Actual codec output must provide bounded explicit Annex B framing, CSD, ordered picture timestamps, keyframe flags and EOS. Only this generated test exports a bounded ILAVC001 file in the app-private cache; the coordinator validates exact declared length/SHA-256 before parsing or decoding and removes that generated file after collection. No real MediaProjection data is retained.

The Rust example android_avc_admission assesses exact recorded units using the existing strict v9 Packetizer without relaxing SPS/PPS/slice rules. Failure is an incompatibility observation, not a reason to weaken the loopback guard. The external CPU FFmpeg test uses only the locally generated and hash-verified fixture, with explicit H.264 pipe input and bounded output frame count. It verifies actual metadata, ordered unique decoded frame hashes and picture count; this is an offline trusted-source decoder qualification, not a sandbox or arbitrary remote decoder. No new AVC LAN path is enabled until the exact format/decoder admission and isolation gates are met.

    cargo build --locked -p intralink-core --example android_avc_admission --release
    python -B tools/verify_android_dense_avc.py --serial OWNER_ADB_SERIAL --admission PATH_TO_EXAMPLE --output NEW_TEST_DIRECTORY

Do not publish generated fixture data or test binaries as application releases.
