package org.intralink.android

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import kotlin.math.min

internal data class CaptureSize(val width: Int, val height: Int)
internal data class CaptureProfile(
    val codec: String,
    val size: CaptureSize,
    val fps: Int,
    val bitrate: Int
)

internal object CaptureProfiles {
    /** No upscale; fit portrait or landscape within a 1920 x 1080 envelope. */
    fun fit(width: Int, height: Int, shortLimit: Int, longLimit: Int, alignW: Int, alignH: Int): CaptureSize? {
        if (width !in 1..16384 || height !in 1..16384 || shortLimit <= 0 || longLimit <= 0 ||
            alignW !in 1..1024 || alignH !in 1..1024) return null
        val landscape = width >= height
        val maxW = if (landscape) longLimit else shortLimit
        val maxH = if (landscape) shortLimit else longLimit
        val ratio = min(1.0, min(maxW.toDouble() / width, maxH.toDouble() / height))
        val w = (width * ratio).toInt() / alignW * alignW
        val h = (height * ratio).toInt() / alignH * alignH
        return if (w >= 64 && h >= 64) CaptureSize(w, h) else null
    }

    fun candidates(width: Int, height: Int): List<CaptureProfile> {
        val encoders = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .filter { it.isEncoder && it.supportedTypes.any { type -> type.equals(MediaFormat.MIMETYPE_VIDEO_AVC, true) } }
            .filter { Build.VERSION.SDK_INT < 29 || !it.isAlias }
            .sortedByDescending { Build.VERSION.SDK_INT >= 29 && it.isHardwareAccelerated }
        val result = LinkedHashSet<CaptureProfile>()
        for ((shortLimit, longLimit) in listOf(1080 to 1920, 720 to 1280, 540 to 960)) {
            for (fps in listOf(60, 30, 24)) {
                for (encoder in encoders) {
                    val caps = try {
                        encoder.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
                    } catch (_: IllegalArgumentException) {
                        continue
                    }
                    if (MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface !in caps.colorFormats) continue
                    val video = caps.videoCapabilities ?: continue
                    val size = fit(width, height, shortLimit, longLimit, video.widthAlignment, video.heightAlignment) ?: continue
                    if (!video.areSizeAndRateSupported(size.width, size.height, fps.toDouble())) continue
                    val target = (size.width.toLong() * size.height * fps / 12).coerceIn(500_000, 12_000_000).toInt()
                    val bitrate = video.bitrateRange.clamp(target)
                    if (bitrate > 12_000_000) continue
                    result.add(CaptureProfile(encoder.name, size, fps, bitrate))
                }
            }
        }
        return result.toList()
    }

    fun format(profile: CaptureProfile): MediaFormat =
        MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, profile.size.width, profile.size.height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, profile.bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, profile.fps)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
        }
}
