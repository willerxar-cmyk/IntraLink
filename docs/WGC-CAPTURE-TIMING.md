# WGC capture timing and bounded callback-lock experiment

## Scope

The ordinary Host retains the previous nonblocking callback-lock and wake-queue policy. The new explicit local-only `desktop-profile-check --capture-lock` command compares it with a bounded 1 ms callback mutex wait, using the same ordered two-demand transport, 1080p60 Compact quality 75, owned calm generated window, real WGC scaling and software codec. The native and app timing instrumentation adds monotonic clock reads; no network timestamps, persistent frame references, new dependencies, remote capability or mandatory NVIDIA acceleration.

## Measurements

- `generation_notification_wait`: elapsed notification-channel waits after inspecting the callback and finding no new generation.
- `callback_contention_wait`: elapsed notification waits after unsuccessful lock acquisition in the original mode, or actual bounded mutex waits in the experiment. These are branch-attributed wall times, not the exact duration of callback ownership.
- `native_consume`: all calls under the callback lock to check device/geometry/generation, create or reuse scaler, read pixels and construct the image.
- `scale_submission`: CPU wall time of scaler source validation and GPU processing submission, excluding scaler creation and downstream readback.
- `copy_submission`: CPU wall time of staging CopyResource submission.
- `map_wait`: CPU wall time of D3D11 Map, including synchronization with outstanding GPU work; not isolated GPU execution time.
- `cpu_row_copy_unmap`: mapped row validation, vector resize, row copy, unmap and final device check.
- The first three are disjoint subsets of capture-attempt time; the latter four are disjoint subsets of consume time. The diagnostic checks those relationships and that every successful native readback has matching timing data. The distribution includes zero waits and initial setup; percentiles are not additive. A missing generation never gets stale readback timings.

The existing pipeline trace now reports p50, p95, p99 and worst for each stage. Readback-specific samples count only actual readbacks; notification/consume distributions include all attempts. Contention poll and affected-attempt counts expose sparse waits that a median may hide. Delivery remains local publication-to-viewer callback, not actual ACK arrival or presentation.

## Experiment and safety

WGC notifies while its callback can still own the lock. A failed try_lock followed by a notification wait may therefore sleep even when the mutex becomes free shortly afterward. The comparison mode waits up to 1 ms (or remaining capture-attempt budget) for that same mutex, checks local cancellation and source liveness again, and repeats only after a bounded blocking wait. It does not busy-spin, alter the timer resolution, expand the eight existing credits, extend deadlines, switch sources, bypass identity checks or change encoded frame order. The synchronous native call still is not forcibly preemptible. This is a hypothesis, not a demonstrated bottleneck until measured.

The comparison uses ABBA: original/bounded/bounded/original, 60 records each. It then runs a separate full-pixel visual regression with the experimental lock policy. Normal `--pipeline` still runs serial/overlap/overlap/serial with the original lock policy; normal `--visual` verifies both existing transport paths. All modes use the calm labeled local window, not a monitor or other user's application. No physical network, true remote presentation or sustained 60 FPS claim.

## Validation

Run the complete gate in NEXT-MODEL-PROMPT.md plus `desktop-profile-check --capture-lock`. Check CLI exclusivity, all timing sample counts, native visual/retarget/geometry results and all real exit codes. Preserve the prior qualified executable. The bounded mutex policy is diagnostic-only; do not activate it for Host sessions until separate measured benefit and cancellation/idle qualification are established. The independent high-resolution Windows timer, ACK timing, presentation timing and adaptive transition barriers remain follow-up work.


### WGC capture timing qualification (2026-11-27)

Gate PID 8404 exited 0 (279.16 s): 98 standalone, 218 CPU workspace, 231 NVIDIA workspace, 60 scalar app tests; fmt, four strict Clippy configurations, locked WGC portable Release, pipeline, visual, capture-lock, retarget (30/30), geometry and lockfile/diff checks passed. No mini-cuda or mini-nvcodec in the WGC graph. Implementation a303d03. Evidence: benchmarks/wgc-capture-timing-validation-20261127.txt and benchmarks/wgc-capture-lock-repeat-20261127.txt (repeat PID 22308 exit 0). Normal pipeline ABBA serial 44.20/45.30 changed FPS and overlap 45.85/47.16; not a speedup over the previous short runs. For the two ordinary overlap runs, native capture p50 7.059-7.911 ms, Map p50 5.799-6.621 ms/p95 9.012-10.548 ms, scale submit p50 0.322-0.342 ms, staging copy submission p50 0.004 ms, row-copy/unmap p50 0.781-0.831 ms. Start lateness p95 7.606-7.831 ms, publication p95 8.303-9.140 ms. Map includes queued GPU work and is not an isolated GPU execution measurement. The 60-sample p99 equals worst; longer samples are needed. Lock ABBA original 43.25/45.71 versus bounded 43.13/47.75; repeat original 43.80/43.59 versus bounded 42.19/43.04. Only 5 of 720 timed attempts encountered callback contention, all in the repeated original policy; the bounded-lock branch did not encounter actual contention. No reproducible gain or contended-path qualification; retain existing Host policy. Visual runs compared 479001600 pixels in 231 changed frames (including independent experimental repeat), max channel MAE 1.1546, max block MAE 5.2227, no pixel channel error above 48. Native local generated-window/QUIC loopback only; no physical presentation or sustained 1080p60 proof. Artifact dist/intralink-windows-x64-wgc-capture-timing-portable-preview.exe (15286784 bytes), SHA256 52a91ed5e69fbe150d191fc8eef8780246f91ca2ae59f19f4b03fc00f72dfde4.

Next: prioritize D3D11 Map synchronization and bounded authorized staging/copy overlap (no speculative readback, no extra demand credit, preserve exact source generation, cancellation and ordered publication), plus a CUDA-independent optional absolute-deadline timer. Existing TimerWorker uses a shared response receiver; design request-specific replies and cancellation-safe shutdown before using it in a select loop. Keep Tokio portable default and compare both under identical scenes with longer runs. Separate actual ACK arrival and presentation from current local delivery. Adaptive transition barriers, physical device sessions and decoder containment remain unqualified.
