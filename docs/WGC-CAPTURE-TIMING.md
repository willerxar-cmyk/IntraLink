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
