package org.intralink.android

import android.media.MediaFormat
import java.nio.ByteBuffer

/** Numeric output format observation, not a negotiated decoder capability. */
internal data class AvcFormatObservation(
    val width: Int = -1,
    val height: Int = -1,
    val profile: Int = -1,
    val level: Int = -1,
    val colorStandard: Int = -1,
    val colorRange: Int = -1,
    val colorTransfer: Int = -1,
    val csdMask: Int = 0,
    val sps: SpsMetadata? = null,
    val changes: Int = 0
)

internal data class SpsMetadata(
    val width: Int,
    val height: Int,
    val profileIdc: Int,
    val levelIdc: Int,
    val constraints: Int,
    val references: Int,
    val pocType: Int
) {
    companion object {
        fun unpack(value: Long): SpsMetadata? {
            if (value < 0 || value ushr 53 != 0L) return null
            fun field(shift: Int, mask: Long) = ((value ushr shift) and mask).toInt()
            return SpsMetadata(field(0, 2047), field(11, 2047), field(22, 255),
                field(30, 255), field(38, 255), field(46, 31), field(51, 3))
        }
    }
}

internal object AvcOutputFormat {
    private const val MAX_CSD = 1032

    // MediaFormat may expose heap-backed CSD. Only these small parameter
    // buffers may be copied into direct storage; never copy coded pictures.
    private fun directCsd(buffer: ByteBuffer?): ByteBuffer? {
        if (buffer == null || buffer.remaining() !in 1..MAX_CSD) return null
        val source = buffer.slice()
        if (source.isDirect) return source
        return ByteBuffer.allocateDirect(source.remaining()).apply {
            put(source)
            flip()
        }
    }

    fun read(format: MediaFormat, changes: Int): AvcFormatObservation {
        fun value(key: String): Int = if (format.containsKey(key)) {
            try { format.getInteger(key) } catch (_: Exception) { -1 }
        } else -1
        fun csd(key: String): ByteBuffer? = if (format.containsKey(key)) {
            try { directCsd(format.getByteBuffer(key)) } catch (_: Exception) { null }
        } else null
        var mask = 0
        var sps: SpsMetadata? = null
        var spsBuffers = 0
        for (key in listOf("csd-0", "csd-1")) {
            val buffer = csd(key) ?: continue
            val parsed = NativeBridge.inspectAvc(buffer, buffer.remaining(), 0)
            if (parsed > 0) {
                mask = mask or parsed
                if (parsed and (1 shl 7) != 0) {
                    // Annex B framing is specified explicitly for Android CSD.
                    val observed = SpsMetadata.unpack(NativeBridge.inspectAvcSps(buffer, buffer.remaining(), 0))
                    spsBuffers++
                    sps = if (spsBuffers == 1) observed else null
                }
            }
        }
        return AvcFormatObservation(value(MediaFormat.KEY_WIDTH), value(MediaFormat.KEY_HEIGHT),
            value(MediaFormat.KEY_PROFILE), value(MediaFormat.KEY_LEVEL),
            value(MediaFormat.KEY_COLOR_STANDARD), value(MediaFormat.KEY_COLOR_RANGE),
            value(MediaFormat.KEY_COLOR_TRANSFER), mask, sps, changes)
    }
}
