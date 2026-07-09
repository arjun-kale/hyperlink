package com.hyperlink.companion

import android.app.Activity
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast

class MainActivity : Activity(), DiscoveryManager.DiscoveryListener, QuicClient.EventListener {
    companion object {
        private const val TAG = "MainActivity"
        private const val REQUEST_MEDIA_PROJECTION = 1001
    }

    private lateinit var statusText: TextView
    private lateinit var scanButton: Button
    private lateinit var hostsContainer: LinearLayout
    private lateinit var pairingContainer: LinearLayout
    private lateinit var pairingPinText: TextView
    private lateinit var confirmButton: Button
    private lateinit var controlContainer: LinearLayout
    private lateinit var messageInput: EditText
    private lateinit var sendButton: Button
    private lateinit var startMirrorButton: Button
    private lateinit var stopMirrorButton: Button
    private lateinit var logsText: TextView

    private lateinit var discoveryManager: DiscoveryManager
    private val discoveredHosts = mutableMapOf<String, Pair<String, Int>>()
    private var isScanning = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        
        // Initialize Core Clients
        discoveryManager = DiscoveryManager(this, this)
        QuicClient.init(this, this)

        // Programmatically Build Premium Dark Theme UI
        val mainLayout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor("#121212")) // Dark Mode background
            setPadding(dp(16), dp(24), dp(16), dp(24))
        }

        // Header Title
        val titleView = TextView(this).apply {
            text = "HyperLink Companion"
            textColor(Color.parseColor("#00E5FF")) // Teal Accent
            textSize = 24f
            typeface = Typeface.create("sans-serif-medium", Typeface.BOLD)
            gravity = Gravity.CENTER_HORIZONTAL
        }
        mainLayout.addView(titleView)

        // Status Panel Card
        val statusCard = createCardLayout().apply {
            addView(TextView(this@MainActivity).apply {
                text = "Connection Status"
                textColor(Color.parseColor("#B0BEC5"))
                textSize = 12f
            })
            statusText = TextView(this@MainActivity).apply {
                text = "Disconnected"
                textColor(Color.WHITE)
                textSize = 18f
                typeface = Typeface.DEFAULT_BOLD
            }
            addView(statusText)
        }
        mainLayout.addView(statusCard, margin(0, 16, 0, 0))

        // Scanner Section
        val scanSection = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        scanSection.addView(TextView(this).apply {
            text = "Discovered Host Daemons"
            textColor(Color.WHITE)
            textSize = 16f
            typeface = Typeface.DEFAULT_BOLD
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        })
        scanButton = createAccentButton("Scan").apply {
            setOnClickListener { toggleScan() }
        }
        scanSection.addView(scanButton)
        mainLayout.addView(scanSection, margin(0, 24, 0, 0))

        // Scrollable Hosts List
        val scrollView = ScrollView(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                dp(120)
            )
        }
        hostsContainer = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        scrollView.addView(hostsContainer)
        mainLayout.addView(scrollView, margin(0, 8, 0, 0))

        // Pairing Action Card (Hidden by default)
        pairingContainer = createCardLayout().apply {
            visibility = View.GONE
            addView(TextView(this@MainActivity).apply {
                text = "Mutual Pairing Required"
                textColor(Color.parseColor("#FF5252")) // Red accent
                textSize = 14f
                typeface = Typeface.DEFAULT_BOLD
            })
            pairingPinText = TextView(this@MainActivity).apply {
                text = "000000"
                textColor(Color.WHITE)
                textSize = 48f
                gravity = Gravity.CENTER
                typeface = Typeface.create("monospace", Typeface.BOLD)
                setPadding(0, dp(12), 0, dp(12))
            }
            addView(pairingPinText)
            addView(TextView(this@MainActivity).apply {
                text = "Confirm this matches host terminal before pairing."
                textColor(Color.parseColor("#B0BEC5"))
                textSize = 12f
                gravity = Gravity.CENTER
            })
            confirmButton = createAccentButton("Confirm & Save Pairing").apply {
                setOnClickListener { confirmPairingFlow() }
            }
            addView(confirmButton, margin(0, 12, 0, 0))
        }
        mainLayout.addView(pairingContainer, margin(0, 16, 0, 0))

        // Session Controller Card (Hidden by default)
        controlContainer = createCardLayout().apply {
            visibility = View.GONE
            addView(TextView(this@MainActivity).apply {
                text = "Secure Control Channel"
                textColor(Color.parseColor("#00E5FF"))
                textSize = 14f
                typeface = Typeface.DEFAULT_BOLD
            })
            messageInput = EditText(this@MainActivity).apply {
                hint = "Enter text message to send..."
                setHintTextColor(Color.parseColor("#78909C"))
                setTextColor(Color.WHITE)
                textSize = 14f
                setPadding(dp(8), dp(12), dp(8), dp(12))
                background = GradientDrawable().apply {
                    setColor(Color.parseColor("#2C2C2C"))
                    cornerRadius = dp(6).toFloat()
                }
            }
            addView(messageInput, margin(0, 12, 0, 0))
            sendButton = createAccentButton("Send Message").apply {
                setOnClickListener { sendMessageFlow() }
            }
            addView(sendButton, margin(0, 12, 0, 0))

            // -- Mirroring Controls --
            addView(TextView(this@MainActivity).apply {
                text = "Screen Mirroring"
                textColor(Color.parseColor("#00E5FF"))
                textSize = 14f
                typeface = Typeface.DEFAULT_BOLD
            }, margin(0, 16, 0, 0))

            startMirrorButton = createAccentButton("Start Mirroring").apply {
                setOnClickListener { requestScreenCapture() }
            }
            addView(startMirrorButton, margin(0, 8, 0, 0))

            stopMirrorButton = createAccentButton("Stop Mirroring").apply {
                setOnClickListener { stopMirroring() }
                visibility = View.GONE
            }
            addView(stopMirrorButton, margin(0, 8, 0, 0))
        }
        mainLayout.addView(controlContainer, margin(0, 16, 0, 0))

        // Logs/Terminal Output
        val logsHeader = TextView(this).apply {
            text = "Activity Log"
            textColor(Color.parseColor("#B0BEC5"))
            textSize = 14f
            typeface = Typeface.DEFAULT_BOLD
        }
        mainLayout.addView(logsHeader, margin(0, 16, 0, 0))

        val logsScrollView = ScrollView(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f
            )
        }
        logsText = TextView(this).apply {
            text = "Ready.\n"
            textColor(Color.parseColor("#4CAF50")) // Green terminal text
            textSize = 12f
            typeface = Typeface.MONOSPACE
        }
        logsScrollView.addView(logsText)
        mainLayout.addView(logsScrollView, margin(0, 8, 0, 0))

        setContentView(mainLayout)
    }

    override fun onDestroy() {
        super.onDestroy()
        discoveryManager.stopDiscovery()
    }

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQUEST_MEDIA_PROJECTION) {
            if (resultCode == RESULT_OK && data != null) {
                log("MediaProjection permission granted. Starting capture...")
                ScreenCaptureService.start(this, resultCode, data)
                runOnUiThread {
                    startMirrorButton.visibility = View.GONE
                    stopMirrorButton.visibility = View.VISIBLE
                }
            } else {
                log("MediaProjection permission denied.")
                Toast.makeText(this, "Screen capture permission denied", Toast.LENGTH_SHORT).show()
            }
        }
    }

    // --- UI Layout Helpers ---

    private fun dp(valPx: Int): Int {
        return TypedValue.applyDimension(
            TypedValue.COMPLEX_UNIT_DIP,
            valPx.toFloat(),
            resources.displayMetrics
        ).toInt()
    }

    private fun TextView.textColor(color: Int) {
        setTextColor(color)
    }

    private fun margin(left: Int, top: Int, right: Int, bottom: Int): LinearLayout.LayoutParams {
        return LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        ).apply {
            setMargins(dp(left), dp(top), dp(right), dp(bottom))
        }
    }

    private fun createCardLayout(): LinearLayout {
        return LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(16), dp(16), dp(16))
            background = GradientDrawable().apply {
                setColor(Color.parseColor("#1E1E1E")) // Sleek grey card
                cornerRadius = dp(12).toFloat()
            }
        }
    }

    private fun createAccentButton(btnText: String): Button {
        return Button(this).apply {
            text = btnText
            setTextColor(Color.BLACK)
            textSize = 12f
            typeface = Typeface.DEFAULT_BOLD
            isAllCaps = false
            setPadding(dp(16), dp(4), dp(16), dp(4))
            background = GradientDrawable().apply {
                setColor(Color.parseColor("#00E5FF")) // Bright Teal button
                cornerRadius = dp(8).toFloat()
            }
        }
    }

    private fun log(message: String) {
        runOnUiThread {
            logsText.append("$message\n")
        }
    }

    // --- Controller Flows ---

    private fun toggleScan() {
        if (isScanning) {
            discoveryManager.stopDiscovery()
            scanButton.text = "Scan"
            isScanning = false
            log("Scan stopped.")
        } else {
            discoveredHosts.clear()
            hostsContainer.removeAllViews()
            discoveryManager.startDiscovery()
            scanButton.text = "Stop"
            isScanning = true
            log("Scanning local network for HyperLink daemons...")
        }
    }

    private fun togglePairingCard(show: Boolean) {
        runOnUiThread {
            pairingContainer.visibility = if (show) View.VISIBLE else View.GONE
        }
    }

    private fun toggleControlCard(show: Boolean) {
        runOnUiThread {
            controlContainer.visibility = if (show) View.VISIBLE else View.GONE
        }
    }

    private fun confirmPairingFlow() {
        if (QuicClient.confirm()) {
            Toast.makeText(this, "Pairing approved!", Toast.LENGTH_SHORT).show()
            log("Pairing confirmed and identity fingerprint persisted.")
            togglePairingCard(false)
        } else {
            Toast.makeText(this, "Failed to confirm pairing", Toast.LENGTH_SHORT).show()
        }
    }

    private fun requestScreenCapture() {
        val projectionManager = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        @Suppress("DEPRECATION")
        startActivityForResult(projectionManager.createScreenCaptureIntent(), REQUEST_MEDIA_PROJECTION)
        log("Requesting screen capture permission...")
    }

    private fun stopMirroring() {
        ScreenCaptureService.stop(this)
        log("Screen mirroring stopped.")
        runOnUiThread {
            startMirrorButton.visibility = View.VISIBLE
            stopMirrorButton.visibility = View.GONE
        }
    }

    private fun sendMessageFlow() {
        val msg = messageInput.text.toString()
        if (msg.isNotEmpty()) {
            val bytes = msg.toByteArray()
            if (QuicClient.send(bytes)) {
                log("→ sent: $msg")
                messageInput.setText("")
            } else {
                log("❌ send failed (control channel inactive)")
            }
        }
    }

    // --- Discovery Callback Listeners ---

    override fun onHostDiscovered(name: String, ip: String, port: Int) {
        runOnUiThread {
            val key = "$name@$ip:$port"
            if (discoveredHosts.containsKey(key)) return@runOnUiThread
            discoveredHosts[key] = Pair(ip, port)

            log("Discovered: $name ($ip:$port)")

            // Add dynamic list item for host
            val hostRow = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                setPadding(dp(12), dp(12), dp(12), dp(12))
                background = GradientDrawable().apply {
                    setColor(Color.parseColor("#262626"))
                    cornerRadius = dp(8).toFloat()
                }
                gravity = Gravity.CENTER_VERTICAL
                setOnClickListener {
                    log("Tapped host: $name. Initiating connection...")
                    // If not paired, connect in pairing mode first
                    QuicClient.connect(ip, port, isPairing = true)
                }
            }

            val label = TextView(this).apply {
                text = "$name\n$ip:$port"
                textColor(Color.WHITE)
                textSize = 14f
                layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
            }
            hostRow.addView(label)

            val actionBtn = createAccentButton("Pair").apply {
                isClickable = false
                isFocusable = false
            }
            hostRow.addView(actionBtn)

            hostsContainer.addView(hostRow, margin(0, 6, 0, 0))
        }
    }

    override fun onDiscoveryStarted() {
        log("NSD Discovery service active.")
    }

    override fun onDiscoveryStopped() {
        log("NSD Discovery service suspended.")
    }

    // --- JNI QUIC Client Callback Listeners ---

    override fun onPairingPin(pin: Int) {
        log("Received pairing PIN request: $pin")
        runOnUiThread {
            pairingPinText.text = String.format("%06d", pin)
            statusText.text = "Pairing PIN Validation"
            statusText.textColor(Color.parseColor("#FF9800")) // Orange
            togglePairingCard(true)
        }
    }

    override fun onConnected() {
        log("Connected to secure server!")
        runOnUiThread {
            statusText.text = "Connected"
            statusText.textColor(Color.parseColor("#4CAF50")) // Green
            togglePairingCard(false)
            toggleControlCard(true)
        }
    }

    override fun onDisconnected(reason: String) {
        log("Disconnected from server: $reason")
        runOnUiThread {
            statusText.text = "Disconnected"
            statusText.textColor(Color.RED)
            togglePairingCard(false)
            toggleControlCard(false)
        }
    }

    override fun onMessage(streamType: Byte, payload: ByteArray) {
        val txt = String(payload)
        log("← echoed: $txt")
    }

    override fun onVideoStreamReady() {
        Log.i(TAG, "Video stream ready event received")
        log("Video stream ready — host is accepting video.")
    }
}
