package org.intralink.android

/**
 * One local sync-frame request per diagnostic epoch. The observation is
 * bounded and cannot prove that a subsequent IDR was caused by the request.
 * No frame contents or transport authority are retained.
 */
internal class KeyframeProbe {
    enum class Phase { WAITING, REQUESTED, OBSERVED, REJECTED, EXPIRED }
    data class Stats(
        val phase: Phase = Phase.WAITING,
        val latencyMs: Long = -1,
        val ptsDeltaUs: Long = -1,
        val interveningPictures: Int = 0
    )

    private var result = Stats()
    private var lastPts = -1L
    private var seenIdr = false
    private var following = 0
    private var requestedAt = -1L
    private var requestPts = -1L

    fun snapshot(): Stats = result

    // Only complete, successfully inspected VCL buffers with a matching
    // encoder key flag and strictly advancing PTS are evidence.
    fun observe(mask: Int, key: Boolean, ptsUs: Long, nowMs: Long) {
        expire(nowMs)
        if (mask <= 0 || ptsUs < 0 || ptsUs <= lastPts) return
        val vcl = mask and 0x3e
        if (vcl != 2 && vcl != 32) return
        val idr = vcl == 32
        if (key != idr) return
        lastPts = ptsUs
        if (result.phase == Phase.REQUESTED) {
            if (idr && ptsUs > requestPts && nowMs >= requestedAt) {
                result = result.copy(phase = Phase.OBSERVED,
                    latencyMs = nowMs - requestedAt, ptsDeltaUs = ptsUs - requestPts)
            } else if (!idr) {
                result = result.copy(interveningPictures = result.interveningPictures + 1)
            }
        }
        if (idr) {
            seenIdr = true
            following = 0
        } else if (seenIdr && following < 2) following++
    }

    fun ready(): Boolean = result.phase == Phase.WAITING && seenIdr && following >= 2

    fun requested(nowMs: Long, accepted: Boolean) {
        check(ready() && nowMs >= 0)
        requestedAt = nowMs
        requestPts = lastPts
        result = Stats(if (accepted) Phase.REQUESTED else Phase.REJECTED)
    }

    fun expire(nowMs: Long) {
        if (result.phase == Phase.REQUESTED &&
            (nowMs < requestedAt || nowMs - requestedAt >= DEADLINE_MS)) {
            result = result.copy(phase = Phase.EXPIRED)
        }
    }

    companion object { private const val DEADLINE_MS = 2_000L }
}
