package org.omnarc.app

import android.app.ActivityManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.BatteryManager

/** What this phone can tell about itself right now: read from Android, never estimated. */
data class DeviceState(val charging: Boolean, val wifi: Boolean, val temperatureC: Float?, val batteryPercent: Int, val memoryBytes: Long)

fun readDevice(context: Context): DeviceState {
    val battery = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
    val plugged = (battery?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0
    val tenths = battery?.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE) ?: Int.MIN_VALUE
    val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
    val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
    val connectivity = context.getSystemService(ConnectivityManager::class.java)
    val caps = connectivity.getNetworkCapabilities(connectivity.activeNetwork)
    val memory = ActivityManager.MemoryInfo().also { context.getSystemService(ActivityManager::class.java).getMemoryInfo(it) }
    return DeviceState(
        charging = plugged,
        wifi = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true,
        temperatureC = if (tenths == Int.MIN_VALUE) null else tenths / 10f,
        batteryPercent = if (level < 0) -1 else level * 100 / scale,
        memoryBytes = memory.totalMem,
    )
}
