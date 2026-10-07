package com.hyperlink.companion

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import com.hyperlink.companion.ui.Computer
import com.hyperlink.companion.ui.LinkPhase
import com.hyperlink.companion.ui.LinkStore

/**
 * The link to the computer, for the whole app process — not tied to any screen.
 *
 * Owns discovery, connecting, pairing, reconnecting, and handling everything
 * the computer sends (input, notification actions, clipboard, handoff). The
 * UI only renders [store] and calls the public methods here, so closing or
 * swiping away the app doesn't drop the link: [ConnectionKeepAliveService]
 * keeps the process alive while linked, and for a while after an unexpected
 * drop so it can reconnect on its own.
 */
object Link : DiscoveryManager.DiscoveryListener, QuicClient.EventListener {
    private const val TAG = "Link"
    /** Give up on a connection attempt that hasn't resolved by then. */
    private const val CONNECT_TIMEOUT_MS = 15_000L
    /** After losing a connection, look again for the computer after this. */
    private const val RECONNECT_DELAY_MS = 2_500L
    /** How long to keep trying in the background after an unexpected drop. */
    private const val RECONNECT_WINDOW_MS = 5 * 60_000L
    /** While reconnecting, how often to look again for the computer. */
    private const val RECONNECT_RETRY_MS = 10_000L

    lateinit var store: LinkStore
        private set
    private lateinit var app: Context
    private lateinit var discovery: DiscoveryManager
    private val main = Handler(Looper.getMainLooper())

    /** The computer the current (or last) connection attempt is for. */
    private var target: Computer? = null
    /** Set when the user disconnects, so we don't immediately reconnect. */
    private var userDisconnected = false
    private var attemptId = 0
    /** Whether a screen is showing; discovery runs then, or while reconnecting. */
    private var uiVisible = false
    /** Until when we keep reconnecting in the background after a drop (0 = not). */
    private var reconnectUntilMs = 0L

    private var proximityService: ProximityRangingService? = null
    private var handoffService: HandoffService? = null

