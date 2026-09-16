package com.worldobservationlog.wrapperlite

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.webkit.JavascriptInterface
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONObject

class WebAppInterface(
    private val context: Context,
    private val serviceProvider: () -> QemuService?,
    private val evaluateJs: (String) -> Unit
) {

    private val prefs = context.getSharedPreferences("wl_prefs", Context.MODE_PRIVATE)

    @JavascriptInterface
    fun getPlatform(): String = "android"

    @JavascriptInterface
    fun isServiceRunning(): Boolean {
        return serviceProvider()?.runner?.isRunning ?: false
    }

    @JavascriptInterface
    fun startService(configJson: String) {
        val s = serviceProvider() ?: return
        try {
            val obj = JSONObject(configJson)
            val host = obj.optString("host", "0.0.0.0")
            val port = obj.optInt("port", 12340)
            val memory = obj.optString("memory", "512")
            val smp = obj.optString("smp", "2")
            val proxy = obj.optString("proxy", "")
            val logLevel = obj.optString("logLevel", "info")
            val refreshInterval = obj.optString("refreshInterval", "1800")

            s.startQemu(host, port, memory, smp, proxy, logLevel, refreshInterval)
        } catch (e: Exception) {
            evaluateJs("window.onAndroidLogEntry('[error] Bad config: ${e.message}')")
        }
    }

    @JavascriptInterface
    fun stopService() {
        serviceProvider()?.stopQemu()
    }

    @JavascriptInterface
    fun login(payloadJson: String) {
        val s = serviceProvider() ?: return
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val obj = JSONObject(payloadJson)
                val username = obj.optString("username", "")
                val password = obj.optString("password", "")
                val twoFactor = obj.optString("twoFactor", "")
                val proxy = obj.optString("proxy", "")
                val deviceInfo = obj.optString("deviceInfo", "")

                val (success, message) = s.runner.login(username, password, twoFactor, proxy, deviceInfo) { line ->
                    val safeLine = line.replace("\\", "\\\\").replace("'", "\\'").replace("\n", " ")
                    evaluateJs("window.onAndroidLogEntry('$safeLine')")
                }

                val resultJson = JSONObject().apply {
                    put("success", success)
                    put("need2FA", message == "2FA")
                    put("message", if (message == "2FA") "Two-factor authentication required" else message)
                }

                val safeResult = resultJson.toString().replace("'", "\\'")
                evaluateJs("window.onAndroidLoginResult('$safeResult')")
            } catch (e: Exception) {
                val errJson = JSONObject().apply {
                    put("success", false)
                    put("need2FA", false)
                    put("message", e.message ?: "Unknown error")
                }
                val safeErr = errJson.toString().replace("'", "\\'")
                evaluateJs("window.onAndroidLoginResult('$safeErr')")
            }
        }
    }

    @JavascriptInterface
    fun checkQemuStatus(): String {
        val s = serviceProvider() ?: return "{}"
        val statusMap = s.assetManager.checkStatus()
        val json = JSONObject()
        statusMap.forEach { (k, v) -> json.put(k, v) }
        return json.toString()
    }

    @JavascriptInterface
    fun downloadQemuPackage() {
        val s = serviceProvider() ?: return
        CoroutineScope(Dispatchers.Main).launch {
            s.assetManager.downloadQemuPackage { pct, speed, status ->
                evaluateJs("window.onAndroidDownloadProgress($pct, '$speed', '$status')")
            }
        }
    }

    @JavascriptInterface
    fun saveConfig(configJson: String) {
        prefs.edit().putString("config", configJson).apply()
    }

    @JavascriptInterface
    fun loadConfig(): String {
        return prefs.getString("config", "{}") ?: "{}"
    }

    @JavascriptInterface
    fun openBrowser(url: String) {
        try {
            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url)).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK
            }
            context.startActivity(intent)
        } catch (e: Exception) {}
    }
}
