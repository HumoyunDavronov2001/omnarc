package org.omnarc.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import java.io.File

/**
 * Keeps this device in the cluster: runs the node agent as a foreground service, so Android does not stop it
 * while the screen is off. The agent is the same binary as on every other device; the app only starts it.
 */
class NodeService : Service() {
    private var process: Process? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notification = notification()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIFICATION, notification)
        }
        if (process?.isAlive != true) process = startNode()
        return START_STICKY
    }

    override fun onDestroy() {
        process?.destroy()
        super.onDestroy()
    }

    private fun startNode(): Process {
        val lib = applicationInfo.nativeLibraryDir
        val data = File(filesDir, "omnarc").apply { mkdirs() }
        val builder = ProcessBuilder("$lib/libomnarc_node.so", "--data", data.path, "serve", "--panel", panel().path)
            .redirectErrorStream(true)
            .redirectOutput(ProcessBuilder.Redirect.appendTo(File(filesDir, "node.log")))
        builder.environment().apply {
            put("HOME", filesDir.path)
            put("TMPDIR", cacheDir.path)
            put("OMNARC_ENGINE", "$lib/libomnarc_engine.so")
        }
        return builder.start()
    }

    /** The panel's files, copied out of the APK once per install so the agent can serve them. */
    private fun panel(): File {
        val dir = File(filesDir, "panel")
        val stamp = File(dir, ".installed")
        val installed = packageManager.getPackageInfo(packageName, 0).lastUpdateTime.toString()
        if (stamp.exists() && stamp.readText() == installed) return dir
        dir.deleteRecursively()
        copyAssets("panel", dir)
        stamp.writeText(installed)
        return dir
    }

    private fun copyAssets(path: String, target: File) {
        val children = assets.list(path).orEmpty()
        if (children.isEmpty()) {
            target.parentFile?.mkdirs()
            assets.open(path).use { input -> target.outputStream().use { input.copyTo(it) } }
            return
        }
        target.mkdirs()
        for (child in children) copyAssets("$path/$child", File(target, child))
    }

    private fun notification(): Notification {
        getSystemService(NotificationManager::class.java)
            .createNotificationChannel(NotificationChannel(CHANNEL, "Omnarc node", NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL)
            .setContentTitle("Omnarc")
            .setContentText("This device is part of the cluster")
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    private companion object {
        const val CHANNEL = "node"
        const val NOTIFICATION = 1
    }
}
