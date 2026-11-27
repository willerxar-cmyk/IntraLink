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
import org.json.JSONObject

/** Separate, non-exported, non-sticky service for one screen-only session. */
class ScreenShareService : Service() {
    private var session = 0L
    private var worker: Thread? = null
    private var destroyed = false
    private val main = Handler(Looper.getMainLooper())

    override fun onBind(intent: Intent?): IBinder? = null

    @Suppress("DEPRECATION")
    private fun consent(intent: Intent): Intent? =
        if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(EXTRA_CONSENT, Intent::class.java)
        else intent.getParcelableExtra(EXTRA_CONSENT)

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val id = intent?.getLongExtra(EXTRA_ID, 0) ?: 0
        if (intent?.action == STOP) {
            if (id != 0L && id == session) ScreenShareRun.cancel(id)
            if (worker == null) stopSelf(startId)
            return START_NOT_STICKY
        }
        if (worker != null) {
            if (id != session) ScreenShareRun.cancel(id)
            return START_NOT_STICKY
        }
        session = id
        var approval: ScreenShareRun.Approval? = null
        try {
            val token = intent?.let(::consent)
            check(intent?.action == START && id != 0L && token != null)
            val notification = notification(id)
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
            } else startForeground(NOTIFICATION, notification)
            val request = requireNotNull(ScreenShareRun.claim(id))
            approval = request
            worker = Thread({ runSession(request, token) }, "IntraLink-screen-share").also { it.start() }
            approval = null // worker owns the password until its finally block
        } catch (_: Exception) {
            approval?.password?.fill(0)
            ScreenShareRun.finish(id, false)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf(startId)
        } catch (_: LinkageError) {
            approval?.password?.fill(0)
            ScreenShareRun.finish(id, false)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf(startId)
        }
        return START_NOT_STICKY
    }

    private fun runSession(approval: ScreenShareRun.Approval, token: Intent) {
        var result: JSONObject? = null
        try {
            check(ScreenShareRun.active(approval.id))
            check(NativeBridge.initializeNetwork(applicationContext))
            val producer = ProjectionFrameProducer(this, Activity.RESULT_OK, token, approval)
            PortableFrameSource(producer) { ScreenShareRun.active(approval.id) }.use { source ->
                result = JSONObject(NativeBridge.runScreen(approval.path, approval.request,
                    approval.id, approval.password, source))
            }
        } catch (_: Exception) {
            result = null
        } catch (_: LinkageError) {
            result = null
        } finally {
            approval.password.fill(0)
            ScreenShareRun.finish(approval.id, result?.optBoolean("ok") == true, result?.toString())
            main.post {
                worker = null
                if (!destroyed) {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                }
            }
        }
    }

    private fun notification(id: Long): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.screen_share_title),
            NotificationManager.IMPORTANCE_LOW))
        check(manager.areNotificationsEnabled() &&
            manager.getNotificationChannel(CHANNEL).importance != NotificationManager.IMPORTANCE_NONE)
        val stop = Intent(this, ScreenShareService::class.java).setAction(STOP).putExtra(EXTRA_ID, id)
        val pending = PendingIntent.getService(this, id.hashCode(), stop,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_intralink)
            .setContentTitle(getString(R.string.screen_share_title))
            .setContentText(getString(R.string.screen_share_notification))
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .addAction(Notification.Action.Builder(null, getString(R.string.capture_stop), pending).build())
            .apply {
                if (Build.VERSION.SDK_INT >= 31) setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
            }.build()
    }

    override fun onDestroy() {
        destroyed = true
        if (session != 0L) ScreenShareRun.cancel(session)
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    companion object {
        const val START = "org.intralink.android.screen.START"
        const val STOP = "org.intralink.android.screen.STOP"
        const val EXTRA_ID = "screen_id"
        const val EXTRA_CONSENT = "screen_consent"
        private const val CHANNEL = "approved_screen_preview"
        private const val NOTIFICATION = 2404
    }
}
