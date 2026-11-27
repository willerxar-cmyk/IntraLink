package org.intralink.android

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.KeyguardManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.IntentFilter
import android.os.PowerManager
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

    private var lockReceiverRegistered = false
    private val lockReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == Intent.ACTION_SCREEN_OFF && session != 0L) ScreenShareRun.cancel(session)
        }
    }

    private fun displayAvailable(): Boolean =
        getSystemService(PowerManager::class.java).isInteractive &&
            !getSystemService(KeyguardManager::class.java).isKeyguardLocked

    private val watchdog = object : Runnable {
        override fun run() {
            if (session == 0L || !ScreenShareRun.active(session)) return
            val manager = getSystemService(NotificationManager::class.java)
            val visible = manager.areNotificationsEnabled() &&
                manager.getNotificationChannel(CHANNEL)?.importance != NotificationManager.IMPORTANCE_NONE &&
                manager.activeNotifications.any { it.id == NOTIFICATION }
            if (!displayAvailable() || !visible) ScreenShareRun.cancel(session)
            else main.postDelayed(this, 250)
        }
    }

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
            val request = requireNotNull(ScreenShareRun.claim(id))
            approval = request
            check(displayAvailable())
            val notification = notification(id, request.navigation)
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
            } else startForeground(NOTIFICATION, notification)
            if (!lockReceiverRegistered) {
                if (Build.VERSION.SDK_INT >= 33) registerReceiver(lockReceiver, IntentFilter(Intent.ACTION_SCREEN_OFF), Context.RECEIVER_NOT_EXPORTED)
                else registerReceiver(lockReceiver, IntentFilter(Intent.ACTION_SCREEN_OFF))
                lockReceiverRegistered = true
            }
            main.postDelayed(watchdog, 500)
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
            PortableFrameSource(producer) {
                ScreenShareRun.active(approval.id) && displayAvailable()
            }.use { source ->
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

    private fun notification(id: Long, navigation: Boolean): Notification {
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
            .setContentText(getString(if (navigation) R.string.screen_share_navigation_notification else R.string.screen_share_notification))
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .addAction(Notification.Action.Builder(null, getString(R.string.capture_stop), pending).build())
            .apply {
                if (Build.VERSION.SDK_INT >= 31) setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
            }.build()
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        if (session != 0L) ScreenShareRun.cancel(session)
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        destroyed = true
        main.removeCallbacks(watchdog)
        if (lockReceiverRegistered) {
            unregisterReceiver(lockReceiver)
            lockReceiverRegistered = false
        }
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
