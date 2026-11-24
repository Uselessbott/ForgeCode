package dev.forgebox.app.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import dev.forgebox.app.R
import dev.forgebox.app.terminal.TerminalSessionController
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * M7: owns N PTY session lifetimes in foreground. Rust runtime remains
 * process-wide singleton (M1 §3.1). Android 12+ phantom-process killer may still
 * terminate children; this service improves liveness but does not guarantee it.
 */
class SessionService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private val scope = CoroutineScope(Dispatchers.Main.immediate + Job())

    override fun onCreate() {
        super.onCreate()
        createChannel()
        scope.launch {
            TerminalSessionController.state.collect { st ->
                val content = NotificationContentBuilder.build(st.sessions, st.activePtyId, st.sessionCap)
                if (content.shouldStopForeground) {
                    releaseWakeLock()
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                } else {
                    val manager = getSystemService(NotificationManager::class.java)
                    manager.notify(NOTIFICATION_ID, buildNotification(content))
                }
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Android 14+ requires foreground service type on startForeground; call immediately.
        val notification = buildNotification()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        when (intent?.action) {
            ACTION_STOP_TERMINAL -> TerminalSessionController.stop()
            ACTION_STOP_ALL -> TerminalSessionController.stopAll()
            ACTION_END_ALL -> {
                TerminalSessionController.stopAll()
                releaseWakeLock()
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_OPEN_TERMINAL -> {
                acquireWakeLock()
                val projectId = intent.getStringExtra(EXTRA_PROJECT_ID)
                TerminalSessionController.open(this, projectId)
            }
        }
        return START_STICKY
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        TerminalSessionController.stopAll()
        releaseWakeLock()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        scope.cancel()
        TerminalSessionController.stopAll()
        releaseWakeLock()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun createChannel() {
        val manager = getSystemService(NotificationManager::class.java)
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.notification_channel),
            NotificationManager.IMPORTANCE_LOW,
        )
        manager.createNotificationChannel(channel)
    }

    private fun buildNotification(content: NotificationContent? = null): Notification {
        val stopIntent = PendingIntent.getService(this, 1, Intent(this, javaClass).setAction(ACTION_STOP_TERMINAL), PendingIntent.FLAG_IMMUTABLE)
        val stopAllIntent = PendingIntent.getService(this, 3, Intent(this, javaClass).setAction(ACTION_STOP_ALL), PendingIntent.FLAG_IMMUTABLE)
        val endIntent = PendingIntent.getService(this, 2, Intent(this, javaClass).setAction(ACTION_END_ALL), PendingIntent.FLAG_IMMUTABLE)
        val st = TerminalSessionController.state.value
        val c = content ?: NotificationContentBuilder.build(st.sessions, st.activePtyId, st.sessionCap)
        val b = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_stat_forgebox)
            .setContentTitle(c.title)
            .setContentText(c.text)
            .setOngoing(true)
        if (c.showStopActive) b.addAction(Notification.Action.Builder(null, "Stop active", stopIntent).build())
        if (c.showStopAll) b.addAction(Notification.Action.Builder(null, "Stop all", stopAllIntent).build())
        if (c.showEndAll) b.addAction(Notification.Action.Builder(null, "End all", endIntent).build())
        return b.build()
    }

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "ForgeBox:TerminalSession").apply {
            setReferenceCounted(false)
            acquire(10 * 60 * 1000L)
        }
    }

    private fun releaseWakeLock() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
    }

    companion object {
        const val CHANNEL_ID = "forgebox_sessions"
        const val NOTIFICATION_ID = 1001
        const val ACTION_OPEN_TERMINAL = "dev.forgebox.action.OPEN_TERMINAL"
        const val ACTION_STOP_TERMINAL = "dev.forgebox.action.STOP_TERMINAL"
        const val ACTION_STOP_ALL = "dev.forgebox.action.STOP_ALL"
        const val ACTION_END_ALL = "dev.forgebox.action.END_ALL"
        const val EXTRA_PROJECT_ID = "project_id"

        fun open(context: Context, projectId: String?) {
            val intent = Intent(context, SessionService::class.java)
                .setAction(ACTION_OPEN_TERMINAL)
                .putExtra(EXTRA_PROJECT_ID, projectId)
            context.startForegroundService(intent)
        }
    }
}
