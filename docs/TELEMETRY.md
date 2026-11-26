# Local workspace performance observations

The Controller owns one `SessionCounters` and `RateSampler` per connected slot. Counters use relaxed atomic operations to avoid synchronization on the media hot path; samples are observational, not transactional. The UI samples roughly once per second using the actual monotonic elapsed time. Each session starts with fresh counters. Nothing is sent to a telemetry server or written to the session journal.

| Value | Measurement |
|---|---|
| RX FPS | Complete changed media records received, excluding repeat records |
| DEC FPS | Changed records successfully applied to the decoder canvas |
| VIEW FPS | Distinct new sequences successfully submitted to the native foreground window |
| Payload Mbps | Changed media payload bytes times eight divided by elapsed seconds and 1,000,000 |
| Mean decode/present milliseconds | Local stage timing estimates available in the sampler; decode includes surface update, present is the successful window submission call |

The displayed `VIEW` metric is not a measurement of physical display scanout. A background or static device can legitimately show zero new presented frames while its authorized connection remains open. A static screen can have zero changed-frame RX as well. Protocol overhead, ACKs, repeat records and retransmissions are excluded from payload Mbps. Rates at different stages can cross at sampling boundaries; do not interpret a difference as lost network frames. Mean stage timing estimates can be affected by independently sampled counters. No claim of network latency, motion-to-photon latency or achieved 60 FPS is inferred from these observations.

The window title reports the selected device's latest sample when windowed. The Tab overview reserves a small footer for each slot's metrics. Its ON/OFF control describes local activation, not proof of an established socket. The overview uses the same bounded 960x540 canvas as before, keeps a thumbnail only for the current slot lifetime and skips resampling an already displayed sequence. A busy decoder retains the last complete thumbnail; an invalidated surface, closed session, slot reuse or overview exit clears it. No new full-size frame cache or network stream is added.

## Acceptance gates

On a Windows or Linux machine with real approved hosts, run the workspace with a continuous-motion desktop workload and test one, three and eight active slots. Compare RX, DEC and VIEW over long stable intervals, check CPU/GPU and RSS, and confirm background devices recover immediately on selection. Record network traffic independently and measure end-to-end latency with a controlled visual event. Repeat with static content and both adaptive and all-live mode. Verify that unchanged frames do not inflate the rates, and confirm disconnect/reconnect never shows another session's thumbnail. Test fullscreen, DPI changes and native window recreation. These gates are not satisfied by the standalone unit tests.
