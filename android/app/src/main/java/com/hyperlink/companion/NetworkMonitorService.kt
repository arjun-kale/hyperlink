package com.hyperlink.companion

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log

/**
 * Android Network Monitor Service for Phase 7 (Network Resilience & Multipath).
 *
 * Listens for network interface transitions (Wi-Fi, Cellular Tether, Ethernet)
 * via ConnectivityManager.NetworkCallback and notifies the Rust JNI bridge.
 */
class NetworkMonitorService(private val context: Context) {

    private val connectivityManager =
        context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

    private var networkCallback: ConnectivityManager.NetworkCallback? = null

    companion object {
        private const val TAG = "NetworkMonitorService"

        const val PATH_ID_PRIMARY_WIFI = 0
        const val PATH_ID_CELLULAR_TETHER = 1
        const val PATH_ID_ETHERNET = 2
        const val PATH_ID_WIFI_MLO_SECONDARY = 3

        @Volatile
        var instance: NetworkMonitorService? = null

        fun init(context: Context): NetworkMonitorService {
            val s = NetworkMonitorService(context.applicationContext)
            instance = s
            s.startMonitoring()
            return s
        }

        // --- JNI External Declarations ---
        @JvmStatic
        external fun onNetworkAvailable(pathId: Int, ip: String, port: Int)

        @JvmStatic
        external fun onNetworkLost(pathId: Int)

        @JvmStatic
        external fun triggerFailover(targetPathId: Int, reasonCode: Int): Boolean

        @JvmStatic
        external fun getActivePath(): Int
    }

    fun startMonitoring() {
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()

        networkCallback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                val caps = connectivityManager.getNetworkCapabilities(network) ?: return
                val pathId = determinePathId(caps)
                val linkProperties = connectivityManager.getLinkProperties(network)
                val ip = linkProperties?.linkAddresses?.firstOrNull()?.address?.hostAddress ?: "0.0.0.0"

                Log.i(TAG, "Network available: pathId=$pathId, ip=$ip")
                try {
                    onNetworkAvailable(pathId, ip, 0)
                } catch (e: UnsatisfiedLinkError) {
                    Log.w(TAG, "JNI onNetworkAvailable not linked yet: ${e.message}")
                }
            }

            override fun onLost(network: Network) {
                val caps = connectivityManager.getNetworkCapabilities(network)
                val pathId = if (caps != null) determinePathId(caps) else PATH_ID_PRIMARY_WIFI
                Log.w(TAG, "Network lost: pathId=$pathId")
                try {
                    onNetworkLost(pathId)
                    val active = getActivePath()
                    if (pathId == active) {
                        // Active path was dropped! Initiate auto-failover to secondary path
                        val target = if (pathId == PATH_ID_PRIMARY_WIFI) PATH_ID_CELLULAR_TETHER else PATH_ID_PRIMARY_WIFI
                        Log.i(TAG, "Active path $pathId dropped! Triggering auto-failover to target $target")
                        val switched = triggerFailover(target, 0) // reason 0 = LinkLoss
                        Log.i(TAG, "Auto-failover trigger returned: $switched")
                    }
                } catch (e: UnsatisfiedLinkError) {
                    Log.w(TAG, "JNI onNetworkLost/triggerFailover not linked yet: ${e.message}")
                }
            }

            override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) {
                val pathId = determinePathId(networkCapabilities)
                Log.d(TAG, "Network capabilities changed: pathId=$pathId")
            }
        }

        try {
            connectivityManager.registerNetworkCallback(request, networkCallback!!)
            Log.i(TAG, "Network callback registered for multipath monitoring")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to register network callback: ${e.message}")
        }
    }

    fun stopMonitoring() {
        networkCallback?.let {
            try {
                connectivityManager.unregisterNetworkCallback(it)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to unregister network callback: ${e.message}")
            }
            networkCallback = null
        }
    }

    private fun determinePathId(caps: NetworkCapabilities): Int {
        return when {
            caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> PATH_ID_PRIMARY_WIFI
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> PATH_ID_CELLULAR_TETHER
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> PATH_ID_ETHERNET
            else -> PATH_ID_PRIMARY_WIFI
        }
    }
}
