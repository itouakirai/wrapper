package com.worldobservationlog.wrapperlite

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.Binder
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat

class QemuService : Service() {

    companion object {
        const val CHANNEL_ID = "wrapper_lite_service_channel"
        const val NOTIFICATION_ID = 1001
        const val ACTION_STOP = "com.worldobservationlog.wrapperlite.STOP"
    }

    inner class LocalBinder : Binder() {
        fun getService(): QemuService = this@QemuService
    }

    private val binder = LocalBinder()
    lateinit var assetManager: QemuAssetManager
    lateinit var runner: QemuRunner

    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    var onLogListener: ((String) -> Unit)? = null
    var onStatusListener: ((Boolean) -> Unit)? = null

    override fun onCreate() {
        super.onCreate()
        assetManager = QemuAssetManager(this)
        runner = QemuRunner(this, assetManager)
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopQemu()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder = binder

    fun startQemu(
        host: String = "0.0.0.0",
        port: Int = 12340,
        memory: String = "512",
        smp: String = "2",
        proxy: String = "",
        logLevel: String = "info",
        refreshInterval: String = "1800"
    ): Boolean {
        acquireLocks()
        startForeground(NOTIFICATION_ID, buildNotification("Running on :$port"))

        val success = runner.start(host, port, memory, smp, proxy, logLevel, refreshInterval) { line ->
            onLogListener?.invoke(line)
        }

        onStatusListener?.invoke(runner.isRunning)
        if (!success) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            releaseLocks()
        }
        return success
    }

    fun stopQemu() {
        runner.stop()
        releaseLocks()
        onStatusListener?.invoke(false)
        stopForeground(STOP_FOREGROUND_REMOVE)
    }

    private fun acquireLocks() {
        if (wakeLock == null) {
            val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
            wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "wrapperlite:qemu_wakelock")
            wakeLock?.acquire(24 * 60 * 60 * 1000L) // 24 hours
        }
        if (wifiLock == null) {
            val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            wifiLock = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "wrapperlite:wifi_lock")
            wifiLock?.acquire()
        }
    }

    private fun releaseLocks() {
        try {
            wakeLock?.let { if (it.isHeld) it.release() }
            wakeLock = null
            wifiLock?.let { if (it.isHeld) it.release() }
            wifiLock = null
        } catch (e: Exception) {}
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                getString(R.string.notification_channel_name),
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = getString(R.string.notification_channel_desc)
            }
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(statusText: String): Notification {
        val openIntent = Intent(this, MainActivity::class.java)
        val pOpen = PendingIntent.getActivity(
            this, 0, openIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val stopIntent = Intent(this, QemuService::class.java).apply { action = ACTION_STOP }
        val pStop = PendingIntent.getService(
            this, 1, stopIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("wrapper-lite QEMU")
            .setContentText(statusText)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentIntent(pOpen)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Stop", pStop)
            .setOngoing(true)
            .build()
    }

    override fun onDestroy() {
        stopQemu()
        super.onDestroy()
    }
}
