package com.hyperlink.companion

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import androidx.core.content.FileProvider
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest

/**
 * Manages bidirectional clipboard synchronization between Android and Linux host.
 *
 * Implements two-tier loop prevention:
 * 1. Origin ID tracking (`phone:<android_id>` vs `host:<hostname>`).
 * 2. SHA-256 content hash caching (`lastSyncedHash`) to ignore local echoes
 *    triggered when remote clipboard items are written locally.
 */
class ClipboardService(private val context: Context) {
    companion object {
        private const val TAG = "ClipboardService"
        var instance: ClipboardService? = null
            private set

        fun init(context: Context): ClipboardService {
            val service = ClipboardService(context)
            instance = service
            return service
        }
    }

    private val clipboardManager =
        context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    private val mainHandler = Handler(Looper.getMainLooper())

    val originId: String by lazy {
        val androidId = try {
            Settings.Secure.getString(context.contentResolver, Settings.Secure.ANDROID_ID) ?: "device"
        } catch (e: Exception) {
            "device"
        }
        "phone:$androidId"
    }

    @Volatile
    private var lastSyncedHash: ByteArray? = null

    private val clipListener = ClipboardManager.OnPrimaryClipChangedListener {
        onPrimaryClipChanged()
    }

    fun start() {
        clipboardManager.addPrimaryClipChangedListener(clipListener)
        Log.i(TAG, "ClipboardService started for origin: $originId")
    }

    fun stop() {
        clipboardManager.removePrimaryClipChangedListener(clipListener)
        Log.i(TAG, "ClipboardService stopped")
    }

    /**
     * Called when local clipboard changes (or triggered via InputService accessibility hook).
     */
    fun onPrimaryClipChanged() {
        try {
            val clip = clipboardManager.primaryClip ?: return
            if (clip.itemCount == 0) return

            val item = clip.getItemAt(0) ?: return

            // 1. Check for text content
            val text = item.text?.toString()
                ?: item.coerceToText(context)?.toString()

            if (!text.isNullOrEmpty()) {
                val bytes = text.toByteArray(Charsets.UTF_8)
                val hash = computeSha256(bytes)
                if (lastSyncedHash != null && hash.contentEquals(lastSyncedHash)) {
                    // Echo loop suppression: clip was written from remote sync
                    return
                }

                lastSyncedHash = hash
                Log.d(TAG, "Syncing local text clip to host (len=${text.length})")
                QuicClient.postClipboardText(originId, text)
                return
            }

            // 2. Check for image content via URI
            val uri = item.uri
            if (uri != null) {
                val mimeType = context.contentResolver.getType(uri) ?: "image/png"
                if (mimeType.startsWith("image/")) {
                    val bytes = readBytesFromUri(uri)
                    if (bytes != null && bytes.isNotEmpty()) {
                        val hash = computeSha256(bytes)
                        if (lastSyncedHash != null && hash.contentEquals(lastSyncedHash)) {
                            return
                        }

                        lastSyncedHash = hash
                        Log.d(TAG, "Syncing local image clip to host (mime=$mimeType, size=${bytes.size})")
                        QuicClient.postClipboardImage(originId, mimeType, bytes)
                    }
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error reading local clipboard: ${e.message}", e)
        }
    }

    /**
     * Writes remote clipboard item received from host onto local Android clipboard.
     */
    fun writeRemoteClip(
        remoteOriginId: String,
        contentType: Int,
        mimeType: String,
        payload: ByteArray
    ) {
        if (remoteOriginId == originId) {
            Log.d(TAG, "Ignoring self echo with origin $remoteOriginId")
            return
        }

        val hash = computeSha256(payload)
        if (lastSyncedHash != null && hash.contentEquals(lastSyncedHash)) {
            Log.d(TAG, "Ignoring already synced clipboard payload")
            return
        }

        lastSyncedHash = hash

        mainHandler.post {
            try {
                if (contentType == 1) {
                    // Text
                    val text = String(payload, Charsets.UTF_8)
                    val clip = ClipData.newPlainText("HyperLink", text)
                    clipboardManager.setPrimaryClip(clip)
                    Log.i(TAG, "Applied remote text clip to device clipboard (len=${text.length})")
                } else if (contentType == 2) {
                    // Image
                    val imageFile = File(context.cacheDir, "clipboard_received.png")
                    FileOutputStream(imageFile).use { out ->
                        out.write(payload)
                    }

                    val authority = "${context.packageName}.fileprovider"
                    val contentUri: Uri = try {
                        FileProvider.getUriForFile(context, authority, imageFile)
                    } catch (e: Exception) {
                        Uri.fromFile(imageFile)
                    }

                    val clip = ClipData.newUri(context.contentResolver, "HyperLink Image", contentUri)
                    clipboardManager.setPrimaryClip(clip)
                    Log.i(TAG, "Applied remote image clip to device clipboard (${payload.size} bytes)")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to write remote clip to Android clipboard: ${e.message}", e)
            }
        }
    }

    private fun readBytesFromUri(uri: Uri): ByteArray? {
        return try {
            context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to read bytes from URI: $uri", e)
            null
        }
    }

    private fun computeSha256(data: ByteArray): ByteArray {
        val digest = MessageDigest.getInstance("SHA-256")
        return digest.digest(data)
    }
}
