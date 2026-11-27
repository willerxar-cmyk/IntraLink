package org.intralink.android

/** Local, bounded metadata only. No codec bytes or native pointers are kept. */
internal class AvcObservation {
    data class Stats(
        val pictureBuffers: Long = 0,
        val idrBuffers: Long = 0,
        val configBuffers: Long = 0,
        val partialBuffers: Long = 0,
        val unparsedBuffers: Long = 0,
        val keyFlagMismatches: Long = 0,
        val nonIncreasingPts: Long = 0,
        val parameterMask: Int = 0,
        val ptsSpanUs: Long = 0,
        val minDeltaUs: Long = 0,
        val maxDeltaUs: Long = 0,
        val format: AvcFormatObservation = AvcFormatObservation()
    )

    private var stats = Stats()
    private var continuation = false
    private var firstPts: Long? = null
    private var lastPts: Long? = null

    fun recordFormat(format: AvcFormatObservation) {
        stats = stats.copy(format = format)
    }

    fun snapshot(): Stats = stats

    // A partial frame and all its continuation buffers are excluded from
    // inspection; we do not pretend each fragment is a complete picture.
    fun canInspect(partial: Boolean): Boolean = !partial && !continuation

    fun record(mask: Int, config: Boolean, partial: Boolean, key: Boolean, ptsUs: Long) {
        if (!canInspect(partial)) {
            continuation = partial
            stats = stats.copy(partialBuffers = stats.partialBuffers + 1)
            return
        }
        if (config) stats = stats.copy(configBuffers = stats.configBuffers + 1)
        if (mask <= 0 || mask and VALID_NALS != mask) {
            stats = stats.copy(unparsedBuffers = stats.unparsedBuffers + 1)
            return
        }
        stats = stats.copy(parameterMask = stats.parameterMask or (mask and PARAMETERS))
        val picture = mask and VCL != 0
        if (config && picture) {
            stats = stats.copy(unparsedBuffers = stats.unparsedBuffers + 1)
            return
        }
        if (!picture) return
        val idr = mask and IDR != 0
        val previous = lastPts
        val valid = ptsUs >= 0 && (previous == null || ptsUs > previous)
        var span = stats.ptsSpanUs
        var minDelta = stats.minDeltaUs
        var maxDelta = stats.maxDeltaUs
        if (valid) {
            if (firstPts == null) firstPts = ptsUs
            span = ptsUs - requireNotNull(firstPts)
            if (previous != null) {
                val delta = ptsUs - previous
                minDelta = if (minDelta == 0L) delta else minOf(minDelta, delta)
                maxDelta = maxOf(maxDelta, delta)
            }
            lastPts = ptsUs
        }
        stats = stats.copy(
            pictureBuffers = stats.pictureBuffers + 1,
            idrBuffers = stats.idrBuffers + if (idr) 1 else 0,
            keyFlagMismatches = stats.keyFlagMismatches + if (idr != key) 1 else 0,
            nonIncreasingPts = stats.nonIncreasingPts + if (!valid) 1 else 0,
            ptsSpanUs = span, minDeltaUs = minDelta, maxDeltaUs = maxDelta
        )
    }

    companion object {
        private const val VALID_NALS = 0x00fffffe
        private const val VCL = 0x3e
        private const val IDR = 1 shl 5
        private const val PARAMETERS = (1 shl 7) or (1 shl 8)
    }
}
