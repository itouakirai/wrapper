package com.worldobservationlog.wrapperlite

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.webkit.JavascriptInterface
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

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
    fun hasLoginCache(): Boolean {
        val qemuDir = java.io.File(context.filesDir, "qemu")
        if (java.io.File(context.filesDir, ".login_cached").exists() || java.io.File(qemuDir, ".login_cached").exists()) {
            return true
        }
        val tokenFiles = listOf(
            java.io.File(context.filesDir, "token_cache.json"),
            java.io.File(context.filesDir, "DEV_TOKEN"),
            java.io.File(context.filesDir, "MUSIC_TOKEN"),
            java.io.File(qemuDir, "token_cache.json")
        )
        return tokenFiles.any { it.exists() }
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
    fun getServiceStatus(host: String, port: Int): String {
        val targetHost = if (host.isBlank() || host == "0.0.0.0") "127.0.0.1" else host
        val urlStr = "http://$targetHost:$port/status"
        val startTime = System.currentTimeMillis()
        var conn: HttpURLConnection? = null
        return try {
            val url = URL(urlStr)
            conn = (url.openConnection() as HttpURLConnection).apply {
                connectTimeout = 2000
                readTimeout = 2000
                requestMethod = "GET"
                instanceFollowRedirects = false
            }
            val code = conn.responseCode
            val latency = System.currentTimeMillis() - startTime
            if (code == 200) {
                val body = conn.inputStream.bufferedReader().use { it.readText() }
                val json = JSONObject(body)
                json.put("online", true)
                json.put("latency", latency)
                json.toString()
            } else {
                JSONObject().apply {
                    put("online", false)
                    put("latency", latency)
                    put("status", code)
                }.toString()
            }
        } catch (e: Exception) {
            val latency = System.currentTimeMillis() - startTime
            JSONObject().apply {
                put("online", false)
                put("latency", latency)
                put("message", e.message ?: "Connection failed")
            }.toString()
        } finally {
            conn?.disconnect()
        }
    }

    @JavascriptInterface
    fun proxyRequest(targetUrl: String, method: String, postBody: String?): String {
        var conn: HttpURLConnection? = null
        return try {
            val url = URL(targetUrl)
            conn = (url.openConnection() as HttpURLConnection).apply {
                connectTimeout = 10000
                readTimeout = 30000
                requestMethod = method.uppercase()
                if (method.equals("POST", ignoreCase = true) && !postBody.isNullOrEmpty()) {
                    doOutput = true
                    setRequestProperty("Content-Type", "application/json")
                    outputStream.use { os ->
                        os.write(postBody.toByteArray(Charsets.UTF_8))
                    }
                }
            }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val body = stream?.bufferedReader()?.use { it.readText() } ?: ""
            JSONObject().apply {
                put("status", code)
                put("statusText", conn.responseMessage ?: "OK")
                put("body", body)
            }.toString()
        } catch (e: Exception) {
            JSONObject().apply {
                put("status", 502)
                put("statusText", "Bad Gateway")
                put("body", "Proxy Error: ${e.message}")
            }.toString()
        } finally {
            conn?.disconnect()
        }
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
