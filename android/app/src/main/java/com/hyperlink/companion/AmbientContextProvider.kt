package com.hyperlink.companion

import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import android.os.Binder
import android.os.IBinder
import android.util.Log
import kotlinx.coroutines.*
import org.json.JSONObject
import kotlin.random.Random

/**
 * Background telemetry and ambient context provider for Android Companion (Phase 10).
 *
 * Monitors screen power state, battery charging telemetry, and device status,
 * publishing structured ambient events across the QUIC control stream to the Linux host.
 */
class AmbientContextProvider : Service() {

    companion object {
        private const val TAG = "AmbientContextProvider"
        const val CATEGORY_NOTIFICATIONS = 1
        const val CATEGORY_SCREEN_STATE = 2
        const val CATEGORY_FOREGROUND_APP = 3
        const val CATEGORY_DEVICE_STATUS = 4
        const val CATEGORY_CLIPBOARD = 5
        const val CATEGORY_MEDIA_STATE = 6
    }

    private val binder = LocalBinder()
    private val serviceScope = CoroutineScope(Dispatchers.Default + Job())
    private var telemetryReceiver: BroadcastReceiver? = null

    inner class LocalBinder : Binder() {
        fun getService(): AmbientContextProvider = this@AmbientContextProvider
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "AmbientContextProvider initialized")
        registerTelemetryReceiver()
    }

    private fun registerTelemetryReceiver() {
        telemetryReceiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) {
                intent?.action?.let { action ->
                    when (action) {
                        Intent.ACTION_SCREEN_ON -> {
                            publishTelemetry(
                                CATEGORY_SCREEN_STATE,
                                "Screen turned ON (Device Active)",
                                JSONObject().put("screen_on", true).toString()
                            )
                        }
                        Intent.ACTION_SCREEN_OFF -> {
                            publishTelemetry(
                                CATEGORY_SCREEN_STATE,
                                "Screen turned OFF (Device Idle / Locked)",
                                JSONObject().put("screen_on", false).toString()
                            )
                        }
                        Intent.ACTION_POWER_CONNECTED -> {
                            publishTelemetry(
                                CATEGORY_DEVICE_STATUS,
                                "Power Connected (Charging)",
                                JSONObject().put("charging", true).toString()
                            )
                        }
                        Intent.ACTION_POWER_DISCONNECTED -> {
                            publishTelemetry(
                                CATEGORY_DEVICE_STATUS,
                                "Power Disconnected (Discharging)",
                                JSONObject().put("charging", false).toString()
                            )
                        }
                        Intent.ACTION_BATTERY_CHANGED -> {
                            val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
                            val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
                            val pct = if (level >= 0 && scale > 0) (level * 100) / scale else -1
                            if (pct >= 0 && pct % 10 == 0) { // Throttle battery reports to every 10%
                                publishTelemetry(
                                    CATEGORY_DEVICE_STATUS,
                                    "Battery level: $pct%",
                                    JSONObject().put("battery_pct", pct).toString()
                                )
                            }
                        }
                    }
                }
            }
        }

        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_POWER_CONNECTED)
            addAction(Intent.ACTION_POWER_DISCONNECTED)
            addAction(Intent.ACTION_BATTERY_CHANGED)
        }

        registerReceiver(telemetryReceiver, filter)
    }

    /**
     * Publishes a telemetry event to the Linux host.
     */
    fun publishTelemetry(category: Int, summary: String, metadataJson: String) {
        val eventId = Random.nextLong(100000, 9999999)
        serviceScope.launch {
            try {
                val ok = QuicClient.publishAmbientEvent(
                    eventId,
                    category,
                    "phone:AndroidCompanion",
                    summary,
                    metadataJson
                )
                if (ok) {
                    Log.d(TAG, "Published ambient event #$eventId: $summary")
                }
            } catch (e: Exception) {
                Log.w(TAG, "Failed to publish ambient event: ${e.message}")
            }
        }
    }

    override fun onDestroy() {
        telemetryReceiver?.let {
            try {
                unregisterReceiver(it)
            } catch (e: Exception) {
                Log.w(TAG, "Error unregistering receiver: ${e.message}")
            }
        }
        serviceScope.cancel()
        super.onDestroy()
        Log.i(TAG, "AmbientContextProvider destroyed")
    }
}
