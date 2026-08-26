package com.hyperlink.companion

import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Binder
import android.os.IBinder
import android.util.Log
import kotlinx.coroutines.*
import org.json.JSONObject
import kotlin.random.Random

/**
 * Background service managing Scoped App-State Handoff on Android (Phase 9).
 *
 * Handles incoming state transfers from Linux desktop (e.g. opening browser tabs,
 * restoring note drafts, continuing media playback) and dispatching outgoing
 * handoffs to the Linux peer.
 */
class HandoffService : Service() {

    companion object {
        private const val TAG = "HandoffService"
        const val TYPE_BROWSER_TAB = 1
        const val TYPE_NOTE_DRAFT = 2
        const val TYPE_MEDIA_PLAYBACK = 3
        const val TYPE_CUSTOM_URI = 4
    }

    private val binder = LocalBinder()
    private val serviceScope = CoroutineScope(Dispatchers.Main + Job())

    inner class LocalBinder : Binder() {
        fun getService(): HandoffService = this@HandoffService
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "HandoffService initialized")
    }

    /**
     * Dispatches an outgoing handoff payload from Android to Linux host.
     */
    fun sendHandoffToHost(
        handoffType: Int,
        appId: String,
        uri: String,
        title: String,
        stateJson: String
    ): Boolean {
        val sessionId = Random.nextLong(1000, 999999)
        Log.i(TAG, "Sending handoff type=$handoffType to host, session=$sessionId, uri=$uri")
        return try {
            QuicClient.sendHandoff(
                sessionId,
                handoffType,
                "phone:AndroidCompanion",
                appId,
                uri,
                title,
                stateJson
            )
        } catch (e: Exception) {
            Log.e(TAG, "Failed to send handoff to host: ${e.message}")
            false
        }
    }

    /**
     * Handles an incoming handoff event received from Linux host.
     */
    fun handleIncomingHandoff(
        context: Context,
        sessionId: Long,
        handoffType: Int,
        appId: String,
        uri: String,
        title: String,
        stateJson: String
    ) {
        Log.i(TAG, "Received incoming handoff type=$handoffType, session=$sessionId, uri=$uri")

        serviceScope.launch {
            var accepted = false
            var statusCode = 0

            try {
                when (handoffType) {
                    TYPE_BROWSER_TAB, TYPE_CUSTOM_URI -> {
                        if (uri.isNotBlank()) {
                            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(uri)).apply {
                                flags = Intent.FLAG_ACTIVITY_NEW_TASK
                            }
                            context.startActivity(intent)
                            accepted = true
                        }
                    }
                    TYPE_NOTE_DRAFT -> {
                        // Open via standard Send/View or note intent
                        val json = JSONObject(stateJson)
                        val textContent = json.optString("content", "")
                        val intent = Intent(Intent.ACTION_SEND).apply {
                            type = "text/plain"
                            putExtra(Intent.EXTRA_SUBJECT, title)
                            putExtra(Intent.EXTRA_TEXT, textContent)
                            flags = Intent.FLAG_ACTIVITY_NEW_TASK
                        }
                        context.startActivity(Intent.createChooser(intent, "Open Note Draft").apply {
                            flags = Intent.FLAG_ACTIVITY_NEW_TASK
                        })
                        accepted = true
                    }
                    TYPE_MEDIA_PLAYBACK -> {
                        if (uri.isNotBlank()) {
                            val intent = Intent(Intent.ACTION_VIEW).apply {
                                setDataAndType(Uri.parse(uri), "video/*")
                                flags = Intent.FLAG_ACTIVITY_NEW_TASK
                            }
                            context.startActivity(intent)
                            accepted = true
                        }
                    }
                    else -> {
                        statusCode = 1 // Unsupported
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to dispatch handoff on device: ${e.message}")
                accepted = false
                statusCode = 2 // Missing handler / failed
            }

            // Acknowledge back to peer
            try {
                QuicClient.acknowledgeHandoff(sessionId, accepted, statusCode)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to send handoff ack: ${e.message}")
            }
        }
    }

    override fun onDestroy() {
        serviceScope.cancel()
        super.onDestroy()
        Log.i(TAG, "HandoffService destroyed")
    }
}
