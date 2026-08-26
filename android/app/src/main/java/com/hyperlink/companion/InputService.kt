package com.hyperlink.companion

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.util.Log
import android.view.accessibility.AccessibilityEvent

/**
 * Android Accessibility Service for zero-root touch and navigation injection (Phase 3).
 *
 * Dispatches normalized touch taps and drag gestures via [dispatchGesture],
 * and handles system navigation actions (Back, Home, Recents) via [performGlobalAction].
 */
class InputService : AccessibilityService() {

    companion object {
        private const val TAG = "HyperLinkInputService"
        var instance: InputService? = null
            private set
    }

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
        Log.i(TAG, "InputService connected and ready for gesture injection")
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        // Android 10+ background clipboard hook: check clipboard when selection or window state changes
        if (event?.eventType == AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED ||
            event?.eventType == AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED) {
            ClipboardService.instance?.onPrimaryClipChanged()
        }
    }

    override fun onInterrupt() {
        Log.w(TAG, "InputService interrupted")
    }

    override fun onDestroy() {
        super.onDestroy()
        instance = null
        Log.i(TAG, "InputService destroyed")
    }

    /**
     * Injects a tap gesture at normalized coordinates (0..65535).
     */
    fun injectTap(xNorm: Int, yNorm: Int) {
        val dm = resources.displayMetrics
        val screenX = (xNorm.toFloat() / 65535f) * dm.widthPixels
        val screenY = (yNorm.toFloat() / 65535f) * dm.heightPixels

        val path = Path().apply {
            moveTo(screenX, screenY)
        }
        val stroke = GestureDescription.StrokeDescription(path, 0, 50)
        val gesture = GestureDescription.Builder().addStroke(stroke).build()
        dispatchGesture(gesture, null, null)
    }

    /**
     * Injects a continuous stroke / move gesture between two normalized points.
     */
    fun injectSwipe(startXNorm: Int, startYNorm: Int, endXNorm: Int, endYNorm: Int, durationMs: Long = 200) {
        val dm = resources.displayMetrics
        val startX = (startXNorm.toFloat() / 65535f) * dm.widthPixels
        val startY = (startYNorm.toFloat() / 65535f) * dm.heightPixels
        val endX = (endXNorm.toFloat() / 65535f) * dm.widthPixels
        val endY = (endYNorm.toFloat() / 65535f) * dm.heightPixels

        val path = Path().apply {
            moveTo(startX, startY)
            lineTo(endX, endY)
        }
        val stroke = GestureDescription.StrokeDescription(path, 0, durationMs)
        val gesture = GestureDescription.Builder().addStroke(stroke).build()
        dispatchGesture(gesture, null, null)
    }

    /**
     * Injects scroll delta simulated via a short directional swipe gesture.
     */
    fun injectScroll(dy: Int, xNorm: Int, yNorm: Int) {
        val dm = resources.displayMetrics
        val startX = (xNorm.toFloat() / 65535f) * dm.widthPixels
        val startY = (yNorm.toFloat() / 65535f) * dm.heightPixels

        // Scroll down moves finger upwards
        val deltaY = (dy.toFloat() / 120f) * (dm.heightPixels * 0.15f)
        val endY = (startY - deltaY).coerceIn(0f, dm.heightPixels.toFloat())

        val path = Path().apply {
            moveTo(startX, startY)
            lineTo(startX, endY)
        }
        val stroke = GestureDescription.StrokeDescription(path, 0, 100)
        val gesture = GestureDescription.Builder().addStroke(stroke).build()
        dispatchGesture(gesture, null, null)
    }

    /**
     * Performs a global navigation action (Back, Home, Recents, Volume).
     */
    fun injectNavAction(action: Int) {
        val audioManager = getSystemService(android.content.Context.AUDIO_SERVICE) as? android.media.AudioManager
        when (action) {
            1 -> performGlobalAction(GLOBAL_ACTION_BACK)
            2 -> performGlobalAction(GLOBAL_ACTION_HOME)
            3 -> performGlobalAction(GLOBAL_ACTION_RECENTS)
            4 -> audioManager?.adjustStreamVolume(
                android.media.AudioManager.STREAM_MUSIC,
                android.media.AudioManager.ADJUST_RAISE,
                android.media.AudioManager.FLAG_SHOW_UI
            )
            5 -> audioManager?.adjustStreamVolume(
                android.media.AudioManager.STREAM_MUSIC,
                android.media.AudioManager.ADJUST_LOWER,
                android.media.AudioManager.FLAG_SHOW_UI
            )
        }
    }

    /**
     * Injects text/key into the currently focused editable input field via AccessibilityNodeInfo.
     *
     * Note: For arbitrary raw key events outside editable fields, Android requires either
     * USB AOA HID emulation or INJECT_EVENTS permission (per SYSTEM_DESIGN.md).
     */
    fun injectKey(action: Int, keycode: Int, modifiers: Int) {
        if (action != 1) return // Only handle key down

        val root = rootInActiveWindow ?: return
        val focusedNode = root.findFocus(android.view.accessibility.AccessibilityNodeInfo.FOCUS_INPUT) ?: return

        if (!focusedNode.isEditable) return

        val currentText = focusedNode.text?.toString() ?: ""

        // Handle Backspace (XKB: 0xff08, or Android keycode 67)
        if (keycode == 0xff08 || keycode == 67) {
            if (currentText.isNotEmpty()) {
                val newText = currentText.substring(0, currentText.length - 1)
                val bundle = android.os.Bundle().apply {
                    putCharSequence(
                        android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                        newText
                    )
                }
                focusedNode.performAction(
                    android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT,
                    bundle
                )
            }
            return
        }

        // Handle Enter / Return (XKB: 0xff0d, ASCII: 10, or Android keycode 66)
        if (keycode == 0xff0d || keycode == 10 || keycode == 66) {
            val newText = currentText + "\n"
            val bundle = android.os.Bundle().apply {
                putCharSequence(
                    android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                    newText
                )
            }
            focusedNode.performAction(
                android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT,
                bundle
            )
            return
        }

        // Handle printable characters (ASCII range 32..126 or unicode codepoint)
        if (keycode in 32..65535) {
            var char = keycode.toChar()
            val shift = (modifiers and 0x01) != 0
            if (shift && char.isLowerCase()) {
                char = char.uppercaseChar()
            }
            val newText = currentText + char
            val bundle = android.os.Bundle().apply {
                putCharSequence(
                    android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                    newText
                )
            }
            focusedNode.performAction(
                android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT,
                bundle
            )
        }
    }
}
