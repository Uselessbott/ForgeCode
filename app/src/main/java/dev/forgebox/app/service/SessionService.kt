package dev.forgebox.app.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.IBinder
import android.os.PowerManager
import dev.forgebox.app.R
import dev.forgebox.app.terminal.TerminalSessionController

/**
 * M6: owns the single PTY session lifetime in foreground. Rust runtime remains
 * process-wide singleton (M1 §3.1). Android 12+ phantom-process killer may still
 * terminate children; this service improves liveness but does not guarantee it.
 */
class SessionService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP_TERMINAL -> TerminalSessionController.stop()
            ACTION_END_ALL -> {
                TerminalSessionController.closeSession()
                releaseWakeLock()
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_OPEN_TERMINAL -> {
                acquireWakeLock()
                val projectId = intent.getStringExtra(EXTRA_PROJECT_ID)
                TerminalSessionController.open(projectId)
            }
        }
        startForeground(NOTIFICATION_ID, buildNotification())
        return START_STICKY
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        TerminalSessionController.closeSession()
        releaseWakeLock()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        TerminalSessionController.closeSession()
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

    private fun buildNotification(): Notification {
        val stopIntent = PendingIntent.getService(this, 1, Intent(this, javaClass).setAction(ACTION_STOP_TERMINAL), PendingIntent.FLAG_IMMUTABLE)
        val endIntent = PendingIntent.getService(this, 2, Intent(this, javaClass).setAction(ACTION_END_ALL), PendingIntent.FLAG_IMMUTABLE)
        val state = TerminalSessionController.state.value
        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_stat_forgebox)
            .setContentTitle("ForgeBox terminal")
            .setContentText(if (state.ptyId != null) "Session running: ${state.cwdLabel}" else "No active terminal session")
            .setOngoing(true)
            .addAction(Notification.Action.Builder(null, "Stop terminal", stopIntent).build())
            .addAction(Notification.Action.Builder(null, "End all", endIntent).build())
            .build()
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
