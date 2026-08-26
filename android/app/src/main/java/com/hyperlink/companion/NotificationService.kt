package com.hyperlink.companion

import android.app.Notification
import android.app.NotificationManager
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import android.graphics.drawable.Icon
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import java.io.ByteArrayOutputStream

/**
 * Android Notification Listener Service for Phase 4 Notification Sync.
 *
 * Captures posted notifications, extracts app metadata, titles, body text,
 * action intents, and compressed icons, and streams them over the QUIC tunnel.
 */
class NotificationService : NotificationListenerService() {

    companion object {
        private const val TAG = "HyperLinkNotifService"
        var instance: NotificationService? = null
            private set
    }

    override fun onListenerConnected() {
        super.onListenerConnected()
        instance = this
        Log.i(TAG, "NotificationService connected and listening for notifications")
    }

    override fun onListenerDisconnected() {
        super.onListenerDisconnected()
        instance = null
        Log.i(TAG, "NotificationService disconnected")
    }

    override fun onDestroy() {
        super.onDestroy()
        instance = null
        Log.i(TAG, "NotificationService destroyed")
    }

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        if (sbn == null) return

        val pkg = sbn.packageName
        // Do not mirror notifications from our own app
        if (pkg == packageName) return

        val notification = sbn.notification ?: return
        val extras = notification.extras ?: return

        // Skip ongoing / foreground service notifications (e.g. media players, download progress, ongoing status)
        val isOngoing = (notification.flags and (Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE)) != 0
        if (isOngoing) {
            Log.d(TAG, "Skipping ongoing/foreground notification from $pkg")
            return
        }

        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()
            ?: extras.getCharSequence(Notification.EXTRA_TITLE_BIG)?.toString()
            ?: ""

        val body = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString()
            ?: extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString()
            ?: ""

        // Skip empty notifications
        if (title.isEmpty() && body.isEmpty()) return

        val pm = packageManager
        val appName = try {
            val appInfo = pm.getApplicationInfo(pkg, 0)
            pm.getApplicationLabel(appInfo).toString()
        } catch (e: PackageManager.NameNotFoundException) {
            pkg
        }

        val iconBytes = extractIconBytes(notification, pm, pkg)
        val timestamp = sbn.postTime

        Log.d(TAG, "Notification posted: [$appName] $title (id=${sbn.key}, ongoing=$isOngoing)")

        QuicClient.postNotification(
            id = sbn.key,
            packageName = pkg,
            appName = appName,
            title = title,
            body = body,
            timestampMs = timestamp,
            iconBytes = iconBytes
        )
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        if (sbn == null) return
        val key = sbn.key
        Log.d(TAG, "Notification removed: $key")
        QuicClient.dismissNotification(key)
    }

    override fun onInterruptionFilterChanged(interruptionFilter: Int) {
        super.onInterruptionFilterChanged(interruptionFilter)
        val dndEnabled = interruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL
        Log.i(TAG, "Interruption filter changed: dndEnabled=$dndEnabled")
        QuicClient.syncDnd(dndEnabled)
    }

    /**
     * Invokes an action on an active notification.
     */
    fun invokeAction(key: String, actionId: Int) {
        val active = activeNotifications ?: return
        val sbn = active.find { it.key == key } ?: return
        val actions = sbn.notification.actions ?: return
        if (actionId < actions.size) {
            try {
                actions[actionId].actionIntent.send()
                Log.i(TAG, "Successfully invoked notification action $actionId on $key")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to invoke action $actionId on $key: ${e.message}", e)
            }
        }
    }

    /**
     * Dismisses an active notification on the phone.
     */
    fun dismiss(key: String) {
        try {
            cancelNotification(key)
            Log.i(TAG, "Cancelled notification $key")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to cancel notification $key: ${e.message}", e)
        }
    }

    private fun extractIconBytes(notification: Notification, pm: PackageManager, pkg: String): ByteArray? {
        try {
            val drawable: Drawable? = notification.getLargeIcon()?.loadDrawable(this)
                ?: notification.smallIcon?.loadDrawable(this)
                ?: pm.getApplicationIcon(pkg)

            if (drawable == null) return null

            val bitmap = drawableToBitmap(drawable)
            // Scale to max 48x48 to stay within 32 KB limit
            val scaled = if (bitmap.width > 48 || bitmap.height > 48) {
                Bitmap.createScaledBitmap(bitmap, 48, 48, true)
            } else {
                bitmap
            }

            val stream = ByteArrayOutputStream()
            scaled.compress(Bitmap.CompressFormat.PNG, 85, stream)
            val bytes = stream.toByteArray()
            if (bytes.size <= 32 * 1024) {
                return bytes
            }
        } catch (e: Exception) {
            Log.w(TAG, "Error extracting icon: ${e.message}")
        }
        return null
    }

    private fun drawableToBitmap(drawable: Drawable): Bitmap {
        if (drawable is BitmapDrawable && drawable.bitmap != null) {
            return drawable.bitmap
        }
        val width = if (drawable.intrinsicWidth > 0) drawable.intrinsicWidth else 48
        val height = if (drawable.intrinsicHeight > 0) drawable.intrinsicHeight else 48
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        drawable.setBounds(0, 0, canvas.width, canvas.height)
        drawable.draw(canvas)
        return bitmap
    }
}
