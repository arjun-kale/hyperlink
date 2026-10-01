package com.hyperlink.companion

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File

object QuicClient {
    private const val TAG = "QuicClient"

    init {
        try {
            System.loadLibrary("hyperlink_bridge")
            Log.i(TAG, "Native library loaded successfully")
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "Failed to load native library: ${e.message}")
        }
    }

    // --- Native JNI Interface declarations ---
    private external fun initialize(storagePath: String)
    private external fun connectHost(hostIp: String, port: Int, isPairing: Boolean)
    private external fun confirmPairing(hostName: String): Boolean
    private external fun sendMessage(payload: ByteArray): Boolean
    private external fun pollEvent(): String?
    private external fun sendVideoFrame(frameData: ByteArray, frameId: Int, timestampUs: Long, isKeyframe: Boolean, width: Int, height: Int): Boolean
    private external fun sendVideoConfig(sps: ByteArray, pps: ByteArray, bitrate: Int, fps: Int): Boolean
    private external fun sendNotificationPost(id: String, packageName: String, appName: String, title: String, body: String, timestampMs: Long, iconBytes: ByteArray?): Boolean
    private external fun sendNotificationDismiss(id: String): Boolean
    private external fun sendDndSync(enabled: Boolean): Boolean
    private external fun sendClipboardText(originId: String, text: String): Boolean
    private external fun sendClipboardImage(originId: String, mimeType: String, imageBytes: ByteArray): Boolean
    external fun getOwnFingerprint(): String?
    external fun sendProximityBeacon(certFp: String, technology: Int, distanceCm: Int, rssiDbm: Int, confidencePct: Int, nonce: Long): Boolean
    external fun saveWorkflowState(width: Int, height: Int, packageName: String, orientation: Int): Boolean
    external fun restoreWorkflowState(): Boolean
    external fun sendHandoff(sessionId: Long, handoffType: Int, sourceOrigin: String, appId: String, uri: String, title: String, stateJson: String): Boolean
    external fun acknowledgeHandoff(sessionId: Long, accepted: Boolean, statusCode: Int): Boolean
    external fun publishAmbientEvent(eventId: Long, category: Int, source: String, summary: String, metadataJson: String): Boolean
    external fun updateAgentConsent(allowNotifications: Boolean, allowScreenState: Boolean, allowForegroundApp: Boolean, allowDeviceStatus: Boolean, allowClipboard: Boolean, allowMediaState: Boolean, allowRawVideo: Boolean): Boolean

    // --- Kotlin Wrapper Logic ---
    interface EventListener {
        fun onPairingPin(pin: Int)
        fun onConnected()
        fun onDisconnected(reason: String)
        fun onMessage(streamType: Byte, payload: ByteArray)
        fun onVideoStreamReady()
        fun onPointerEvent(action: Int, button: Int, xNorm: Int, yNorm: Int, pressure: Int) {}
        fun onKeyEvent(action: Int, keycode: Int, modifiers: Int) {}
        fun onScrollEvent(dx: Int, dy: Int, xNorm: Int, yNorm: Int) {}
        fun onNavEvent(action: Int) {}
        fun onNotificationAction(key: String, actionId: Int) {}
        fun onNotificationDismiss(key: String) {}
        fun onDndSync(enabled: Boolean) {}
        fun onClipboardReceived(originId: String, contentType: Int, mimeType: String, payload: ByteArray) {}
        fun onHandoffReceived(sessionId: Long, handoffType: Int, appId: String, uri: String, title: String, stateJson: String) {}
        fun onAgentConsentUpdated(allowNotifications: Boolean, allowClipboard: Boolean, allowRawVideo: Boolean) {}
    }

    private var listener: EventListener? = null
    private val scope = CoroutineScope(Dispatchers.Default)
    private var pollJob: Job? = null

    /**
     * Initializes the client configuration using the app private storage path.
     */
    fun init(context: Context, eventListener: EventListener) {
        this.listener = eventListener
        val storageDir = context.filesDir
        Log.d(TAG, "initializing client with storage dir: ${storageDir.absolutePath}")
        initialize(storageDir.absolutePath)
        
        // Start background events polling loop.
        startPollingLoop()
    }

    /**
     * Attempts to connect to the given host IP and port.
     */
    fun connect(hostIp: String, port: Int, isPairing: Boolean) {
        Log.i(TAG, "connecting to $hostIp:$port (pairing=$isPairing)")
        connectHost(hostIp, port, isPairing)
    }

    /**
     * Confirms a pending pairing request, trusting the host under its mDNS name.
     */
    fun confirm(hostName: String): Boolean {
        Log.i(TAG, "confirming pairing with $hostName")
        return confirmPairing(hostName)
    }

    /**
     * Sends a message payload over the QUIC control plane stream.
     */
    fun send(payload: ByteArray): Boolean {
        return sendMessage(payload)
    }

    fun sendFrame(frameData: ByteArray, frameId: Int, timestampUs: Long, isKeyframe: Boolean, width: Int, height: Int): Boolean {
        return sendVideoFrame(frameData, frameId, timestampUs, isKeyframe, width, height)
    }

    fun sendConfig(sps: ByteArray, pps: ByteArray, bitrate: Int, fps: Int): Boolean {
        return sendVideoConfig(sps, pps, bitrate, fps)
    }

    fun postNotification(id: String, packageName: String, appName: String, title: String, body: String, timestampMs: Long, iconBytes: ByteArray?): Boolean {
        return sendNotificationPost(id, packageName, appName, title, body, timestampMs, iconBytes)
    }

    fun dismissNotification(id: String): Boolean {
        return sendNotificationDismiss(id)
    }

    fun syncDnd(enabled: Boolean): Boolean {
        return sendDndSync(enabled)
    }

    fun postClipboardText(originId: String, text: String): Boolean {
        return sendClipboardText(originId, text)
    }

    fun postClipboardImage(originId: String, mimeType: String, imageBytes: ByteArray): Boolean {
        return sendClipboardImage(originId, mimeType, imageBytes)
    }

    /** This device's own certificate fingerprint, for [ProximityRangingService] to broadcast. */
    fun ownFingerprint(): String? = getOwnFingerprint()

    private fun startPollingLoop() {
        pollJob?.cancel()
        pollJob = scope.launch {
            Log.d(TAG, "starting event polling loop")
            while (isActive) {
                val jsonStr = pollEvent()
                if (jsonStr != null) {
                    Log.d(TAG, "received native event JSON: $jsonStr")
                    try {
                        val json = JSONObject(jsonStr)
                        val type = json.optString("type")
                        when (type) {
                            "pairing_pin" -> {
                                val pin = json.optInt("pin")
                                listener?.onPairingPin(pin)
                            }
                            "connected" -> {
                                listener?.onConnected()
                            }
                            "disconnected" -> {
                                val reason = json.optString("reason", "Unknown reason")
                                listener?.onDisconnected(reason)
                            }
                            "message" -> {
                                val streamType = json.optInt("stream_type").toByte()
                                val hexStr = json.optString("payload")
                                val payload = hexStringToByteArray(hexStr)
                                listener?.onMessage(streamType, payload)
                            }
                            "video_ready" -> {
                                listener?.onVideoStreamReady()
                            }
                            "pointer" -> {
                                val action = json.optInt("action")
                                val button = json.optInt("button")
                                val xNorm = json.optInt("x_norm")
                                val yNorm = json.optInt("y_norm")
                                val pressure = json.optInt("pressure")
                                listener?.onPointerEvent(action, button, xNorm, yNorm, pressure)
                            }
                            "key" -> {
                                val action = json.optInt("action")
                                val keycode = json.optInt("keycode")
                                val modifiers = json.optInt("modifiers")
                                listener?.onKeyEvent(action, keycode, modifiers)
                            }
                            "scroll" -> {
                                val dx = json.optInt("dx")
                                val dy = json.optInt("dy")
                                val xNorm = json.optInt("x_norm")
                                val yNorm = json.optInt("y_norm")
                                listener?.onScrollEvent(dx, dy, xNorm, yNorm)
                            }
                            "nav" -> {
                                val action = json.optInt("action")
                                listener?.onNavEvent(action)
                            }
                            "notif_action" -> {
                                val key = json.optString("key")
                                val actionId = json.optInt("action_id")
                                listener?.onNotificationAction(key, actionId)
                            }
                            "notif_dismiss" -> {
                                val key = json.optString("key")
                                listener?.onNotificationDismiss(key)
                            }
                            "dnd_sync" -> {
                                val enabled = json.optBoolean("enabled")
                                listener?.onDndSync(enabled)
                            }
                            "clipboard" -> {
                                val originId = json.optString("origin_id")
                                val contentType = json.optInt("content_type")
                                val mimeType = json.optString("mime_type")
                                val dataB64 = json.optString("data_b64")
                                val payload = if (dataB64.isNotEmpty()) {
                                    android.util.Base64.decode(dataB64, android.util.Base64.DEFAULT)
                                } else {
                                    ByteArray(0)
                                }
                                listener?.onClipboardReceived(originId, contentType, mimeType, payload)
                            }
                            "handoff" -> {
                                val sessionId = json.optLong("session_id")
                                val handoffType = json.optInt("handoff_type")
                                val appId = json.optString("app_id")
                                val uri = json.optString("uri")
                                val title = json.optString("title")
                                val stateJson = json.optString("state_json")
                                listener?.onHandoffReceived(sessionId, handoffType, appId, uri, title, stateJson)
                            }
                            "agent_consent" -> {
                                val allowNotifications = json.optBoolean("allow_notifications")
                                val allowClipboard = json.optBoolean("allow_clipboard")
                                val allowRawVideo = json.optBoolean("allow_raw_video")
                                listener?.onAgentConsentUpdated(allowNotifications, allowClipboard, allowRawVideo)
                            }
                        }
                    } catch (e: Exception) {
                        Log.e(TAG, "error parsing event JSON: ${e.message}", e)
                    }
                }
                // Yield to prevent CPU thrashing (using a micro-sleep).
                delay(20)
            }
        }
    }

    private fun hexStringToByteArray(hex: String): ByteArray {
        val len = hex.length
        val data = ByteArray(len / 2)
        var i = 0
        while (i < len) {
            data[i / 2] = ((Character.digit(hex[i], 16) shl 4) + Character.digit(hex[i + 1], 16)).toByte()
            i += 2
        }
        return data
    }
}
