package com.hyperlink.companion

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.util.Log

class DiscoveryManager(private val context: Context, private val listener: DiscoveryListener) {
    private val TAG = "DiscoveryManager"
    private val nsdManager = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val serviceType = "_hyperlink._udp."
    private var discoveryListener: NsdManager.DiscoveryListener? = null
    private var isScanning = false

    interface DiscoveryListener {
        /** `id` is the mDNS service name (stable per computer); `name` is what to show. */
        fun onHostDiscovered(id: String, name: String, ip: String, port: Int)
        fun onHostLost(id: String)
        fun onDiscoveryStarted()
        fun onDiscoveryStopped()
    }

    /**
     * The computer's own name, as the user set it: the `device_name` TXT
     * attribute, falling back to the service name minus the "-host" suffix the
     * host appends (older hosts advertise without the attribute).
     */
    private fun displayName(info: NsdServiceInfo): String {
        val fromTxt = info.attributes["device_name"]?.let { String(it, Charsets.UTF_8) }?.trim()
        if (!fromTxt.isNullOrEmpty()) return fromTxt
        val service = info.serviceName ?: return "Computer"
        return service.removeSuffix("-host")
    }

    @Synchronized
    fun startDiscovery() {
        if (isScanning) return
        isScanning = true

        discoveryListener = object : NsdManager.DiscoveryListener {
            override fun onStartDiscoveryFailed(serviceType: String?, errorCode: Int) {
                Log.e(TAG, "Discovery start failed: Error code $errorCode")
                stopDiscovery()
            }

            override fun onStopDiscoveryFailed(serviceType: String?, errorCode: Int) {
                Log.e(TAG, "Discovery stop failed: Error code $errorCode")
                nsdManager.stopServiceDiscovery(this)
            }

            override fun onDiscoveryStarted(serviceType: String?) {
                Log.d(TAG, "Discovery started")
                listener.onDiscoveryStarted()
            }

            override fun onDiscoveryStopped(serviceType: String?) {
                Log.d(TAG, "Discovery stopped")
                listener.onDiscoveryStopped()
            }

            override fun onServiceFound(serviceInfo: NsdServiceInfo?) {
                Log.d(TAG, "Service found: $serviceInfo")
                if (serviceInfo == null) return
                if (serviceInfo.serviceType.contains(serviceType)) {
                    // Resolve service details (IP address and port)
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                        nsdManager.registerServiceInfoCallback(serviceInfo, context.mainExecutor, object : NsdManager.ServiceInfoCallback {
                            override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {}
                            override fun onServiceUpdated(resolvedServiceInfo: NsdServiceInfo) {
                                val ip = resolvedServiceInfo.host?.hostAddress ?: return
                                listener.onHostDiscovered(
                                    resolvedServiceInfo.serviceName ?: ip,
                                    displayName(resolvedServiceInfo),
                                    ip,
                                    resolvedServiceInfo.port,
                                )
                            }
                            override fun onServiceLost() {}
                            override fun onServiceInfoCallbackUnregistered() {}
                        })
                    } else {
                        nsdManager.resolveService(serviceInfo, object : NsdManager.ResolveListener {
                            override fun onResolveFailed(serviceInfo: NsdServiceInfo?, errorCode: Int) {
                                Log.e(TAG, "Resolve failed: Error code $errorCode")
                            }

                            override fun onServiceResolved(resolvedServiceInfo: NsdServiceInfo?) {
                                Log.d(TAG, "Service resolved: $resolvedServiceInfo")
                                if (resolvedServiceInfo == null) return
                                val ip = resolvedServiceInfo.host?.hostAddress ?: return
                                listener.onHostDiscovered(
                                    resolvedServiceInfo.serviceName ?: ip,
                                    displayName(resolvedServiceInfo),
                                    ip,
                                    resolvedServiceInfo.port,
                                )
                            }
                        })
                    }
                }
            }

            override fun onServiceLost(serviceInfo: NsdServiceInfo?) {
                Log.d(TAG, "Service lost: $serviceInfo")
                serviceInfo?.serviceName?.let(listener::onHostLost)
            }
        }

        nsdManager.discoverServices(serviceType, NsdManager.PROTOCOL_DNS_SD, discoveryListener)
    }

    @Synchronized
    fun stopDiscovery() {
        if (!isScanning) return
        isScanning = false

        discoveryListener?.let {
            try {
                nsdManager.stopServiceDiscovery(it)
            } catch (e: Exception) {
                Log.e(TAG, "failed to stop service discovery: ${e.message}")
            }
        }
        discoveryListener = null
    }
}
