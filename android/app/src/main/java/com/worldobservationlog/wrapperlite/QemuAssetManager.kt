package com.worldobservationlog.wrapperlite

import android.content.Context
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.BufferedInputStream
import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.zip.ZipInputStream

class QemuAssetManager(private val context: Context) {

    companion object {
        private const val TAG = "QemuAssetManager"
        const val NIGHTLY_ANDROID_ZIP =
            "https://nightly.link/itouakirai/wrapper/workflows/build-lite/lite/wrapper-lite-qemu-android-aarch64.zip"
    }

    val qemuDir: File = File(context.filesDir, "qemu")
    val binDir: File = File(qemuDir, "bin")

    fun getQemuExecutable(): File {
        val candidates = listOf(
            File(binDir, "qemu-system-x86_64"),
            File(binDir, "qemu-system-x86-64-headless"),
            File(binDir, "qemu-system-x86_64-headless"),
            File(qemuDir, "bin/qemu-system-x86_64"),
            File(qemuDir, "qemu-system-x86_64"),
            File(context.filesDir, "bin/qemu-system-x86_64"),
            File(context.filesDir, "qemu-system-x86_64")
        )
        return candidates.firstOrNull { it.exists() } ?: File(binDir, "qemu-system-x86_64")
    }

    fun getLauncherExecutable(): File {
        val candidates = listOf(
            File(context.filesDir, "wrapper-lite-qemu"),
            File(qemuDir, "wrapper-lite-qemu"),
            File(binDir, "wrapper-lite-qemu")
        )
        return candidates.firstOrNull { it.exists() } ?: File(context.filesDir, "wrapper-lite-qemu")
    }

    fun getKernel(): File {
        return File(qemuDir, "vmlinuz-lite-qemu")
    }

    fun getInitramfs(): File {
        return File(qemuDir, "lite-initramfs.cpio.gz")
    }

    fun getDataDisk(): File {
        return File(qemuDir, "data.img")
    }

    fun makeExecutable(file: File) {
        if (!file.exists()) return
        file.setExecutable(true, false)
        file.setReadable(true, false)
        try {
            Runtime.getRuntime().exec(arrayOf("chmod", "755", file.absolutePath)).waitFor()
        } catch (e: Exception) {
            // best effort
        }
    }

    fun checkStatus(): Map<String, Boolean> {
        val qemuBin = getQemuExecutable().exists() && (getQemuExecutable().canExecute() || true)
        val launcher = getLauncherExecutable().exists()
        val kernel = getKernel().exists()
        val initramfs = getInitramfs().exists()
        val disk = getDataDisk().exists()
        val allReady = qemuBin && kernel && initramfs && disk

        return mapOf(
            "qemuBin" to qemuBin,
            "launcher" to launcher,
            "kernel" to kernel,
            "initramfs" to initramfs,
            "disk" to disk,
            "allPresent" to allReady
        )
    }

    /**
     * Extracts prebundled assets from APK assets/qemu if present.
     */
    suspend fun extractBundledAssetsIfNeeded(): Boolean = withContext(Dispatchers.IO) {
        try {
            val assetManager = context.assets
            val list = assetManager.list("qemu") ?: return@withContext false
            if (list.isEmpty()) return@withContext false

            qemuDir.mkdirs()
            binDir.mkdirs()

            copyAssetFolder("qemu", qemuDir)

            // Ensure executable permissions for QEMU and launcher
            val qemuExe = getQemuExecutable()
            makeExecutable(qemuExe)

            val launcher = getLauncherExecutable()
            makeExecutable(launcher)

            // If launcher was extracted into qemuDir, also sync to filesDir
            val rootLauncher = File(context.filesDir, "wrapper-lite-qemu")
            val qemuLauncher = File(qemuDir, "wrapper-lite-qemu")
            if (qemuLauncher.exists() && !rootLauncher.exists()) {
                try {
                    qemuLauncher.copyTo(rootLauncher, overwrite = true)
                    makeExecutable(rootLauncher)
                } catch (e: Exception) {}
            }

            binDir.listFiles()?.forEach { f ->
                if (f.isFile) makeExecutable(f)
            }
            true
        } catch (e: Exception) {
            Log.e(TAG, "Error extracting bundled assets", e)
            false
        }
    }

    private fun copyAssetFolder(srcName: String, dstDir: File) {
        val assetManager = context.assets
        val fileList = assetManager.list(srcName) ?: return
        if (fileList.isEmpty()) {
            // It's a file
            copyAssetFile(srcName, dstDir)
        } else {
            dstDir.mkdirs()
            for (filename in fileList) {
                copyAssetFolder("$srcName/$filename", File(dstDir, filename))
            }
        }
    }

