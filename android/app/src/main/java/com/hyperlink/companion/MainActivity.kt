package com.hyperlink.companion

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.hyperlink.companion.ui.Computer
import com.hyperlink.companion.ui.HyperLinkApp
import com.hyperlink.companion.ui.HyperLinkTheme
import com.hyperlink.companion.ui.LinkPhase
import com.hyperlink.companion.ui.LinkStore
import com.hyperlink.companion.ui.PermissionKind
import com.hyperlink.companion.ui.UiActions
import com.hyperlink.companion.ui.permissionIntent
import com.hyperlink.companion.ui.readPermissions

class MainActivity : ComponentActivity(), DiscoveryManager.DiscoveryListener, QuicClient.EventListener, UiActions {
    companion object {
        private const val TAG = "MainActivity"
        private const val REQUEST_POST_NOTIFICATIONS = 1002
        /** Give up on a connection attempt that hasn't resolved by then. */
        private const val CONNECT_TIMEOUT_MS = 15_000L
        /** After losing a connection, look again for the computer after this. */
        private const val RECONNECT_DELAY_MS = 2_500L
    }

    private lateinit var store: LinkStore
    private lateinit var discoveryManager: DiscoveryManager
    private val main = Handler(Looper.getMainLooper())

    /** The computer the current (or last) connection attempt is for. */
    private var target: Computer? = null
    /** Set when the user disconnects, so we don't immediately reconnect. */
    private var userDisconnected = false
    private var attemptId = 0

    // Phase 8/9/10 background services — real `Service()` subclasses (unlike
    // ClipboardService/FileAccessService/NetworkMonitorService, which are
    // plain classes instantiated directly), so they're bound rather than `init()`'d.
    private var proximityService: ProximityRangingService? = null
    private var handoffService: HandoffService? = null
    private var ambientService: AmbientContextProvider? = null

