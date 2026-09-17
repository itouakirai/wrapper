package com.worldobservationlog.wrapperlite

import android.content.Context
import android.os.Build
import android.util.Base64
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.File
import java.io.InputStreamReader
import java.util.Collections
import java.util.concurrent.atomic.AtomicBoolean

class QemuRunner(private val context: Context, private val assetManager: QemuAssetManager) {

    companion object {
        private const val TAG = "QemuRunner"
    }

    private var process: Process? = null
    var isRunning: Boolean = false
        private set

    fun start(
        host: String = "0.0.0.0",
        port: Int = 12340,
        memory: String = "512",
        smp: String = "2",
        proxy: String = "",
        logLevel: String = "info",
        refreshInterval: String = "1800",
        onLog: (String) -> Unit
    ): Boolean {
        if (isRunning) return false

        val qemuBin = assetManager.getQemuExecutable()
        val kernel = assetManager.getKernel()
        val initramfs = assetManager.getInitramfs()
        val disk = assetManager.getDataDisk()
        val binDir = assetManager.binDir

        if (!qemuBin.exists() || !kernel.exists() || !initramfs.exists() || !disk.exists()) {
            onLog("[error] Missing QEMU components. Please download QEMU package from the 'QEMU Package' tab.")
            return false
        }

        // Build guest lite arguments
        val guestArgsList = mutableListOf(
            "--base-dir", "/data",
            "--host", "0.0.0.0",
            "--port", "12340"
        )
        if (proxy.isNotBlank()) {
            guestArgsList.add("--proxy")
            guestArgsList.add(proxy)
        }
        if (logLevel.isNotBlank()) {
            guestArgsList.add("--log-level")
            guestArgsList.add(logLevel)
        }
        if (refreshInterval.isNotBlank()) {
            guestArgsList.add("--token-refresh-interval")
            guestArgsList.add(refreshInterval)
        }

        val guestArgsStr = guestArgsList.joinToString("\n")
        val b64Args = Base64.encodeToString(guestArgsStr.toByteArray(Charsets.UTF_8), Base64.NO_WRAP)
        val appendStr = "console=ttyS0 quiet net.ifnames=0 biosdevname=0 lite_args_b64=$b64Args"

        val argsFile = File(assetManager.qemuDir, ".lite-qemu-args")
        try {
            argsFile.writeText(guestArgsStr)
        } catch (e: Exception) {}

        // Build QEMU arguments
        val cmd = mutableListOf(
            qemuBin.absolutePath,
            "-L", binDir.absolutePath,
            "-accel", "tcg",
            "-cpu", "max",
            "-m", memory,
            "-smp", smp,
            "-kernel", kernel.absolutePath,
            "-initrd", initramfs.absolutePath,
            "-append", appendStr,
            "-display", "none",
            "-serial", "stdio",
            "-no-reboot",
            "-nic", "user,model=e1000,hostfwd=tcp:$host:$port-:12340",
            "-drive", "file=${disk.absolutePath},format=raw,if=virtio"
        )
        if (argsFile.exists()) {
            cmd.add("-fw_cfg")
            cmd.add("name=lite_args,file=${argsFile.absolutePath}")
        }

        val pb = ProcessBuilder(cmd)
        pb.directory(assetManager.qemuDir)

        val env = pb.environment()
        setupEnvironment(env, binDir)

        try {
            onLog("[run] Starting headless QEMU (guest forwarding to $host:$port, mem ${memory}MB)...")
            val proc = pb.start()
            process = proc
            isRunning = true

            // Read stdout & stderr
            Thread {
                val reader = BufferedReader(InputStreamReader(proc.inputStream))
                var line: String?
                while (reader.readLine().also { line = it } != null) {
                    line?.let {
                        if (!it.contains("request: GET /status")) {
                            onLog(it)
                        }
                    }
                }
            }.start()

            Thread {
                val errReader = BufferedReader(InputStreamReader(proc.errorStream))
                var line: String?
                while (errReader.readLine().also { line = it } != null) {
                    line?.let {
                        if (!it.contains("request: GET /status")) {
                            onLog(it)
                        }
                    }
                }
            }.start()

            Thread {
                val exitCode = proc.waitFor()
                isRunning = false
                process = null
                argsFile.delete()
                onLog("[run] QEMU process exited with code $exitCode")
            }.start()

            return true
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start QEMU", e)
            onLog("[error] Failed to start QEMU process: ${e.message}")
            isRunning = false
            argsFile.delete()
            return false
        }
    }

