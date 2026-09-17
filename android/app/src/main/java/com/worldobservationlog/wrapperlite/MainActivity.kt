package com.worldobservationlog.wrapperlite

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.webkit.JsResult
import android.webkit.WebChromeClient
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONObject

class MainActivity : AppCompatActivity() {

    private lateinit var webView: WebView
    private var qemuService: QemuService? = null
    private var isBound = false

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            val binder = service as QemuService.LocalBinder
            qemuService = binder.getService()
            isBound = true

            // Attach listeners
            qemuService?.onLogListener = { line ->
                if (!line.contains("request: GET /status")) {
                    runOnUiThread {
                        val safeLine = line.replace("\\", "\\\\").replace("'", "\\'").replace("\n", " ")
                        webView.evaluateJavascript("window.onAndroidLogEntry('$safeLine')", null)
                    }
                }
            }

            qemuService?.onStatusListener = { running ->
                runOnUiThread {
                    val status = JSONObject().apply {
                        put("running", running)
                    }
                    webView.evaluateJavascript("window.onAndroidStatusUpdate('$status')", null)
                }
            }

            // Extract bundled assets if present
            CoroutineScope(Dispatchers.IO).launch {
                qemuService?.assetManager?.extractBundledAssetsIfNeeded()
            }
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            qemuService = null
            isBound = false
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Request notification permission on Android 13+
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED) {
                ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 101)
            }
        }

        // Start & Bind QemuService
        val serviceIntent = Intent(this, QemuService::class.java)
        startService(serviceIntent)
        bindService(serviceIntent, connection, Context.BIND_AUTO_CREATE)

        // Set up WebView
        webView = WebView(this)
        setContentView(webView)

        val settings: WebSettings = webView.settings
        settings.javaScriptEnabled = true
        settings.domStorageEnabled = true
        settings.allowFileAccess = true
        settings.allowContentAccess = true
        settings.allowFileAccessFromFileURLs = true
        settings.allowUniversalAccessFromFileURLs = true
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            settings.mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
        }
        settings.loadWithOverviewMode = true
        settings.useWideViewPort = true
        settings.cacheMode = WebSettings.LOAD_DEFAULT

        webView.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView?, url: String?) {
                super.onPageFinished(view, url)
                // Initialize Android mode in UI
                webView.evaluateJavascript("detectPlatform()", null)
            }
        }

        webView.webChromeClient = object : WebChromeClient() {
            override fun onJsAlert(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean {
                if (isFinishing || isDestroyed) {
                    result?.cancel()
                    return true
                }
                AlertDialog.Builder(this@MainActivity)
                    .setTitle(R.string.app_name)
                    .setMessage(message ?: "")
                    .setPositiveButton(android.R.string.ok) { _, _ -> result?.confirm() }
                    .setOnCancelListener { result?.confirm() }
                    .show()
                return true
            }

            override fun onJsConfirm(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean {
                if (isFinishing || isDestroyed) {
                    result?.cancel()
                    return true
                }
                AlertDialog.Builder(this@MainActivity)
                    .setTitle(R.string.app_name)
                    .setMessage(message ?: "")
                    .setPositiveButton(android.R.string.ok) { _, _ -> result?.confirm() }
                    .setNegativeButton(android.R.string.cancel) { _, _ -> result?.cancel() }
                    .setOnCancelListener { result?.cancel() }
                    .show()
                return true
            }
        }

        val bridge = WebAppInterface(
            context = this,
            serviceProvider = { qemuService },
            evaluateJs = { js ->
                runOnUiThread {
                    try {
                        if (!isFinishing && !isDestroyed) {
                            webView.evaluateJavascript(js, null)
                        }
                    } catch (e: Throwable) {
                        android.util.Log.e("MainActivity", "Failed to evaluate JS", e)
                    }
                }
            }
        )
        webView.addJavascriptInterface(bridge, "Android")

        webView.loadUrl("file:///android_asset/web/index.html")
    }

    override fun onBackPressed() {
        if (webView.canGoBack()) {
            webView.goBack()
        } else {
            super.onBackPressed()
        }
    }

    override fun onDestroy() {
        if (isBound) {
            unbindService(connection)
            isBound = false
        }
        super.onDestroy()
    }
}
