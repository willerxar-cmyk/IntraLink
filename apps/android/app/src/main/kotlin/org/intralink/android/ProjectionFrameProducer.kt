package org.intralink.android

import android.content.Context
import android.content.Intent
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Handler
import android.os.Looper
import android.view.Surface

/** OS-approved projection, owned and released by the single screen worker. */
internal class ProjectionFrameProducer(
    context: Context,
    resultCode: Int,
    consent: Intent,
    private val approval: ScreenShareRun.Approval
) : PortableFrameSource.Producer {
    private val manager = context.getSystemService(MediaProjectionManager::class.java)
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null
    private val callback = object : MediaProjection.Callback() {
        override fun onStop() = ScreenShareRun.cancel(approval.id)
        override fun onCapturedContentResize(width: Int, height: Int) {
            if (width != approval.width || height != approval.height) {
                ScreenShareRun.cancel(approval.id)
            }
        }
        override fun onCapturedContentVisibilityChanged(isVisible: Boolean) {
            if (!isVisible) ScreenShareRun.cancel(approval.id)
        }
    }

    init {
        check(ScreenShareRun.active(approval.id))
        val token = requireNotNull(manager.getMediaProjection(resultCode, consent))
        projection = token
        try {
            token.registerCallback(callback, Handler(Looper.getMainLooper()))
        } catch (error: Exception) {
            token.stop()
            projection = null
            throw error
        }
    }

    override fun attach(surface: Surface, width: Int, height: Int) {
        check(ScreenShareRun.active(approval.id) && display == null)
        // Android 12L+ uniformly scales and centers the approved display
        // into the destination Surface. No stretching or orientation guess.
        display = requireNotNull(projection).createVirtualDisplay(
            "IntraLink approved screen preview", width, height, approval.density,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, surface, null, null
        ) ?: error("Cannot create approved virtual display")
    }

    override fun demand() { check(ScreenShareRun.active(approval.id)) }
    override fun complete(): Boolean = false

    override fun close() {
        try { display?.release() } finally {
            display = null
            val token = projection
            projection = null
            if (token != null) {
                try { token.unregisterCallback(callback) } finally { token.stop() }
            }
        }
    }
}