    private fun copyAssetFile(srcName: String, dstFile: File) {
        context.assets.open(srcName).use { input ->
            FileOutputStream(dstFile).use { output ->
                input.copyTo(output)
            }
        }
    }

    /**
     * Downloads the precompiled Android package from nightly.link with progress callbacks.
     */
    suspend fun downloadQemuPackage(
        onProgress: (percent: Int, speed: String, status: String) -> Unit
    ): Boolean = withContext(Dispatchers.IO) {
        try {
            onProgress(0, "", "Connecting to nightly server...")

            var currentUrl = NIGHTLY_ANDROID_ZIP
            var conn: HttpURLConnection
            var redirects = 0
            while (true) {
                val url = URL(currentUrl)
                conn = url.openConnection() as HttpURLConnection
                conn.instanceFollowRedirects = true
                conn.connectTimeout = 15000
                conn.readTimeout = 30000
                val code = conn.responseCode
                if (code in 300..399 && redirects < 5) {
                    val loc = conn.getHeaderField("Location")
                    if (loc != null) {
                        currentUrl = loc
                        redirects++
                        conn.disconnect()
                        continue
                    }
                }
                break
            }

            val totalSize = conn.contentLength.toLong()
            val tempZip = File(context.cacheDir, "qemu_android.zip")

            BufferedInputStream(conn.inputStream).use { input ->
                FileOutputStream(tempZip).use { output ->
                    val buffer = ByteArray(64 * 1024)
                    var bytesRead: Int
                    var totalRead: Long = 0
                    var lastTime = System.currentTimeMillis()
                    var lastBytes: Long = 0

                    while (input.read(buffer).also { bytesRead = it } != -1) {
                        output.write(buffer, 0, bytesRead)
                        totalRead += bytesRead

                        val now = System.currentTimeMillis()
                        if (now - lastTime >= 500) {
                            val elapsedSec = (now - lastTime) / 1000.0
                            val speedKb = ((totalRead - lastBytes) / elapsedSec / 1024).toLong()
                            val pct = if (totalSize > 0) ((totalRead * 100) / totalSize).toInt() else 0
                            onProgress(pct, "$speedKb KB/s", "Downloading QEMU package ($pct%)...")
                            lastTime = now
                            lastBytes = totalRead
                        }
                    }
                }
            }

            onProgress(99, "", "Extracting and verifying QEMU assets...")
            unzip(tempZip, context.filesDir)
            tempZip.delete()

            // Ensure executable permissions
            makeExecutable(getQemuExecutable())
            makeExecutable(getLauncherExecutable())

            val rootLauncher = File(context.filesDir, "wrapper-lite-qemu")
            val qemuLauncher = File(qemuDir, "wrapper-lite-qemu")
            if (qemuLauncher.exists() && !rootLauncher.exists()) {
                try {
                    qemuLauncher.copyTo(rootLauncher, overwrite = true)
                    makeExecutable(rootLauncher)
                } catch (e: Exception) {}
            }

            binDir.listFiles()?.forEach { f ->
                if (f.isFile) makeExecutable(f)
            }

            onProgress(100, "", "Installation completed!")
            true
        } catch (e: Exception) {
            Log.e(TAG, "Download failed", e)
            onProgress(0, "", "Error: ${e.message}")
            false
        }
    }

    private fun unzip(zipFile: File, destDir: File) {
        ZipInputStream(zipFile.inputStream().buffered()).use { zis ->
            var entry = zis.nextEntry
            while (entry != null) {
                val outFile = File(destDir, entry.name)
                // Prevent Zip Slip
                val canonicalDest = destDir.canonicalPath
                val canonicalOut = outFile.canonicalPath
                if (!canonicalOut.startsWith(canonicalDest + File.separator) && canonicalOut != canonicalDest) {
                    entry = zis.nextEntry
                    continue
                }

                if (entry.isDirectory) {
                    outFile.mkdirs()
                } else {
                    outFile.parentFile?.mkdirs()
                    FileOutputStream(outFile).use { fos ->
                        zis.copyTo(fos)
                    }
                    val name = entry.name
                    if (name.contains("bin/") || name.endsWith("wrapper-lite-qemu") ||
                        name.endsWith("qemu-system-x86_64") || name.endsWith(".so")) {
                        makeExecutable(outFile)
                    }
                }
                zis.closeEntry()
                entry = zis.nextEntry
            }
        }
    }
}
