package org.intralink.android

import android.Manifest
import android.app.Activity
import android.app.NotificationManager
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.media.projection.MediaProjectionConfig
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.View
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

/**
 * Explicit local capture test screen, separate from the FLAG_SECURE vault UI.
 * Only a generated moving fixture and numeric encoder measurements appear here.
 */
class CaptureActivity : Activity() {
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var start: Button
    private lateinit var stop: Button
    private lateinit var status: TextView
    private lateinit var metrics: TextView
    private var resumed = false
    private var asking = false
    private var pendingConsent: Intent? = null
    private var pendingAt = 0L
    private var session = 0L

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val body = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(28), dp(20), dp(24))
            setBackgroundColor(Color.WHITE)
        }
        body.addView(TextView(this).apply {
            textSize = 24f
            setText(R.string.capture_title)
        })
        body.addView(TextView(this).apply { setText(R.string.capture_explanation) })
        body.addView(Fixture(this), LinearLayout.LayoutParams(-1, 0, 1f))
        status = TextView(this).apply {
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
            setText(R.string.capture_idle)
        }
        body.addView(status)
        metrics = TextView(this)
        body.addView(metrics)
        start = Button(this).apply {
            setText(R.string.capture_start)
            isAllCaps = false
            setOnClickListener { requestConsent() }
        }
        stop = Button(this).apply {
            setText(R.string.capture_stop)
            isAllCaps = false
            setOnClickListener { cancel() }
        }
        body.addView(start)
        body.addView(stop)
        body.addView(Button(this).apply {
            setText(R.string.cancel)
            setOnClickListener { cancel(); finish() }
        })
        setContentView(body)
        if (Build.VERSION.SDK_INT >= 30) {
            body.setOnApplyWindowInsetsListener { view, insets ->
                val safe = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
                view.setPadding(dp(20) + safe.left, dp(28) + safe.top, dp(20) + safe.right, dp(24) + safe.bottom)
                insets
            }
            body.requestApplyInsets()
        }
        refresh()
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    // A framework-only Activity supports API 26+ without AndroidX. The two
    // legacy result APIs are intentionally isolated to the consent boundary.
    @Suppress("DEPRECATION")
    private fun requestConsent() {
        if (!resumed || !hasWindowFocus() || asking || busy()) return
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            status.setText(R.string.capture_notifications_required)
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATIONS_REQUEST)
            return
        }
        if (!getSystemService(NotificationManager::class.java).areNotificationsEnabled()) {
            status.setText(R.string.capture_notifications_required)
            return
        }
        try {
            val manager = getSystemService(MediaProjectionManager::class.java)
            val request = if (Build.VERSION.SDK_INT >= 34) {
                manager.createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())
            } else manager.createScreenCaptureIntent()
            session = 0L
            asking = true
            start.isEnabled = false
            startActivityForResult(request, CAPTURE_REQUEST)
        } catch (_: Exception) {
            asking = false
            status.setText(R.string.capture_error)
            start.isEnabled = true
        }
    }

    @Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != CAPTURE_REQUEST || !asking) return
        asking = false
        if (resultCode == RESULT_OK && data != null && !isFinishing && !isDestroyed) {
            pendingConsent = data
            pendingAt = SystemClock.elapsedRealtime()
            beginIfReady()
        } else {
            status.setText(R.string.capture_denied)
            start.isEnabled = true
        }
    }

    private fun beginIfReady() {
        if (!resumed || !hasWindowFocus()) return
        val consent = pendingConsent ?: return
        pendingConsent = null
        if (SystemClock.elapsedRealtime() - pendingAt > 10_000 || busy()) {
            status.setText(R.string.capture_error)
            return
        }
        val bounds = if (Build.VERSION.SDK_INT >= 30) windowManager.maximumWindowMetrics.bounds else null
        val width = bounds?.width() ?: resources.displayMetrics.widthPixels
        val height = bounds?.height() ?: resources.displayMetrics.heightPixels
        val id = CaptureRun.reserve()
        if (id == 0L) return
        session = id
        try {
            val service = Intent(this, CaptureService::class.java).setAction(CaptureService.START)
                .putExtra(CaptureService.EXTRA_ID, id)
                .putExtra(CaptureService.EXTRA_CONSENT, consent)
                .putExtra(CaptureService.EXTRA_RESULT, RESULT_OK)
                .putExtra(CaptureService.EXTRA_WIDTH, width)
                .putExtra(CaptureService.EXTRA_HEIGHT, height)
                .putExtra(CaptureService.EXTRA_DENSITY, resources.displayMetrics.densityDpi)
            startForegroundService(service)
        } catch (_: Exception) {
            CaptureRun.finish(id, CaptureRun.Phase.ERROR)
        }
        refresh()
    }

    private fun busy(): Boolean = CaptureRun.snapshot().phase in setOf(
        CaptureRun.Phase.STARTING, CaptureRun.Phase.RUNNING, CaptureRun.Phase.STOPPING
    )

    private fun cancel() {
        pendingConsent = null
        if (session != 0L) CaptureRun.cancel(session)
        refresh()
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        beginIfReady()
        handler.post(tick)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!::start.isInitialized) return
        if (hasFocus) beginIfReady() else if (session != 0L && CaptureRun.active(session)) cancel()
    }

    override fun onPause() {
        resumed = false
        cancel()
        handler.removeCallbacks(tick)
        super.onPause()
    }

    private val tick = object : Runnable {
        override fun run() {
            refresh()
            if (resumed) handler.postDelayed(this, 250)
        }
    }

    private fun refresh() {
        val result = CaptureRun.snapshot()
        start.isEnabled = !asking && pendingConsent == null && !busy()
        stop.isEnabled = session != 0L && CaptureRun.active(session)
        if (session == 0L || result.id != session) return
        status.setText(when (result.phase) {
            CaptureRun.Phase.IDLE -> R.string.capture_idle
            CaptureRun.Phase.STARTING -> R.string.capture_starting
            CaptureRun.Phase.RUNNING -> R.string.capture_running
            CaptureRun.Phase.STOPPING -> R.string.capture_stopping
            CaptureRun.Phase.COMPLETE -> R.string.capture_complete
            CaptureRun.Phase.STOPPED, CaptureRun.Phase.SYSTEM_STOP -> R.string.capture_stopped
            CaptureRun.Phase.RESIZED -> R.string.capture_resized
            CaptureRun.Phase.NO_OUTPUT -> R.string.capture_no_output
            CaptureRun.Phase.ERROR -> R.string.capture_error
        })
        val avc = result.avc
        val observedRate = if (result.elapsedMs > 0) avc.pictureBuffers * 1000.0 / result.elapsedMs else 0.0
        metrics.text = getString(R.string.capture_metrics_combined,
            getString(R.string.capture_metrics, result.codec, result.width, result.height,
                result.fps, result.buffers, result.bytes, result.elapsedMs),
            getString(R.string.capture_metrics_combined,
                getString(R.string.capture_avc_metrics, avc.pictureBuffers, observedRate, avc.idrBuffers,
                    avc.configBuffers, avc.partialBuffers, avc.unparsedBuffers,
                    avc.keyFlagMismatches, avc.nonIncreasingPts),
                getString(R.string.capture_format_metrics,
                    avc.format.sps?.profileIdc ?: -1, avc.format.sps?.levelIdc ?: -1,
                    avc.format.sps?.width ?: -1, avc.format.sps?.height ?: -1,
                    avc.format.csdMask, avc.format.changes, avc.format.colorStandard,
                    avc.format.colorRange, avc.format.colorTransfer,
                    avc.minDeltaUs, avc.maxDeltaUs, avc.ptsSpanUs,
                    avc.sync.phase.ordinal, avc.sync.latencyMs, avc.sync.ptsDeltaUs)))
    }

    private class Fixture(activity: Activity) : View(activity) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        override fun onDraw(canvas: Canvas) {
            super.onDraw(canvas)
            canvas.drawColor(Color.rgb(20, 36, 54))
            paint.color = Color.rgb(104, 216, 191)
            val travel = ((SystemClock.elapsedRealtime() % 2000).toFloat() / 2000)
            val radius = (minOf(width, height) * 0.12f).coerceAtLeast(1f)
            canvas.drawCircle(radius + (width - radius * 2).coerceAtLeast(0f) * travel,
                height / 2f, radius, paint)
            if (isShown) postInvalidateOnAnimation()
        }
    }

    companion object {
        private const val CAPTURE_REQUEST = 2402
        private const val NOTIFICATIONS_REQUEST = 2403
    }
}