    private val proximityConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            proximityService = (binder as? ProximityRangingService.LocalBinder)?.getService()
            log("ProximityRangingService bound")
        }
        override fun onServiceDisconnected(name: ComponentName?) {
            proximityService = null
        }
    }

    private val handoffConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            handoffService = (binder as? HandoffService.LocalBinder)?.getService()
            log("HandoffService bound")
        }
        override fun onServiceDisconnected(name: ComponentName?) {
            handoffService = null
        }
    }

    private val ambientConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            ambientService = (binder as? AmbientContextProvider.LocalBinder)?.getService()
            log("AmbientContextProvider bound")
        }
        override fun onServiceDisconnected(name: ComponentName?) {
            ambientService = null
        }
    }

    private val screenCapture = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val data = result.data
        if (result.resultCode == RESULT_OK && data != null) {
            log("Screen capture allowed; starting")
            ScreenCaptureService.start(this, result.resultCode, data)
            store.update { it.copy(mirroring = true) }
        } else {
            log("Screen capture not allowed")
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        store = LinkStore(this)

        // Initialize Core Clients
        discoveryManager = DiscoveryManager(this, this)
        QuicClient.init(this, this)
        ClipboardService.init(this).start()
        FileAccessService.init(this)
        NetworkMonitorService.init(this)

        // Bind Phase 8/9/10 services. BIND_AUTO_CREATE starts them if not already
        // running, so AmbientContextProvider's telemetry receiver (registered in its
        // own onCreate) is live for the whole activity lifetime — publish calls are
        // harmless no-ops on the Rust side until a connection actually exists.
        bindService(Intent(this, ProximityRangingService::class.java), proximityConnection, Context.BIND_AUTO_CREATE)
        bindService(Intent(this, HandoffService::class.java), handoffConnection, Context.BIND_AUTO_CREATE)
        bindService(Intent(this, AmbientContextProvider::class.java), ambientConnection, Context.BIND_AUTO_CREATE)

        setContent {
            val state by store.state.collectAsStateWithLifecycle()
            HyperLinkTheme {
                HyperLinkApp(state, this)
            }
        }
    }

    override fun onStart() {
        super.onStart()
        if (store.state.value.phase !is LinkPhase.Connected) {
            discoveryManager.startDiscovery()
        }
    }

    override fun onResume() {
        super.onResume()
        // Permissions are granted in system Settings; re-read them on every return.
        store.update { it.copy(permissions = readPermissions(this)) }
    }

    override fun onStop() {
        super.onStop()
        discoveryManager.stopDiscovery()
    }

    override fun onDestroy() {
        super.onDestroy()
        main.removeCallbacksAndMessages(null)
        ClipboardService.instance?.stop()
        discoveryManager.stopDiscovery()
        proximityService?.stopRanging()
        unbindService(proximityConnection)
        unbindService(handoffConnection)
        unbindService(ambientConnection)
        ConnectionKeepAliveService.stop(this)
    }

    private fun log(message: String) {
        Log.i(TAG, message)
        if (::store.isInitialized) store.log(message)
    }

    // ── UiActions ──────────────────────────────────────────────────────────

    override fun finishOnboarding() {
        store.setOnboarded()
        // Needed for the "connected" notification that keeps the link alive.
        if (Build.VERSION.SDK_INT >= 33) {
            requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), REQUEST_POST_NOTIFICATIONS)
        }
    }

    override fun connect(computer: Computer) {
        userDisconnected = false
        target = computer
        val attempt = ++attemptId
        store.update { it.copy(phase = LinkPhase.Connecting(computer.name)) }
        log("Connecting to ${computer.name} (${computer.ip}:${computer.port})")
        // Always allow pairing: the bridge only shows a code if this computer's
        // certificate isn't already trusted, otherwise it connects normally.
        QuicClient.connect(computer.ip, computer.port, isPairing = true)

        main.postDelayed({
            if (attempt == attemptId && store.state.value.phase is LinkPhase.Connecting) {
                showProblem(
                    "Couldn't reach ${computer.name}. Check that it's on and on the same Wi-Fi as this phone.",
                    canRetry = true,
                )
                QuicClient.disconnect()
            }
        }, CONNECT_TIMEOUT_MS)
    }

    override fun confirmPairing() {
        val phase = store.state.value.phase as? LinkPhase.Pairing ?: return
        if (QuicClient.confirm(phase.computer)) {
            store.update { it.copy(phase = phase.copy(confirmed = true)) }
            log("Pairing code confirmed on phone; waiting for the computer")
        } else {
            showProblem("Pairing ended before it finished. Try again.", canRetry = true)
        }
    }

    override fun cancelPairing() {
        userDisconnected = true
        QuicClient.disconnect()
        store.update { it.copy(phase = LinkPhase.Searching, searchingSinceMs = System.currentTimeMillis()) }
    }

    override fun retry() {
        target?.let(::connect) ?: run {
            store.update { it.copy(phase = LinkPhase.Searching, searchingSinceMs = System.currentTimeMillis()) }
            restartDiscovery()
        }
    }

    override fun disconnect() {
        userDisconnected = true
        stopMirroring()
        QuicClient.disconnect()
    }

    override fun startMirroring() {
        val projectionManager = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        screenCapture.launch(projectionManager.createScreenCaptureIntent())
    }

    override fun stopMirroring() {
        if (store.state.value.mirroring) {
            ScreenCaptureService.stop(this)
            log("Screen sharing stopped")
        }
        store.update { it.copy(mirroring = false) }
    }

    override fun openPermission(kind: PermissionKind) {
        try {
            startActivity(permissionIntent(this, kind))
        } catch (e: Exception) {
            // Some OEM builds lack the specific screen; the app's own page always exists.
            startActivity(
                Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
                    .setData(android.net.Uri.parse("package:$packageName")),
            )
        }
    }

    override fun forgetComputer(name: String) {
        store.forgetPaired(name)
    }

    // ── Discovery ──────────────────────────────────────────────────────────

    override fun onHostDiscovered(id: String, name: String, ip: String, port: Int) {
        runOnUiThread {
            val computer = Computer(id, name, ip, port, paired = store.isPaired(name))
            store.upsertComputer(computer)
            log("Found $name ($ip:$port)")
            // A paired computer showing up means we can reconnect on our own.
            if (computer.paired && !userDisconnected && store.state.value.phase is LinkPhase.Searching) {
                connect(computer)
            }
        }
    }

    override fun onHostLost(id: String) {
        runOnUiThread { store.removeComputer(id) }
    }

    override fun onDiscoveryStarted() {
        log("Looking for computers on this Wi-Fi")
    }

    override fun onDiscoveryStopped() {
        log("Stopped looking for computers")
    }

    private fun restartDiscovery() {
        discoveryManager.stopDiscovery()
        store.update { it.copy(computers = emptyList()) }
        discoveryManager.startDiscovery()
    }

    // ── Connection events ──────────────────────────────────────────────────

    private fun showProblem(message: String, canRetry: Boolean) {
        val name = target?.name ?: "your computer"
        store.update { it.copy(phase = LinkPhase.Problem(name, message, canRetry)) }
    }

    override fun onPairingPin(pin: Int) {
        log("Pairing code shown")
        runOnUiThread {
            val name = target?.name ?: "your computer"
            store.update { it.copy(phase = LinkPhase.Pairing(name, pin, confirmed = false)) }
        }
    }

    override fun onPaired() {
        log("Pairing complete")
    }

    override fun onConnected() {
        val name = target?.name ?: "your computer"
        log("Connected to $name")
        runOnUiThread {
            // Connecting at all means the computer is trusted on this phone.
            store.rememberPaired(name)
            store.update { it.copy(phase = LinkPhase.Connected(name)) }
            discoveryManager.stopDiscovery()
        }

        ConnectionKeepAliveService.start(this, name)

        val fingerprint = try {
            QuicClient.ownFingerprint()
        } catch (e: Exception) {
            null
        }
        if (fingerprint != null) {
            proximityService?.startRanging(fingerprint)
            log("Proximity pre-warm ranging started")
        } else {
            log("Proximity pre-warm not started: own fingerprint unavailable")
        }
    }

    override fun onDisconnected(reason: String) {
        log("Disconnected: $reason")
        proximityService?.stopRanging()
        ConnectionKeepAliveService.stop(this)
        runOnUiThread { handleDisconnect(reason) }
    }

    /** Turns a disconnect reason into the next state and plain-language copy. */
    private fun handleDisconnect(reason: String) {
        val previous = store.state.value.phase
        val name = target?.name ?: "your computer"
        val wasPaired = store.isPaired(name)
        if (store.state.value.mirroring) {
            ScreenCaptureService.stop(this)
            store.update { it.copy(mirroring = false) }
        }
        val lower = reason.lowercase()
        when {
            reason == "closed_locally" || userDisconnected -> {
                store.update { it.copy(phase = LinkPhase.Searching, searchingSinceMs = System.currentTimeMillis()) }
                discoveryManager.startDiscovery()
            }
            reason == "pairing_timed_out" ->
                showProblem("$name didn't get an answer in time. Choose Start Pairing on that computer again, then tap it here.", canRetry = true)
            reason == "pairing_rejected" ->
                showProblem("$name didn't accept the pairing. Make sure you pick the matching code, then try again.", canRetry = true)
            reason == "device_revoked" -> {
                store.forgetPaired(name)
                showProblem("This phone was removed on $name. To reconnect, choose Start Pairing on that computer, then tap it here.", canRetry = true)
            }
            reason == "pairing_closed" || "handshake" in lower || "certificate" in lower || "aborted" in lower ->
                if (wasPaired) {
                    showProblem("$name doesn't recognise this phone anymore. Choose Start Pairing on that computer, then try again.", canRetry = true)
                } else {
                    showProblem("$name isn't ready to pair. On that computer, choose Start Pairing, then try again.", canRetry = true)
                }
            previous is LinkPhase.Pairing ->
                showProblem("Pairing didn't finish. Try again.", canRetry = true)
            previous is LinkPhase.Connected -> {
                // Lost an established link (Wi-Fi blip, computer asleep): go back
                // to looking for it; it reconnects as soon as it's seen again.
                store.update { it.copy(phase = LinkPhase.Searching, searchingSinceMs = System.currentTimeMillis()) }
                main.postDelayed({ restartDiscovery() }, RECONNECT_DELAY_MS)
            }
            "timed out" in lower || reason == "timed_out" ->
                showProblem("Couldn't reach $name. Check that it's on and on the same Wi-Fi as this phone.", canRetry = true)
            else ->
                showProblem("Lost the connection to $name. Try again.", canRetry = true)
        }
    }

    override fun onMessage(streamType: Byte, payload: ByteArray) {
        log("Message on stream $streamType (${payload.size} bytes)")
    }

    override fun onKeyframeRequest() {
        ScreenCaptureService.requestKeyframe()
    }

    override fun onVideoStreamReady() {
        log("Computer is ready to show your screen")
    }

    // ── Input injection (from the computer) ────────────────────────────────

    private var dragStartXNorm: Int = 0
    private var dragStartYNorm: Int = 0
    private var dragStartTimeMs: Long = 0L
    private var isDragging: Boolean = false

    override fun onPointerEvent(action: Int, button: Int, xNorm: Int, yNorm: Int, pressure: Int) {
        val service = InputService.instance
        if (service == null) {
            Log.d(TAG, "Input event dropped: InputService accessibility service not enabled")
            return
        }

        when (action) {
            1 -> { // Down
                dragStartXNorm = xNorm
                dragStartYNorm = yNorm
                dragStartTimeMs = System.currentTimeMillis()
                isDragging = true
            }
            3 -> { // Move
                // Pointer motion tracking
            }
            2 -> { // Up
                if (isDragging) {
                    val dx = Math.abs(xNorm - dragStartXNorm)
                    val dy = Math.abs(yNorm - dragStartYNorm)
                    val duration = (System.currentTimeMillis() - dragStartTimeMs).coerceIn(50L, 500L)
                    // If moved beyond ~1.5% of normalized screen dimension, treat as swipe/drag
                    if (dx > 1000 || dy > 1000) {
                        service.injectSwipe(dragStartXNorm, dragStartYNorm, xNorm, yNorm, duration)
                    } else {
                        service.injectTap(dragStartXNorm, dragStartYNorm)
                    }
                    isDragging = false
                }
            }
            4 -> { // Cancel
                isDragging = false
            }
        }
    }

    override fun onKeyEvent(action: Int, keycode: Int, modifiers: Int) {
        InputService.instance?.injectKey(action, keycode, modifiers)
    }

    override fun onScrollEvent(dx: Int, dy: Int, xNorm: Int, yNorm: Int) {
        InputService.instance?.injectScroll(dy, xNorm, yNorm)
    }

    override fun onNavEvent(action: Int) {
        InputService.instance?.injectNavAction(action)
    }

    // ── Notifications, clipboard, handoff ──────────────────────────────────

    override fun onNotificationAction(key: String, actionId: Int) {
        log("Running notification action $actionId on $key")
        NotificationService.instance?.invokeAction(key, actionId)
    }

    override fun onNotificationDismiss(key: String) {
        log("Dismissing notification $key from computer")
        NotificationService.instance?.dismiss(key)
    }

    override fun onDndSync(enabled: Boolean) {
        log("Do Not Disturb from computer: $enabled")
        val nm = getSystemService(NOTIFICATION_SERVICE) as android.app.NotificationManager
        if (nm.isNotificationPolicyAccessGranted) {
            val filter = if (enabled) {
                android.app.NotificationManager.INTERRUPTION_FILTER_PRIORITY
            } else {
                android.app.NotificationManager.INTERRUPTION_FILTER_ALL
            }
            nm.setInterruptionFilter(filter)
        } else {
            log("Do Not Disturb sync skipped: access not granted")
        }
    }

    override fun onClipboardReceived(originId: String, contentType: Int, mimeType: String, payload: ByteArray) {
        log("Clipboard from $originId ($mimeType, ${payload.size} bytes)")
        ClipboardService.instance?.writeRemoteClip(originId, contentType, mimeType, payload)
    }

    override fun onHandoffReceived(sessionId: Long, handoffType: Int, appId: String, uri: String, title: String, stateJson: String) {
        log("Handoff from computer: \"$title\"")
        val service = handoffService
        if (service == null) {
            Log.w(TAG, "Handoff dropped: HandoffService not bound yet")
            return
        }
        service.handleIncomingHandoff(this, sessionId, handoffType, appId, uri, title, stateJson)
    }

    override fun onAgentConsentUpdated(allowNotifications: Boolean, allowClipboard: Boolean, allowRawVideo: Boolean) {
        // The ambient-agent consent policy is enforced host-side (AgentConsentPolicy
        // gates AmbientEventBus::query in linux/src/ambient.rs) — this device has no
        // local policy store to update yet. Logged so a host-initiated policy push is
        // at least visible on the phone rather than silently discarded.
        log("Computer updated agent consent: notifications=$allowNotifications clipboard=$allowClipboard rawVideo=$allowRawVideo")
    }
}
