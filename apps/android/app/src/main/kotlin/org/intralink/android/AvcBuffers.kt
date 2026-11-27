package org.intralink.android

import java.nio.ByteBuffer

/** The caller owns this codec buffer exclusively until inspection returns. */
internal object AvcBuffers {
    private const val MAX_ACCESS_UNIT = 2 * 1024 * 1024 - 8
    private const val ANNEX_B = 0

    fun inspect(buffer: ByteBuffer?, size: Int): Int {
        // MediaCodec.getOutputBuffer sets position/limit to valid output bytes.
        // A slice has exactly that capacity; no frame copy or framing guess.
        if (buffer == null || !buffer.isDirect || size !in 1..MAX_ACCESS_UNIT ||
            buffer.remaining() != size) return -1
        return NativeBridge.inspectAvc(buffer.slice(), size, ANNEX_B)
    }
}
