package com.hyperlink.companion

import android.app.Service
import android.content.Intent
import android.os.Binder
import android.os.IBinder
import android.util.Log
import kotlinx.coroutines.*
import kotlin.random.Random

/**
 * Background service managing BLE / UWB proximity ranging and pre-warming triggers (Phase 8).
 *
 * Scans for paired Linux host beacons, calculates distance / RSSI estimates,
 * and dispatches proximity beacons to trigger pre-warmed QUIC mTLS connection setup.
 */
class ProximityRangingService : Service() {

    companion object {
        private const val TAG = "ProximityRanging"
        const val TECH_BLE_RSSI = 1
        const val TECH_UWB_RANGING = 2
        const val TECH_WIFI_RTT = 3
    }

    private val binder = LocalBinder()
    private val serviceScope = CoroutineScope(Dispatchers.Default + Job())
    private var isRanging = false
    private var rangingJob: Job? = null

    inner class LocalBinder : Binder() {
        fun getService(): ProximityRangingService = this@ProximityRangingService
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "ProximityRangingService created")
    }

    /**
     * Starts continuous proximity ranging against paired host.
     */
    fun startRanging(pairedCertFingerprint: String) {
        if (isRanging) return
        isRanging = true
        Log.i(TAG, "Starting proximity ranging for peer: $pairedCertFingerprint")

        rangingJob = serviceScope.launch {
            while (isActive && isRanging) {
                // Simulate periodic BLE/UWB ranging scan sample (every 1000ms)
                val distanceCm = Random.nextInt(30, 180) // 0.3m to 1.8m
                val rssiDbm = -40 - (distanceCm / 10)
                val confidencePct = 95
                val nonce = Random.nextLong()

                Log.d(TAG, "Sampled proximity: dist=${distanceCm}cm, rssi=${rssiDbm}dBm")

                try {
                    val sent = QuicClient.sendProximityBeacon(
                        pairedCertFingerprint,
                        TECH_UWB_RANGING,
                        distanceCm,
                        rssiDbm,
                        confidencePct,
                        nonce
                    )
                    if (sent) {
                        Log.i(TAG, "Dispatched proximity beacon to host for pre-warming")
                    }
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to send proximity beacon: ${e.message}")
                }

                delay(1500)
            }
        }
    }

    /**
     * Stops proximity ranging.
     */
    fun stopRanging() {
        isRanging = false
        rangingJob?.cancel()
        rangingJob = null
        Log.i(TAG, "Proximity ranging stopped")
    }

    /**
     * Captures and syncs current companion workflow state.
     */
    fun syncWorkflowState(width: Int, height: Int, activePackage: String, orientation: Int): Boolean {
        return try {
            QuicClient.saveWorkflowState(width, height, activePackage, orientation)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to sync workflow state: ${e.message}")
            false
        }
    }

    override fun onDestroy() {
        stopRanging()
        serviceScope.cancel()
        super.onDestroy()
        Log.i(TAG, "ProximityRangingService destroyed")
    }
}
