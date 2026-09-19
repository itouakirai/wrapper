package com.worldobservationlog.wrapperlite

import android.content.Context
import android.os.Build
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
        val candidates = listOf(
            File(qemuDir, "vmlinuz-lite-qemu"),
            File(context.filesDir, "vmlinuz-lite-qemu"),
            File(binDir, "vmlinuz-lite-qemu")
        )
        return candidates.firstOrNull { it.exists() } ?: File(qemuDir, "vmlinuz-lite-qemu")
    }

    fun getInitramfs(): File {
        val canonicalCandidates = listOf(
            File(qemuDir, "lite-initramfs.cpio.gz"),
            File(context.filesDir, "lite-initramfs.cpio.gz"),
            File(binDir, "lite-initramfs.cpio.gz")
        )
        val found = canonicalCandidates.firstOrNull { it.exists() }
        if (found != null) return found

        val strippedCandidates = listOf(
            File(qemuDir, "lite-initramfs.cpio"),
            File(context.filesDir, "lite-initramfs.cpio"),
            File(binDir, "lite-initramfs.cpio")
        )
        val stripped = strippedCandidates.firstOrNull { it.exists() }
        if (stripped != null) {
            val targetGz = File(stripped.parentFile, "lite-initramfs.cpio.gz")
            try {
                if (stripped.renameTo(targetGz)) {
                    return targetGz
                }
            } catch (e: Exception) {
                Log.w(TAG, "Failed to auto-rename lite-initramfs.cpio to lite-initramfs.cpio.gz", e)
            }
            return stripped
        }
        return File(qemuDir, "lite-initramfs.cpio.gz")
    }

    fun getDataDisk(): File {
        val candidates = listOf(
            File(qemuDir, "data.img"),
            File(context.filesDir, "data.img")
        )
        return candidates.firstOrNull { it.exists() } ?: File(qemuDir, "data.img")
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

    private fun isElfFile(file: File): Boolean {
        if (!file.exists() || !file.isFile || file.length() < 4) return false
        return try {
            file.inputStream().use { input ->
                val header = ByteArray(4)
                if (input.read(header) == 4) {
                    header[0] == 0x7f.toByte() &&
                    header[1] == 'E'.code.toByte() &&
                    header[2] == 'L'.code.toByte() &&
                    header[3] == 'F'.code.toByte()
                } else false
            }
        } catch (e: Exception) {
            false
        }
    }

    fun checkStatus(): Map<String, Boolean> {
        val qemuExe = getQemuExecutable()
        val exeParent = qemuExe.parentFile ?: binDir
        val hasSoInBin = binDir.listFiles { _, name -> name.endsWith(".so") || name.contains(".so.") }?.isNotEmpty() == true
        val hasSoInParent = exeParent.listFiles { _, name -> name.endsWith(".so") || name.contains(".so.") }?.isNotEmpty() == true
        val pixmanLib = File(exeParent, "libpixman-1.so").exists() || File(binDir, "libpixman-1.so").exists()
        val librariesReady = pixmanLib || hasSoInBin || hasSoInParent || (qemuExe.exists() && qemuExe.length() > 5000000)
        val qemuBin = qemuExe.exists() && librariesReady
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

    fun fixPermissionsAndLibraries(baseDir: File) {
        if (!baseDir.exists()) return

        // Pass 1: resolve pseudo-symlinks (plain text pointer files from zip extraction)
        baseDir.walkTopDown().filter { it.isFile }.forEach { f ->
            if (f.name.contains(".so") && !isElfFile(f) && f.length() in 1..1024) {
                try {
                    val visited = mutableSetOf<File>(f)
                    var currentTargetName = f.readText(Charsets.UTF_8).trim()
                    var realElfFile: File? = null

                    while (currentTargetName.isNotEmpty() && visited.size < 10) {
                        var candidate = File(f.parentFile, currentTargetName)
                        if (!candidate.exists()) {
                            candidate = File(f.parentFile, File(currentTargetName).name)
                        }
                        if (!candidate.exists() && baseDir != f.parentFile) {
                            candidate = File(baseDir, File(currentTargetName).name)
                        }
                        if (!candidate.exists() || !candidate.isFile || !visited.add(candidate)) {
                            break
                        }
                        if (isElfFile(candidate)) {
                            realElfFile = candidate
                            break
                        }
                        if (candidate.length() in 1..1024) {
                            currentTargetName = candidate.readText(Charsets.UTF_8).trim()
                        } else {
                            break
                        }
                    }

                    if (realElfFile != null && realElfFile.exists()) {
                        Log.d(TAG, "Resolving pseudo-symlink ${f.name} -> ${realElfFile.name}")
                        realElfFile.copyTo(f, overwrite = true)
                    }
                } catch (e: Exception) {
                    Log.w(TAG, "Failed resolving pseudo-symlink for ${f.name}", e)
                }
            }
        }

        // Pass 2: batch set executable permissions via recursive chmod, with individual fallback
        try {
            Runtime.getRuntime().exec(arrayOf("chmod", "-R", "755", baseDir.absolutePath)).waitFor()
        } catch (e: Exception) {
            // best effort
        }

        baseDir.walkTopDown().filter { it.isFile }.forEach { f ->
            val name = f.name
            val parentName = f.parentFile?.name ?: ""
            if (parentName in listOf("bin", "lib") ||
                name.contains(".so") ||
                name.startsWith("qemu-system-") ||
                name == "wrapper-lite-qemu" ||
                name == "wrapper-lite-gui"
            ) {
                f.setExecutable(true, false)
                f.setReadable(true, false)
            }
        }
    }

    /**
     * Extracts prebundled assets from APK assets/qemu if present.
     */
    suspend fun extractBundledAssetsIfNeeded(): Boolean = withContext(Dispatchers.IO) {
        try {
            val assetManager = context.assets
            val list = assetManager.list("qemu") ?: return@withContext false
            if (list.isEmpty()) return@withContext false

            val prefs = context.getSharedPreferences("wl_asset_prefs", Context.MODE_PRIVATE)
            val packageInfo = try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    context.packageManager.getPackageInfo(context.packageName, android.content.pm.PackageManager.PackageInfoFlags.of(0))
                } else {
                    @Suppress("DEPRECATION")
                    context.packageManager.getPackageInfo(context.packageName, 0)
                }
            } catch (e: Exception) { null }
            val currentVersionCode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                packageInfo?.longVersionCode ?: 1L
            } else {
                @Suppress("DEPRECATION")
                packageInfo?.versionCode?.toLong() ?: 1L
            }
            val lastExtractedVersion = prefs.getLong("extracted_version_code", -1L)

            val qemuExe = getQemuExecutable()
            val kernel = getKernel()
            val initramfs = getInitramfs()
            val disk = getDataDisk()
            if (lastExtractedVersion == currentVersionCode && qemuExe.exists() && kernel.exists() && initramfs.exists() && disk.exists()) {
                // Assets already fully extracted and up to date; skip expensive I/O and chmod
                return@withContext true
            }

            qemuDir.mkdirs()
            binDir.mkdirs()

            copyAssetFolder("qemu", qemuDir)

            // Auto-heal: In case AAPT/AAPT2 stripped .gz from lite-initramfs.cpio.gz during packaging,
            // or an older version already unpacked it as lite-initramfs.cpio
            val strippedInitramfs = File(qemuDir, "lite-initramfs.cpio")
            val canonicalInitramfs = File(qemuDir, "lite-initramfs.cpio.gz")
            if (strippedInitramfs.exists() && !canonicalInitramfs.exists()) {
                try {
                    strippedInitramfs.renameTo(canonicalInitramfs)
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to rename stripped initramfs asset", e)
                }
            }

            // Ensure executable permissions and resolve libraries
            fixPermissionsAndLibraries(qemuDir)
            fixPermissionsAndLibraries(context.filesDir)

            val resolvedQemuExe = getQemuExecutable()
            makeExecutable(resolvedQemuExe)

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

            prefs.edit().putLong("extracted_version_code", currentVersionCode).apply()
            true
        } catch (e: Exception) {
            Log.e(TAG, "Error extracting bundled assets", e)
            false
        }
    }

    private fun copyAssetFolder(srcName: String, dstDir: File) {
        val assetManager = context.assets
        try {
            // Try opening as a file stream first
            assetManager.open(srcName).use { input ->
                var targetFile = dstDir
                // AAPT/AAPT2 automatically strips the ".gz" extension from asset filenames when packaging APK.
                // Restore canonical filename when writing to disk.
                if (targetFile.name == "lite-initramfs.cpio") {
                    targetFile = File(targetFile.parentFile, "lite-initramfs.cpio.gz")
                }
                if (targetFile.name == "data.img" && targetFile.exists() && targetFile.length() > 0) {
                    return
                }
                targetFile.parentFile?.mkdirs()
                FileOutputStream(targetFile).use { output ->
                    input.copyTo(output)
                }
            }
        } catch (e: Exception) {
            // It is a directory: recurse over children
            val children = assetManager.list(srcName) ?: return
            if (children.isNotEmpty()) {
                dstDir.mkdirs()
                for (child in children) {
                    val subSrc = if (srcName.isEmpty()) child else "$srcName/$child"
                    val destChildName = if (child == "lite-initramfs.cpio") "lite-initramfs.cpio.gz" else child
                    copyAssetFolder(subSrc, File(dstDir, destChildName))
                }
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

            // Ensure executable permissions and resolve libraries
            fixPermissionsAndLibraries(context.filesDir)
            fixPermissionsAndLibraries(qemuDir)

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
                val extractedFile = File(destDir, entry.name)
                // Prevent Zip Slip
                val canonicalDest = destDir.canonicalPath
                val canonicalOut = extractedFile.canonicalPath
                if (!canonicalOut.startsWith(canonicalDest + File.separator) && canonicalOut != canonicalDest) {
                    entry = zis.nextEntry
                    continue
                }

                val outFile = if (extractedFile.name == "lite-initramfs.cpio") {
                    File(extractedFile.parentFile, "lite-initramfs.cpio.gz")
                } else {
                    extractedFile
                }

                if (entry.isDirectory) {
                    outFile.mkdirs()
                } else {
                    // Do not overwrite existing user data disk
                    if (outFile.name == "data.img" && outFile.exists() && outFile.length() > 0) {
                        zis.closeEntry()
                        entry = zis.nextEntry
                        continue
                    }
                    outFile.parentFile?.mkdirs()
                    FileOutputStream(outFile).use { fos ->
                        zis.copyTo(fos)
                    }
                    val name = entry.name
                    if (name.contains("bin/") || name.contains("lib/") ||
                        name.contains(".so") || name.contains("qemu-system-") ||
                        name.endsWith("wrapper-lite-qemu") || name.endsWith("wrapper-lite-gui")) {
                        outFile.setExecutable(true, false)
                        outFile.setReadable(true, false)
                    }
                }
                zis.closeEntry()
                entry = zis.nextEntry
            }
        }
    }
}
