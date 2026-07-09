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
    private external fun confirmPairing(): Boolean
    private external fun sendMessage(payload: ByteArray): Boolean
    private external fun pollEvent(): String?
    private external fun sendVideoFrame(frameData: ByteArray, frameId: Int, timestampUs: Long, isKeyframe: Boolean, width: Int, height: Int): Boolean
    private external fun sendVideoConfig(sps: ByteArray, pps: ByteArray, bitrate: Int, fps: Int): Boolean

    // --- Kotlin Wrapper Logic ---
    interface EventListener {
        fun onPairingPin(pin: Int)
        fun onConnected()
        fun onDisconnected(reason: String)
        fun onMessage(streamType: Byte, payload: ByteArray)
        fun onVideoStreamReady()
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
     * Confirms a pending pairing request.
     */
    fun confirm(): Boolean {
        Log.i(TAG, "confirming pairing")
        return confirmPairing()
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
