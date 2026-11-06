package com.worldobservationlog.wrapperlite

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import java.io.File
import java.security.SecureRandom

/**
 * Keeps `wrapper gui` (the Rust binary shipped as libwrapper.so) running in a
 * foreground service with wake/Wi-Fi locks. Everything else — QEMU, accounts,
 * logs, settings — is handled by wrapper and its web panel.
 */
class WrapperService : Service() {

    companion object {
        private const val TAG = "WrapperService"
        const val CHANNEL_ID = "wrapper_service"
        const val NOTIFICATION_ID = 1001
        const val ACTION_STOP = "com.worldobservationlog.wrapperlite.STOP"

        private val panel = MutableStateFlow<String?>(null)
        private val failure = MutableStateFlow<String?>(null)

        /** Panel URL (with access token) once wrapper is listening. */
        val panelUrl: StateFlow<String?> = panel
        /** Last startup problem, for the activity to show. */
        val error: StateFlow<String?> = failure
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    @Volatile private var process: Process? = null
    @Volatile private var stopping = false
    private var started = false
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            shutdown()
            return START_NOT_STICKY
        }
        if (!started) {
            started = true
            startForeground(NOTIFICATION_ID, notification(getString(R.string.status_starting)))
            acquireLocks()
            scope.launch { supervise() }
        }
        return START_STICKY
    }

    private fun accessToken(): String {
        val prefs = getSharedPreferences("wrapper", Context.MODE_PRIVATE)
        prefs.getString("access_token", null)?.let { return it }
        val bytes = ByteArray(16).also { SecureRandom().nextBytes(it) }
        val token = bytes.joinToString("") { "%02x".format(it) }
        prefs.edit().putString("access_token", token).apply()
        return token
    }

    private suspend fun supervise() {
        if (!QemuAssetManager(this).extractIfNeeded()) {
            failure.value = getString(R.string.error_extract)
        }
        val binary = File(applicationInfo.nativeLibraryDir, "libwrapper.so")
        if (!binary.exists()) {
            failure.value = getString(R.string.error_binary)
            return
        }
        val token = accessToken()
        var backoff = 1000L
        while (!stopping) {
            val startedAt = System.currentTimeMillis()
            try {
                val pb = ProcessBuilder(
                    binary.absolutePath, "gui",
                    "--no-browser",
                    // Panel and API share one port (Settings -> Network,
                    // default 127.0.0.1:12340). The access token keeps other
                    // apps on this device out of the panel.
                    "--data-dir", filesDir.absolutePath,
                    "--assets-dir", File(filesDir, "qemu").absolutePath,
                    "--access-token", token,
                ).redirectErrorStream(true).directory(filesDir)
                pb.environment()["HOME"] = filesDir.absolutePath
                pb.environment()["TMPDIR"] = cacheDir.absolutePath
                val proc = pb.start()
                process = proc
                failure.value = null
                proc.inputStream.bufferedReader().forEachLine { line ->
                    Log.i(TAG, line)
                    // "wrapper 3.0.4 panel: http://127.0.0.1:12340/?token=..."
                    Regex("panel: (http\\S+)").find(line)?.let {
                        panel.value = it.groupValues[1]
                        updateNotification(getString(R.string.status_running))
                    }
                }
                val code = proc.waitFor()
                Log.w(TAG, "wrapper exited with $code")
            } catch (e: Exception) {
                Log.e(TAG, "failed to run wrapper", e)
                failure.value = e.message
            }
            process = null
            panel.value = null
            if (stopping) break
            if (System.currentTimeMillis() - startedAt > 60_000) backoff = 1000L
            updateNotification(getString(R.string.status_restarting))
            delay(backoff)
            backoff = (backoff * 2).coerceAtMost(30_000L)
        }
    }

    private fun shutdown() {
        stopping = true
        updateNotification(getString(R.string.status_stopping))
        scope.launch {
            process?.let { p ->
                // SIGTERM: wrapper stops its engine (powering the guest off) first.
                p.destroy()
                if (!awaitExit(p, 40_000) && Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) p.destroyForcibly()
            }
            panel.value = null
            releaseLocks()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    /** Process.waitFor(timeout) needs API 26; poll instead. */
    private suspend fun awaitExit(p: Process, timeoutMs: Long): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            try {
                p.exitValue()
                return true
            } catch (e: IllegalThreadStateException) {
                delay(200)
            }
        }
        return false
    }

    override fun onDestroy() {
        stopping = true
        process?.destroy()
        releaseLocks()
        scope.cancel()
        super.onDestroy()
    }

    private fun acquireLocks() {
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "wrapper:service").apply {
            setReferenceCounted(false)
            acquire()
        }
        val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        @Suppress("DEPRECATION")
        wifiLock = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "wrapper:wifi").apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    private fun releaseLocks() {
        runCatching { wakeLock?.takeIf { it.isHeld }?.release() }
        runCatching { wifiLock?.takeIf { it.isHeld }?.release() }
        wakeLock = null
        wifiLock = null
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                getString(R.string.notification_channel_name),
                NotificationManager.IMPORTANCE_LOW,
            ).apply { description = getString(R.string.notification_channel_desc) }
            getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        }
    }

    private fun notification(text: String): Notification {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, WrapperService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setSmallIcon(R.drawable.ic_stat_wrapper)
            .setContentIntent(open)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, getString(R.string.action_stop), stop)
            .setOngoing(true)
            .build()
    }

    private fun updateNotification(text: String) {
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification(text))
    }
}
