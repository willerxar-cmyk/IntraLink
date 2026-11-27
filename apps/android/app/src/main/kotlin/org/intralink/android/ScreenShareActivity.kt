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
import android.text.InputFilter
import android.text.InputType
import android.view.View
import android.view.WindowInsets
import android.view.WindowManager
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import java.nio.CharBuffer
import java.nio.charset.StandardCharsets

/** One visible screen-only preview for a reviewed, previously paired BASE. */
class ScreenShareActivity : Activity() {
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var form: LinearLayout
    private lateinit var password: EditText
    private lateinit var address: EditText
    private lateinit var port: EditText
    private lateinit var status: TextView
    private lateinit var start: Button
    private lateinit var stop: Button
    private lateinit var navigation: CheckBox
    private var approvedNavigation = false
    private var foreground = false
    private var asking = false
    private var request: String? = null
    private var pass: ByteArray? = null
    private var consent: Intent? = null
    private var approvedAt = 0L
    private var session = 0L
    private var name = ""
    private var identity = ""

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Android 12L+ has the aspect-preserving virtual-display scaling
        // required by this first portable 16:9 screen preview.
        if (Build.VERSION.SDK_INT < 32) { finish(); return }
        name = intent.getStringExtra(EXTRA_NAME).orEmpty()
        identity = intent.getStringExtra(EXTRA_IDENTITY).orEmpty()
        if (!name.matches(Regex("[A-Za-z0-9_-]{1,48}")) || identity.length !in 1..128) {
            finish()
            return
        }
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE or WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val body = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(20), dp(20), dp(20))
            setBackgroundColor(Color.WHITE)
        }
        body.addView(TextView(this).apply {
            textSize = 24f
            setText(R.string.screen_share_title)
        })
        body.addView(TextView(this).apply { text = getString(R.string.screen_share_review, name, identity) })
        form = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        address = field(R.string.local_ip, 15).apply {
            setText(ConnectionCard.localAddress(this@ScreenShareActivity).orEmpty())
        }
        port = field(R.string.port, 5).apply {
            inputType = InputType.TYPE_CLASS_NUMBER
            setText(R.string.default_port)
        }
        password = field(R.string.vault_passphrase, 1024).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
        }
        form.addView(address)
        form.addView(port)
        form.addView(password)
        navigation = CheckBox(this).apply {
            setText(R.string.screen_share_navigation)
            isChecked = false
            isSaveEnabled = false
        }
        form.addView(navigation)
        body.addView(form)
        body.addView(PreviewFixture(this), LinearLayout.LayoutParams(-1, 0, 1f))
        status = TextView(this).apply {
            setText(R.string.capture_idle)
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        }
        body.addView(status)
        start = Button(this).apply {
            isAllCaps = false
            setText(R.string.screen_share_approve)
            setOnClickListener { requestConsent() }
        }
        stop = Button(this).apply {
            isAllCaps = false
            setText(R.string.capture_stop)
            setOnClickListener { cancel() }
        }
        body.addView(start)
        body.addView(stop)
        body.addView(Button(this).apply {
            isAllCaps = false
            setText(R.string.cancel)
            setOnClickListener { cancel(); finish() }
        })
        setContentView(body)
        body.setOnApplyWindowInsetsListener { view, insets ->
            val safe = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            view.setPadding(dp(20) + safe.left, dp(20) + safe.top, dp(20) + safe.right, dp(20) + safe.bottom)
            insets
        }
        body.requestApplyInsets()
        refresh()
    }

    private fun dp(n: Int) = (n * resources.displayMetrics.density).toInt()
    private fun field(hintId: Int, limit: Int) = EditText(this).apply {
        setHint(hintId)
        setSingleLine(true)
        filters = arrayOf(InputFilter.LengthFilter(limit))
        isSaveEnabled = false
    }

    private fun takePassword(): ByteArray {
        val chars = CharArray(password.text.length) { password.text[it] }
        password.text.clear()
        return try {
            val encoded = StandardCharsets.UTF_8.encode(CharBuffer.wrap(chars))
            try {
                ByteArray(encoded.remaining()).also { encoded.get(it) }
            } finally {
                if (encoded.hasArray()) encoded.array().fill(0)
            }
        } finally { chars.fill('\u0000') }
    }

    private fun clearPending() {
        pass?.fill(0)
        pass = null
        request = null
        consent = null
        asking = false
        handler.removeCallbacks(expiry)
    }

    private val expiry = Runnable {
        clearPending()
        if (!isFinishing && !isDestroyed && ::status.isInitialized) {
            status.setText(R.string.capture_error)
            refresh()
        }
    }

    @Suppress("DEPRECATION")
    private fun requestConsent() {
        if (!foreground || !hasWindowFocus() || asking || pass != null || busy()) return
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            password.text.clear()
            status.setText(R.string.capture_notifications_required)
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATIONS)
            return
        }
        if (!getSystemService(NotificationManager::class.java).areNotificationsEnabled()) {
            status.setText(R.string.capture_notifications_required)
            return
        }
        val ip = address.text.toString().trim()
        val udp = port.text.toString().toIntOrNull()
        if (ip.isEmpty() || udp == null || udp !in 1..65535) {
            status.setText(R.string.pair_invalid)
            return
        }
        val secret = takePassword()
        if (secret.size !in 12..1024) {
            secret.fill(0)
            status.setText(R.string.passphrase_length)
            return
        }
        approvedNavigation = navigation.isChecked
        request = JSONObject().put("name", name).put("peer_id", identity)
            .put("host", ip).put("port", udp).put("height", 1080).put("fps", 60)
            .put("seconds", if (approvedNavigation) 90 else 15).toString()
        pass = secret
        session = 0
        approvedAt = SystemClock.elapsedRealtime()
        asking = true
        getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(password.windowToken, 0)
        form.visibility = View.GONE
        refresh()
        try {
            val manager = getSystemService(MediaProjectionManager::class.java)
            val system = if (Build.VERSION.SDK_INT >= 34)
                manager.createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())
                else manager.createScreenCaptureIntent()
            handler.postDelayed(expiry, 60_000)
            startActivityForResult(system, PROJECTION)
        } catch (_: Exception) {
            clearPending()
            status.setText(R.string.capture_error)
            refresh()
        }
    }

    @Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != PROJECTION || !asking) return
        asking = false
        if (resultCode != RESULT_OK || data == null || isFinishing || isDestroyed ||
            SystemClock.elapsedRealtime() - approvedAt >= 60_000) {
            clearPending()
            status.setText(R.string.capture_denied)
            refresh()
            return
        }
        consent = data
        begin()
    }

    private fun begin() {
        if (!foreground || !hasWindowFocus() || !::start.isInitialized) return
        val token = consent ?: return
        val secret = pass ?: return
        val payload = request ?: return
        if (SystemClock.elapsedRealtime() - approvedAt >= 60_000) {
            clearPending(); refresh(); return
        }
        // Clear the local password UI before removing FLAG_SECURE. Only this
        // preview Activity stays visible; leaving it immediately cancels.
        if (Build.VERSION.SDK_INT < 32) {
            clearPending()
            return
        }
        val bounds = windowManager.maximumWindowMetrics.bounds
        val id = try {
            ScreenShareRun.reserve(File(filesDir, "profile.vault").absolutePath,
                payload, secret, bounds.width(), bounds.height(), resources.displayMetrics.densityDpi,
                approvedNavigation)
        } catch (_: Exception) { 0L } catch (_: LinkageError) { 0L }
        if (id == 0L) {
            clearPending()
            status.setText(R.string.probe_busy)
            refresh()
            return
        }
        pass = null // ownership transferred to the process-local approval
        clearPending()
        session = id
        window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        try {
            startForegroundService(Intent(this, ScreenShareService::class.java)
                .setAction(ScreenShareService.START)
                .putExtra(ScreenShareService.EXTRA_ID, id)
                .putExtra(ScreenShareService.EXTRA_CONSENT, token))
        } catch (_: Exception) {
            ScreenShareRun.finish(id, false)
        }
        refresh()
    }

    private fun busy(): Boolean {
        val phase = ScreenShareRun.snapshot().phase
        return phase == ScreenShareRun.Phase.WAITING || phase == ScreenShareRun.Phase.STOPPING
    }

    private fun cancel() {
        clearPending()
        if (session != 0L) ScreenShareRun.cancel(session)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        refresh()
    }

    override fun onResume() {
        super.onResume()
        foreground = true
        if (::start.isInitialized) {
            begin()
            handler.post(tick)
        }
    }

    override fun onPause() {
        foreground = false
        if (::password.isInitialized) password.text.clear()
        if (session != 0L && ScreenShareRun.active(session) && !ScreenShareRun.mayNavigate(session)) cancel()
        if (!asking && consent == null) clearPending()
        handler.removeCallbacks(tick)
        super.onPause()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!::start.isInitialized) return
        if (hasFocus) begin()
        else if (session != 0L && ScreenShareRun.active(session) && !ScreenShareRun.mayNavigate(session)) cancel()
    }

    override fun onDestroy() {
        if (::start.isInitialized) cancel() else clearPending()
        handler.removeCallbacks(tick)
        super.onDestroy()
    }

    private val tick = object : Runnable {
        override fun run() {
            refresh()
            if (foreground) handler.postDelayed(this, 250)
        }
    }

    private fun refresh() {
        if (!::start.isInitialized) return
        val current = ScreenShareRun.snapshot()
        val running = session != 0L && current.id == session && busy()
        start.isEnabled = !asking && pass == null && !busy()
        stop.isEnabled = running || asking || pass != null
        form.visibility = if (start.isEnabled) View.VISIBLE else View.GONE
        if (session == 0L || current.id != session) return
        if (!running) window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        when (current.phase) {
            ScreenShareRun.Phase.WAITING -> status.setText(
                if (ScreenShareRun.mayNavigate(session)) R.string.screen_share_navigation_active
                else R.string.screen_share_waiting)
            ScreenShareRun.Phase.STOPPING -> status.setText(R.string.capture_stopping)
            ScreenShareRun.Phase.STOPPED -> status.setText(R.string.capture_stopped)
            ScreenShareRun.Phase.ERROR -> status.setText(R.string.screen_share_failed)
            ScreenShareRun.Phase.COMPLETE -> {
                val stats = current.result?.let { JSONObject(it) }
                status.text = getString(R.string.screen_share_result,
                    stats?.optLong("frames") ?: 0L, stats?.optLong("media_bytes") ?: 0L,
                    stats?.optLong("screen_elapsed_ms") ?: 0L)
            }
        }
    }

    private class PreviewFixture(activity: Activity) : View(activity) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        override fun onDraw(canvas: Canvas) {
            canvas.drawColor(Color.rgb(20, 36, 54))
            val radius = minOf(width, height) * 0.12f
            paint.color = Color.rgb(104, 216, 191)
            val phase = (SystemClock.elapsedRealtime() % 2000) / 2000f
            canvas.drawCircle(radius + (width - radius * 2).coerceAtLeast(0f) * phase,
                height / 2f, radius, paint)
            if (isShown) postInvalidateOnAnimation()
        }
    }

    companion object {
        const val EXTRA_NAME = "screen_peer_name"
        const val EXTRA_IDENTITY = "screen_peer_identity"
        private const val PROJECTION = 2405
        private const val NOTIFICATIONS = 2406
    }
}