    private fun is2FALine(line: String): Boolean {
        return line.contains("need2FA: true") ||
               line.contains("2FA: true") ||
               line.contains("2FA code") ||
               line.contains("requiresHSA2VerificationCode") ||
               line.contains("Enter your 2FA code into")
    }

    fun login(
        username: String,
        password: String,
        twoFactor: String = "",
        proxy: String = "",
        deviceInfo: String = "",
        onLog: (String) -> Unit
    ): Pair<Boolean, String> {
        if (isRunning) {
            return Pair(false, "Cannot login while service is running. Stop service first.")
        }

        val qemuBin = assetManager.getQemuExecutable()
        val kernel = assetManager.getKernel()
        val initramfs = assetManager.getInitramfs()
        val disk = assetManager.getDataDisk()
        val binDir = assetManager.binDir

        if (!qemuBin.exists() || !kernel.exists() || !initramfs.exists() || !disk.exists()) {
            return Pair(false, "Missing QEMU components. Please download QEMU package from the 'QEMU Package' tab.")
        }

        val fullPassword = if (twoFactor.isNotBlank()) password + twoFactor else password
        val loginCred = "$username:$fullPassword"

        val guestArgsList = mutableListOf(
            "--base-dir", "/data",
            "--login", loginCred,
            "--code-from-file"
        )
        if (proxy.isNotBlank()) {
            guestArgsList.add("--proxy")
            guestArgsList.add(proxy)
        }
        if (deviceInfo.isNotBlank()) {
            guestArgsList.add("--device-info")
            guestArgsList.add(deviceInfo)
        }

        val guestArgsStr = guestArgsList.joinToString("\n")
        val b64Args = Base64.encodeToString(guestArgsStr.toByteArray(Charsets.UTF_8), Base64.NO_WRAP)
        val appendStr = "console=ttyS0 quiet net.ifnames=0 biosdevname=0 lite_args_b64=$b64Args"

        val argsFile = File(assetManager.qemuDir, ".lite-qemu-args")
        try {
            argsFile.writeText(guestArgsStr)
        } catch (e: Exception) {}

        val cmd = mutableListOf(
            qemuBin.absolutePath,
            "-L", binDir.absolutePath,
            "-accel", "tcg",
            "-cpu", "max",
            "-m", "512",
            "-smp", "2",
            "-kernel", kernel.absolutePath,
            "-initrd", initramfs.absolutePath,
            "-append", appendStr,
            "-display", "none",
            "-serial", "stdio",
            "-no-reboot",
            "-nic", "user,model=e1000",
            "-drive", "file=${disk.absolutePath},format=raw,if=virtio"
        )
        if (argsFile.exists()) {
            cmd.add("-fw_cfg")
            cmd.add("name=lite_args,file=${argsFile.absolutePath}")
        }

        val pb = ProcessBuilder(cmd)
        pb.directory(assetManager.qemuDir)

        val env = pb.environment()
        setupEnvironment(env, binDir)

        val outputLines = Collections.synchronizedList(mutableListOf<String>())
        val need2FADetected = AtomicBoolean(false)

        try {
            onLog("[auth] Running Apple Music login in QEMU guest for account $username...")
            val proc = pb.start()

            val checkAndTrigger2FA = { lineText: String ->
                if (!need2FADetected.get() && is2FALine(lineText)) {
                    if (need2FADetected.compareAndSet(false, true)) {
                        onLog("[auth] Apple 2FA requirement detected. Terminating QEMU guest early to prompt for verification code...")
                        try {
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                                proc.destroyForcibly()
                            } else {
                                proc.destroy()
                            }
                            Unit
                        } catch (e: Exception) {
                            Log.e(TAG, "Failed to terminate QEMU process on 2FA detection", e)
                        }
                    }
                }
                Unit
            }

            val stdoutThread = Thread {
                val reader = BufferedReader(InputStreamReader(proc.inputStream))
                var line: String?
                while (reader.readLine().also { line = it } != null) {
                    line?.let {
                        outputLines.add(it)
                        onLog(it)
                        checkAndTrigger2FA(it)
                    }
                }
            }
            val stderrThread = Thread {
                val errReader = BufferedReader(InputStreamReader(proc.errorStream))
                var line: String?
                while (errReader.readLine().also { line = it } != null) {
                    line?.let {
                        outputLines.add(it)
                        onLog(it)
                        checkAndTrigger2FA(it)
                    }
                }
            }
            stdoutThread.start()
            stderrThread.start()

            val exitCode = proc.waitFor()
            stdoutThread.join(2000)
            stderrThread.join(2000)
            argsFile.delete()

            val fullOutput = synchronized(outputLines) { outputLines.joinToString("\n") }
            if (need2FADetected.get() || is2FALine(fullOutput)) {
                return Pair(false, "2FA")
            }
            if (exitCode == 0 && (fullOutput.contains("login successful") || fullOutput.contains("login complete") || fullOutput.contains("Tokens cached"))) {
                try {
                    File(context.filesDir, ".login_cached").writeText(System.currentTimeMillis().toString())
                    File(assetManager.qemuDir, ".login_cached").writeText(System.currentTimeMillis().toString())
                } catch (e: Exception) {}
                return Pair(true, "Login successful! Decryption tokens cached.")
            } else {
                return Pair(false, "Login exited with code $exitCode. Check credentials and logs.")
            }
        } catch (e: Exception) {
            argsFile.delete()
            return Pair(false, "Login failed: ${e.message}")
        }
    }

    fun stop() {
        val proc = process
        if (proc != null) {
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    proc.destroyForcibly()
                } else {
                    proc.destroy()
                }
                Unit
            } catch (e: Exception) {
                Log.e(TAG, "Error destroying QEMU process", e)
            }
        }
        isRunning = false
        process = null
    }

    private fun setupEnvironment(env: MutableMap<String, String>, binDir: File) {
        val ldPaths = mutableListOf<String>()
        val qemuExe = assetManager.getQemuExecutable()
        if (qemuExe.parentFile != null && qemuExe.parentFile.exists()) {
            ldPaths.add(qemuExe.parentFile.absolutePath)
        }
        ldPaths.add(binDir.absolutePath)
        val qemuLibDir = File(assetManager.qemuDir, "lib")
        if (qemuLibDir.exists()) ldPaths.add(qemuLibDir.absolutePath)
        val filesLibDir = File(context.filesDir, "lib")
        if (filesLibDir.exists()) ldPaths.add(filesLibDir.absolutePath)
        val filesBinDir = File(context.filesDir, "bin")
        if (filesBinDir.exists()) ldPaths.add(filesBinDir.absolutePath)
        val nativeLibDir = context.applicationInfo.nativeLibraryDir
        if (!nativeLibDir.isNullOrBlank()) {
            ldPaths.add(nativeLibDir)
        }
        val existingLd = env["LD_LIBRARY_PATH"]
        if (!existingLd.isNullOrBlank()) {
            ldPaths.add(existingLd)
        }
        env["LD_LIBRARY_PATH"] = ldPaths.filter { it.isNotBlank() }.distinct().joinToString(":")
        env["QEMU_MODULE_DIR"] = binDir.absolutePath
        env["TMPDIR"] = context.cacheDir.absolutePath
        if (env["PATH"].isNullOrBlank()) {
            env["PATH"] = "/system/bin:/system/xbin"
        }
    }
}
