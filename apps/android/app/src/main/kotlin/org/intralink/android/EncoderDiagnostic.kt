package org.intralink.android

import android.content.Context
import android.content.Intent
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.Surface

/**
 * All projection/codec ownership stays on this one worker. Encoded buffers
 * are inspected synchronously for bounded NAL metadata, then released. No
 * encoded content is copied into app storage or sent over the network.
 */
internal class EncoderDiagnostic(
    context: Context,
    private val id: Long,
    private val resultCode: Int,
    private val consent: Intent,
    private val sourceWidth: Int,
    private val sourceHeight: Int,
    private val density: Int,
    private val finishedCallback: () -> Unit
) {
    private val app = context.applicationContext
    private val thread = HandlerThread("IntraLink-capture-diagnostic").apply { start() }
    private val handler = Handler(thread.looper)
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null
    private var codec: MediaCodec? = null
    private var surface: Surface? = null
    private var codecStarted = false
    private var finished = false
    private var startMs = 0L
    private var metrics = CaptureRun.Snapshot(id = id)
    private val info = MediaCodec.BufferInfo()
    private val observation = AvcObservation()
    private val keyframeProbe = KeyframeProbe()

    private val callback = object : MediaProjection.Callback() {
        override fun onStop() = finish(CaptureRun.Phase.SYSTEM_STOP)

        override fun onCapturedContentResize(width: Int, height: Int) {
            // One token, one virtual display. If the system selects different
            // content dimensions, end this diagnostic and request new consent.
            if (width != sourceWidth || height != sourceHeight) finish(CaptureRun.Phase.RESIZED)
        }

        override fun onCapturedContentVisibilityChanged(isVisible: Boolean) {
            if (!isVisible) finish(CaptureRun.Phase.SYSTEM_STOP)
        }
    }

    fun start() {
        handler.post {
            try {
                if (!CaptureRun.active(id)) {
                    finish(CaptureRun.Phase.STOPPED)
                    return@post
                }
                configureEncoder()
                if (!CaptureRun.active(id)) {
                    finish(CaptureRun.Phase.STOPPED)
                    return@post
                }
                // The service has already entered its mediaProjection foreground
                // type before this call. Never obtain the token a second time.
                val manager = app.getSystemService(MediaProjectionManager::class.java)
                val active = requireNotNull(manager.getMediaProjection(resultCode, consent))
                projection = active
                active.registerCallback(callback, handler)
                if (!CaptureRun.active(id)) {
                    finish(CaptureRun.Phase.STOPPED)
                    return@post
                }
                display = requireNotNull(active.createVirtualDisplay(
                    "IntraLink local diagnostic", metrics.width, metrics.height, density,
                    DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, requireNotNull(surface), null, handler
                ))
                startMs = SystemClock.elapsedRealtime()
                CaptureRun.update(metrics)
                handler.post(drain)
            } catch (_: Exception) {
                finish(if (CaptureRun.active(id)) CaptureRun.Phase.ERROR else CaptureRun.Phase.STOPPED)
            }
        }
    }

    private fun configureEncoder() {
        // Hardware preference is optional. Try only bounded advertised
        // profiles; release a failed codec before selecting another one.
        val setupStarted = SystemClock.elapsedRealtime()
        for (profile in CaptureProfiles.candidates(sourceWidth, sourceHeight)) {
            check(CaptureRun.active(id) && SystemClock.elapsedRealtime() - setupStarted <= 5_000)
            var candidate: MediaCodec? = null
            var input: Surface? = null
            try {
                candidate = MediaCodec.createByCodecName(profile.codec)
                candidate.configure(CaptureProfiles.format(profile), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                input = candidate.createInputSurface()
                candidate.start()
                codec = candidate
                surface = input
                codecStarted = true
                metrics = metrics.copy(codec = profile.codec, width = profile.size.width, height = profile.size.height, fps = profile.fps)
                return
            } catch (_: Exception) {
                try { candidate?.release() } catch (_: Exception) { /* next profile remains isolated */ }
                try { input?.release() } catch (_: Exception) { /* no surface is retained */ }
            }
        }
        error("No supported AVC surface encoder")
    }

    private val drain = object : Runnable {
        override fun run() {
            if (finished) return
            if (!CaptureRun.active(id)) {
                finish(CaptureRun.Phase.STOPPED)
                return
            }
            try {
                val encoder = requireNotNull(codec)
                var count = metrics.buffers
                var bytes = metrics.bytes
                var drained = false
                for (attempt in 0 until 32) {
                    if (!CaptureRun.active(id)) {
                        finish(CaptureRun.Phase.STOPPED)
                        return
                    }
                    val index = encoder.dequeueOutputBuffer(info, 0)
                    if (index == MediaCodec.INFO_TRY_AGAIN_LATER) {
                        drained = true
                        break
                    }
                    if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                        observation.recordFormat(AvcOutputFormat.read(encoder.outputFormat,
                            observation.snapshot().format.changes + 1))
                        continue
                    }
                    if (index >= 0) {
                        try {
                            if (info.size > 0) {
                                val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                                val partial = info.flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME != 0
                                val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                                if (!config) {
                                    count++
                                    bytes += info.size.toLong()
                                }
                                val mask = if (observation.canInspect(partial)) {
                                    AvcBuffers.inspect(encoder.getOutputBuffer(index), info.size)
                                } else -1
                                observation.record(mask, config, partial, key, info.presentationTimeUs)
                                if (!config && !partial && mask > 0) {
                                    keyframeProbe.observe(mask, key, info.presentationTimeUs,
                                        SystemClock.elapsedRealtime())
                                }
                            }
                        } finally {
                            encoder.releaseOutputBuffer(index, false)
                        }
                    }
                }
                val now = SystemClock.elapsedRealtime()
                // Only request after the currently available output is drained
                // and the owner has actually observed a reference chain.
                if (drained && keyframeProbe.ready() && CaptureRun.active(id) &&
                    now - startMs < DURATION_MS - 2_000L) {
                    val accepted = try {
                        encoder.setParameters(Bundle().apply {
                            putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0)
                        })
                        true
                    } catch (_: Exception) { false }
                    keyframeProbe.requested(now, accepted)
                }
                keyframeProbe.expire(now)
                val elapsed = now - startMs
                metrics = metrics.copy(buffers = count, bytes = bytes, elapsedMs = elapsed,
                    avc = observation.snapshot().copy(sync = keyframeProbe.snapshot()))
                CaptureRun.update(metrics)
                if (elapsed >= DURATION_MS) {
                    finish(if (count > 0) CaptureRun.Phase.COMPLETE else CaptureRun.Phase.NO_OUTPUT)
                } else handler.postDelayed(this, 10)
            } catch (_: Exception) {
                finish(CaptureRun.Phase.ERROR)
            }
        }
    }

    fun requestStop() {
        CaptureRun.cancel(id)
        handler.post { finish(CaptureRun.Phase.STOPPED) }
    }

    private fun finish(reason: CaptureRun.Phase) {
        if (finished) return
        finished = true
        handler.removeCallbacksAndMessages(null)
        var result = reason
        fun release(operation: () -> Unit) {
            try { operation() } catch (_: Exception) { result = CaptureRun.Phase.ERROR }
        }
        release { display?.release() }
        display = null
        if (codecStarted) release { codec?.stop() }
        release { codec?.release() }
        codec = null
        release { surface?.release() }
        surface = null
        release { projection?.unregisterCallback(callback) }
        release { projection?.stop() }
        projection = null
        CaptureRun.finish(id, result, metrics)
        thread.quitSafely()
        finishedCallback()
    }

    companion object {
        const val DURATION_MS = 15_000L
    }
}
