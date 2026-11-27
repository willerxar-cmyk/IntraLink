package org.intralink.android

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper

/** Non-exported, non-sticky foreground service for one local diagnostic. */
class CaptureService : Service() {
    private var runner: EncoderDiagnostic? = null
    private var session = 0L
    private var destroyed = false
    private val main = Handler(Looper.getMainLooper())

    override fun onBind(intent: Intent?): IBinder? = null

    // The typed Parcelable accessor is available from API 33. Retain the
    // platform accessor only for the supported API 26-32 compatibility path.
    @Suppress("DEPRECATION")
    private fun consent(intent: Intent): Intent? =
        if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(EXTRA_CONSENT, Intent::class.java)
        else intent.getParcelableExtra(EXTRA_CONSENT)

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val id = intent?.getLongExtra(EXTRA_ID, 0) ?: 0
        if (intent?.action == STOP) {
            if (id != 0L && id == session) {
                CaptureRun.cancel(id)
                runner?.requestStop()
            }
            if (runner == null) stopSelf(startId)
            return START_NOT_STICKY
        }
        if (runner != null) {
            if (id != session) CaptureRun.finish(id, CaptureRun.Phase.ERROR)
            return START_NOT_STICKY
        }
        session = id
        try {
            // Start foreground before obtaining MediaProjection (API 29+).
            val notification = notification(id)
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
            } else startForeground(NOTIFICATION, notification)
            val token = intent?.let(::consent)
            val code = intent?.getIntExtra(EXTRA_RESULT, Activity.RESULT_CANCELED)
            val width = intent?.getIntExtra(EXTRA_WIDTH, 0) ?: 0
            val height = intent?.getIntExtra(EXTRA_HEIGHT, 0) ?: 0
            val density = intent?.getIntExtra(EXTRA_DENSITY, 0) ?: 0
            if (intent?.action != START || id == 0L || token == null || code != Activity.RESULT_OK ||
                width !in 64..16384 || height !in 64..16384 || density !in 1..2000 || !CaptureRun.claim(id)) {
                CaptureRun.finish(id, CaptureRun.Phase.STOPPED)
                stopSelf(startId)
                return START_NOT_STICKY
            }
            runner = EncoderDiagnostic(this, id, code, token, width, height, density) {
                main.post {
                    runner = null
                    if (!destroyed) {
                        stopForeground(STOP_FOREGROUND_REMOVE)
                        stopSelf()
                    }
                }
            }.also { it.start() }
        } catch (_: Exception) {
            CaptureRun.finish(id, CaptureRun.Phase.ERROR)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf(startId)
        }
        return START_NOT_STICKY
    }

    private fun notification(id: Long): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.capture_channel), NotificationManager.IMPORTANCE_LOW))
        check(manager.areNotificationsEnabled() && manager.getNotificationChannel(CHANNEL).importance != NotificationManager.IMPORTANCE_NONE)
        val stop = Intent(this, CaptureService::class.java).setAction(STOP).putExtra(EXTRA_ID, id)
        val pending = PendingIntent.getService(this, id.hashCode(), stop, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_intralink)
            .setContentTitle(getString(R.string.capture_title))
            .setContentText(getString(R.string.capture_notification))
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .addAction(Notification.Action.Builder(null, getString(R.string.capture_stop), pending).build())
            .apply {
                if (Build.VERSION.SDK_INT >= 31) setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
            }.build()
    }

    override fun onDestroy() {
        destroyed = true
        if (session != 0L) CaptureRun.cancel(session)
        runner?.requestStop()
        runner = null
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    companion object {
        const val START = "org.intralink.android.capture.START"
        const val STOP = "org.intralink.android.capture.STOP"
        const val EXTRA_ID = "capture_id"
        const val EXTRA_CONSENT = "capture_consent"
        const val EXTRA_RESULT = "capture_result"
        const val EXTRA_WIDTH = "capture_width"
        const val EXTRA_HEIGHT = "capture_height"
        const val EXTRA_DENSITY = "capture_density"
        private const val CHANNEL = "local_capture_diagnostic"
        private const val NOTIFICATION = 2401
    }
}