    private val proximityConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            proximityService = (binder as? ProximityRangingService.LocalBinder)?.getService()
        }
        override fun onServiceDisconnected(name: ComponentName?) {
            proximityService = null
        }
    }

    private val handoffConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            handoffService = (binder as? HandoffService.LocalBinder)?.getService()
        }
        override fun onServiceDisconnected(name: ComponentName?) {
            handoffService = null
        }
    }

    private val ambientConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {}
        override fun onServiceDisconnected(name: ComponentName?) {}
    }

    /** Called once, from [HyperLinkApplication]. */
    fun init(context: Context) {
        app = context.applicationContext
        store = LinkStore(app)
        discovery = DiscoveryManager(app, this)
        QuicClient.init(app, this)
        // The name the user gave this phone (Settings → About phone), shown on the computer.
        val name = android.provider.Settings.Global.getString(app.contentResolver, android.provider.Settings.Global.DEVICE_NAME)
            ?: android.os.Build.MODEL
        QuicClient.setDeviceName(name)
        ClipboardService.init(app).start()
        FileAccessService.init(app)
        NetworkMonitorService.init(app)
        // Phase 8/9/10 services are real Service()s, bound for the process lifetime.
        // AmbientContextProvider's telemetry receiver is live from its onCreate;
        // publish calls are no-ops on the Rust side until a connection exists.
        app.bindService(Intent(app, ProximityRangingService::class.java), proximityConnection, Context.BIND_AUTO_CREATE)
        app.bindService(Intent(app, HandoffService::class.java), handoffConnection, Context.BIND_AUTO_CREATE)
        app.bindService(Intent(app, AmbientContextProvider::class.java), ambientConnection, Context.BIND_AUTO_CREATE)
    }

    private fun log(message: String) {
        Log.i(TAG, message)
        store.log(message)
    }

    private fun computerName() = target?.name ?: "your computer"

    private fun searching() = store.update {
        it.copy(phase = LinkPhase.Searching, searchingSinceMs = System.currentTimeMillis())
    }

    private fun reconnecting() = System.currentTimeMillis() < reconnectUntilMs

    // ── Visibility & discovery ─────────────────────────────────────────────

    fun onUiVisible(visible: Boolean) {
        uiVisible = visible
        updateDiscovery()
    }

    private fun updateDiscovery() {
        val wanted = (uiVisible || reconnecting()) && store.state.value.phase !is LinkPhase.Connected
        if (wanted) discovery.startDiscovery() else discovery.stopDiscovery()
    }

    private fun restartDiscovery() {
        discovery.stopDiscovery()
        store.update { it.copy(computers = emptyList()) }
        updateDiscovery()
    }

    // ── Actions ────────────────────────────────────────────────────────────

    fun connect(computer: Computer) {
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
                QuicClient.disconnect()
                if (reconnecting()) {
                    // Not back yet: keep looking until the reconnect window ends.
                    searching()
                    main.postDelayed({ restartDiscovery() }, RECONNECT_RETRY_MS)
                } else {
                    showProblem(
                        "Couldn't reach ${computer.name}. Check that it's on and on the same Wi-Fi as this phone.",
                        canRetry = true,
                    )
                }
            }
        }, CONNECT_TIMEOUT_MS)
    }

    fun confirmPairing() {
        val phase = store.state.value.phase as? LinkPhase.Pairing ?: return
        if (QuicClient.confirm(phase.computer)) {
            store.update { it.copy(phase = phase.copy(confirmed = true)) }
            log("Pairing code confirmed on phone; waiting for the computer")
        } else {
            showProblem("Pairing ended before it finished. Try again.", canRetry = true)
        }
    }

    fun cancelPairing() {
        userDisconnected = true
        QuicClient.disconnect()
        searching()
    }

    fun retry() {
        target?.let(::connect) ?: run {
            searching()
            restartDiscovery()
        }
    }

    fun disconnect() {
        userDisconnected = true
        reconnectUntilMs = 0
        stopMirroring()
        QuicClient.disconnect()
    }

    fun onMirroringStarted() {
        store.update { it.copy(mirroring = true) }
    }

    fun stopMirroring() {
        if (store.state.value.mirroring) {
            ScreenCaptureService.stop(app)
            log("Screen sharing stopped")
        }
        store.update { it.copy(mirroring = false) }
    }

    fun forgetComputer(name: String) {
        store.forgetPaired(name)
    }

    // ── Discovery events ───────────────────────────────────────────────────

    override fun onHostDiscovered(id: String, name: String, ip: String, port: Int) {
        main.post {
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
        main.post { store.removeComputer(id) }
    }

    override fun onDiscoveryStarted() {
        log("Looking for computers on this Wi-Fi")
    }

    override fun onDiscoveryStopped() {
        log("Stopped looking for computers")
    }

    // ── Connection events ──────────────────────────────────────────────────

    private fun showProblem(message: String, canRetry: Boolean) {
        reconnectUntilMs = 0
        ConnectionKeepAliveService.stop(app)
        store.update { it.copy(phase = LinkPhase.Problem(computerName(), message, canRetry)) }
        updateDiscovery()
    }

    override fun onPairingPin(pin: Int) {
        log("Pairing code shown")
        main.post {
            store.update { it.copy(phase = LinkPhase.Pairing(computerName(), pin, confirmed = false)) }
        }
    }

    override fun onPaired() {
        log("Pairing complete")
    }

    override fun onConnected() {
        val name = computerName()
        log("Connected to $name")
        main.post {
            reconnectUntilMs = 0
            // Connecting at all means the computer is trusted on this phone.
            store.rememberPaired(name)
            store.update { it.copy(phase = LinkPhase.Connected(name)) }
            updateDiscovery()
            ConnectionKeepAliveService.showLinked(app, name)
        }

        val fingerprint = try {
            QuicClient.ownFingerprint()
        } catch (e: Exception) {
            null
        }
        if (fingerprint != null) {
            proximityService?.startRanging(fingerprint)
        }
    }

    override fun onDisconnected(reason: String) {
        log("Disconnected: $reason")
        proximityService?.stopRanging()
        main.post { handleDisconnect(reason) }
    }

    /** Turns a disconnect reason into the next state and plain-language copy. */
    private fun handleDisconnect(reason: String) {
        val previous = store.state.value.phase
        val name = computerName()
        val wasPaired = store.isPaired(name)
        if (store.state.value.mirroring) {
            ScreenCaptureService.stop(app)
            store.update { it.copy(mirroring = false) }
        }
        val lower = reason.lowercase()
        when {
            reason == "closed_locally" || userDisconnected -> {
                ConnectionKeepAliveService.stop(app)
                searching()
                updateDiscovery()
            }
            reason == "pairing_timed_out" ->
                showProblem("$name didn't get an answer in time. Choose Start Pairing on that computer again, then tap it here.", canRetry = true)
            reason == "pairing_rejected" ->
                showProblem("$name didn't accept the pairing. Make sure you pick the matching code, then try again.", canRetry = true)
            reason == "protocol_mismatch" ->
                showProblem("This phone and $name are running different versions of HyperLink. Update both to the latest version.", canRetry = false)
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
            previous is LinkPhase.Connected || reconnecting() -> {
                // Lost an established link (Wi-Fi blip, computer asleep): keep the
                // process alive and keep looking for a while, even with the app
                // closed; it reconnects as soon as the computer is seen again.
                if (previous is LinkPhase.Connected) {
                    reconnectUntilMs = System.currentTimeMillis() + RECONNECT_WINDOW_MS
                    main.postDelayed(::endReconnectWindowIfStale, RECONNECT_WINDOW_MS + 1_000)
                }
                ConnectionKeepAliveService.showReconnecting(app, name)
                searching()
                main.postDelayed({ restartDiscovery() }, RECONNECT_DELAY_MS)
            }
            "timed out" in lower || reason == "timed_out" ->
                showProblem("Couldn't reach $name. Check that it's on and on the same Wi-Fi as this phone.", canRetry = true)
            else ->
                showProblem("Lost the connection to $name. Try again.", canRetry = true)
        }
    }

    private fun endReconnectWindowIfStale() {
        if (store.state.value.phase is LinkPhase.Connected || reconnecting()) return
        if (reconnectUntilMs != 0L) {
            log("Stopped trying to reconnect")
            showProblem("Couldn't reconnect to ${computerName()}. HyperLink will connect again when you open it.", canRetry = true)
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
        val nm = app.getSystemService(android.app.NotificationManager::class.java)
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
        service.handleIncomingHandoff(app, sessionId, handoffType, appId, uri, title, stateJson)
    }

    override fun onAgentConsentUpdated(allowNotifications: Boolean, allowClipboard: Boolean, allowRawVideo: Boolean) {
        // The ambient-agent consent policy is enforced host-side (AgentConsentPolicy
        // gates AmbientEventBus::query in linux/src/ambient.rs) — this device has no
        // local policy store to update yet. Logged so a host-initiated policy push is
        // at least visible on the phone rather than silently discarded.
        log("Computer updated agent consent: notifications=$allowNotifications clipboard=$allowClipboard rawVideo=$allowRawVideo")
    }
}
