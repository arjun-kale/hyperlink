package com.hyperlink.companion

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.os.Bundle
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.IBinder
import android.util.Log
import android.view.WindowManager
import android.view.WindowMetrics
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

class ScreenCaptureService : Service() {

    companion object {
        private const val TAG = "ScreenCaptureService"
        private const val CHANNEL_ID = "hyperlink_screen_capture"
        private const val CHANNEL_NAME = "HyperLink Screen Capture"
        private const val NOTIFICATION_ID = 1
        private const val EXTRA_RESULT_CODE = "result_code"
        private const val EXTRA_DATA = "data"

        private const val MAX_WIDTH = 1080
        // Upper bound for variable bitrate: plenty for sharp text on a LAN, and a
        // static screen costs almost nothing.
        private const val BITRATE = 8_000_000 // 8 Mbps
        private const val FRAME_RATE = 60
        /** When the screen is static, re-send the last frame after this long, so a
         *  dropped frame doesn't leave a stale image on the computer. */
        private const val REPEAT_FRAME_AFTER_US = 100_000L
        private const val I_FRAME_INTERVAL = 1 // seconds
        private const val MIME_TYPE = MediaFormat.MIMETYPE_VIDEO_AVC

        fun start(context: Context, resultCode: Int, data: Intent) {
            val intent = Intent(context, ScreenCaptureService::class.java).apply {
                putExtra(EXTRA_RESULT_CODE, resultCode)
                putExtra(EXTRA_DATA, data)
            }
            context.startForegroundService(intent)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, ScreenCaptureService::class.java))
        }

        @Volatile
        private var instance: ScreenCaptureService? = null

        /** Asks the encoder for a keyframe now (the computer lost a frame). */
        fun requestKeyframe() {
            val codec = instance?.encoder ?: return
            try {
                codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
            } catch (e: IllegalStateException) {
                // Encoder is stopping; nothing to recover.
            }
        }
    }

    private var mediaProjection: MediaProjection? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var encoder: MediaCodec? = null
    private var encoderThread: Thread? = null
    private val isEncoding = AtomicBoolean(false)
    private val frameCounter = AtomicInteger(0)

    private var captureWidth = 0
    private var captureHeight = 0
    private var densityDpi = 0

    override fun onBind(intent: Intent?): IBinder? = null

    /**
     * Keeps Wi-Fi out of power-save while sharing the screen. In power-save the
     * radio batches packets, which adds delay and bursty loss to the video.
     * Held only while sharing: it costs battery.
     */
    private var wifiLock: android.net.wifi.WifiManager.WifiLock? = null

    override fun onCreate() {
        super.onCreate()
        instance = this
        createNotificationChannel()
        wifiLock = applicationContext.getSystemService(android.net.wifi.WifiManager::class.java)
            ?.createWifiLock(android.net.wifi.WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "HyperLink:mirroring")
            ?.apply {
                setReferenceCounted(false)
                acquire()
            }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent == null) {
            stopSelf()
            return START_NOT_STICKY
        }

        // Android requires startForeground() promptly after startForegroundService(),
        // even if we're about to bail out — otherwise it kills the whole app.
        startForeground(
            NOTIFICATION_ID,
            buildNotification(),
            android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION,
        )

        // Note RESULT_OK is -1, so the "missing" default must be something else.
        val resultCode = intent.getIntExtra(EXTRA_RESULT_CODE, Int.MIN_VALUE)
        val data: Intent? = intent.getParcelableExtra(EXTRA_DATA, Intent::class.java)

        if (resultCode != android.app.Activity.RESULT_OK || data == null) {
            Log.e(TAG, "Invalid start parameters (resultCode=$resultCode, data=${data != null})")
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }

        // Compute capture dimensions
        computeCaptureDimensions()

        // Obtain MediaProjection
        val projectionManager = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        mediaProjection = projectionManager.getMediaProjection(resultCode, data)

        if (mediaProjection == null) {
            Log.e(TAG, "Failed to obtain MediaProjection")
            stopSelf()
            return START_NOT_STICKY
        }

        mediaProjection?.registerCallback(projectionCallback, null)

        // Start capture pipeline
        startCapture()

        return START_NOT_STICKY
    }

    override fun onDestroy() {
        instance = null
        wifiLock?.takeIf { it.isHeld }?.release()
        wifiLock = null
        stopCapture()
        super.onDestroy()
    }

    private fun computeCaptureDimensions() {
        val windowManager = getSystemService(WINDOW_SERVICE) as WindowManager
        val metrics: WindowMetrics = windowManager.currentWindowMetrics
        val bounds = metrics.bounds
        densityDpi = resources.displayMetrics.densityDpi

        val deviceWidth = bounds.width()
        val deviceHeight = bounds.height()

        if (deviceWidth <= MAX_WIDTH) {
            captureWidth = deviceWidth
            captureHeight = deviceHeight
        } else {
            val scale = MAX_WIDTH.toFloat() / deviceWidth.toFloat()
            captureWidth = MAX_WIDTH
            captureHeight = (deviceHeight * scale).toInt()
        }

        // Ensure dimensions are even (required by H.264)
        captureWidth = captureWidth and 0x7FFE
        captureHeight = captureHeight and 0x7FFE

        Log.i(TAG, "Capture dimensions: ${captureWidth}x${captureHeight} @ ${densityDpi}dpi")
    }

    private fun startCapture() {
        try {
            // Configure encoder
            val format = MediaFormat.createVideoFormat(MIME_TYPE, captureWidth, captureHeight).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                setInteger(MediaFormat.KEY_BIT_RATE, BITRATE)
                setInteger(MediaFormat.KEY_FRAME_RATE, FRAME_RATE)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, I_FRAME_INTERVAL)
                setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR)
                // Baseline: no B-frames, so every frame can be shown as soon as it's decoded.
                // No KEY_LEVEL: the encoder picks one valid for this resolution (the old
                // Level 3.1 only covers up to 720p).
                setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline)
                // Low latency: emit each frame immediately, and run as a realtime codec.
                setInteger(MediaFormat.KEY_LATENCY, 1)
                setInteger(MediaFormat.KEY_PRIORITY, 0)
                setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, REPEAT_FRAME_AFTER_US)
                // SPS/PPS on every keyframe, so the computer's decoder can (re)start
                // from any keyframe, not only the first one.
                setInteger(MediaFormat.KEY_PREPEND_HEADER_TO_SYNC_FRAMES, 1)
            }

            encoder = MediaCodec.createEncoderByType(MIME_TYPE).also { codec ->
                codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)

                val inputSurface = codec.createInputSurface()
                codec.start()

                // Create virtual display bound to encoder's input surface
                virtualDisplay = mediaProjection?.createVirtualDisplay(
                    "HyperLinkCapture",
                    captureWidth,
                    captureHeight,
                    densityDpi,
                    DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                    inputSurface,
                    null,
                    null
                )

                // Start encoder output thread
                isEncoding.set(true)
                frameCounter.set(0)
                configSent = false
                encoderThread = Thread({ drainEncoder(codec) }, "HyperLink-Encoder").also {
                    it.start()
                }
            }

            Log.i(TAG, "Screen capture started")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start capture: ${e.message}", e)
            stopSelf()
        }
    }

    private fun drainEncoder(codec: MediaCodec) {
        val bufferInfo = MediaCodec.BufferInfo()

        while (isEncoding.get()) {
            val outputIndex = codec.dequeueOutputBuffer(bufferInfo, 10_000) // 10ms timeout

            when {
                outputIndex == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    val newFormat = codec.outputFormat
                    Log.i(TAG, "Encoder output format changed: $newFormat")
                    handleFormatChanged(newFormat)
                }
                outputIndex >= 0 -> {
                    val outputBuffer = codec.getOutputBuffer(outputIndex) ?: continue

                    if (bufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                        // Config data (SPS/PPS) can arrive as a buffer instead of in the format.
                        val config = ByteArray(bufferInfo.size)
                        outputBuffer.position(bufferInfo.offset)
                        outputBuffer.get(config)
                        if (!configSent) sendConfigFrom(config)
                        codec.releaseOutputBuffer(outputIndex, false)
                        continue
                    }

                    if (bufferInfo.size > 0) {
                        outputBuffer.position(bufferInfo.offset)
                        outputBuffer.limit(bufferInfo.offset + bufferInfo.size)

                        val frameData = ByteArray(bufferInfo.size)
                        outputBuffer.get(frameData)

                        val isKeyframe = (bufferInfo.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME) != 0
                        // Last resort: keyframes carry SPS/PPS inline (KEY_PREPEND_HEADER_TO_SYNC_FRAMES).
                        if (isKeyframe && !configSent) sendConfigFrom(frameData)
                        val frameId = frameCounter.getAndIncrement()
                        val timestampUs = bufferInfo.presentationTimeUs

                        QuicClient.sendFrame(
                            frameData,
                            frameId,
                            timestampUs,
                            isKeyframe,
                            captureWidth,
                            captureHeight
                        )
                    }

                    codec.releaseOutputBuffer(outputIndex, false)
                }
                outputIndex == MediaCodec.INFO_TRY_AGAIN_LATER -> {
                    // No output available yet
                }
            }
        }

        Log.i(TAG, "Encoder drain loop exited")
    }

    /** Whether the computer has this stream's SPS/PPS yet (it can't decode without them). */
    @Volatile
    private var configSent = false

    private fun handleFormatChanged(format: MediaFormat) {
        val csd = listOfNotNull(format.getByteBuffer("csd-0"), format.getByteBuffer("csd-1"))
            .map { buf -> ByteArray(buf.remaining()).also { buf.get(it) } }
        if (csd.isEmpty() || !sendConfigFrom(csd.reduce { a, b -> a + b })) {
            // Some encoders (with KEY_PREPEND_HEADER_TO_SYNC_FRAMES) leave the format
            // without SPS/PPS; they're taken from a codec-config buffer or the first
            // keyframe instead (see drainEncoder).
            Log.i(TAG, "Encoder format has no SPS/PPS; will take them from the stream")
        }
    }

    /**
     * Finds the SPS and PPS in Annex-B data and sends them as the stream config,
     * without start codes (the computer adds its own). Returns true if sent.
     */
    private fun sendConfigFrom(annexB: ByteArray): Boolean {
        val nals = nalUnits(annexB)
        val sps = nals.firstOrNull { it.isNotEmpty() && (it[0].toInt() and 0x1F) == 7 } ?: return false
        val pps = nals.firstOrNull { it.isNotEmpty() && (it[0].toInt() and 0x1F) == 8 } ?: return false
        Log.i(TAG, "Sending video config: SPS=${sps.size} bytes, PPS=${pps.size} bytes")
        QuicClient.sendConfig(sps, pps, BITRATE, FRAME_RATE)
        configSent = true
        return true
    }

    /** Splits Annex-B byte-stream data into NAL units, without start codes. */
    private fun nalUnits(data: ByteArray): List<ByteArray> {
        val starts = mutableListOf<Pair<Int, Int>>() // (start-code offset, payload offset)
        var i = 0
        while (i + 3 <= data.size) {
            if (data[i].toInt() == 0 && data[i + 1].toInt() == 0) {
                if (data[i + 2].toInt() == 1) {
                    starts += i to i + 3; i += 3; continue
                }
                if (i + 4 <= data.size && data[i + 2].toInt() == 0 && data[i + 3].toInt() == 1) {
                    starts += i to i + 4; i += 4; continue
                }
            }
            i++
        }
        return starts.mapIndexed { n, (_, payload) ->
            val end = if (n + 1 < starts.size) starts[n + 1].first else data.size
            data.copyOfRange(payload, end)
        }
    }

    private fun stopCapture() {
        Log.i(TAG, "Stopping screen capture")

        isEncoding.set(false)

        encoderThread?.let { thread ->
            try {
                thread.join(2000)
            } catch (_: InterruptedException) {}
        }
        encoderThread = null

        virtualDisplay?.release()
        virtualDisplay = null

        encoder?.let { codec ->
            try {
                codec.stop()
                codec.release()
            } catch (e: Exception) {
                Log.w(TAG, "Error releasing encoder: ${e.message}")
            }
        }
        encoder = null

        mediaProjection?.unregisterCallback(projectionCallback)
        mediaProjection?.stop()
        mediaProjection = null

        Log.i(TAG, "Screen capture stopped")
    }

    private val projectionCallback = object : MediaProjection.Callback() {
        override fun onStop() {
            Log.i(TAG, "MediaProjection stopped by system")
            stopCapture()
            stopSelf()
            // Stopped outside the app (e.g. the status-bar cast chip): keep the UI honest.
            Link.store.update { it.copy(mirroring = false) }
        }
    }

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            CHANNEL_NAME,
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "Shows while HyperLink is mirroring your screen"
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(channel)
    }

    private fun buildNotification(): Notification {
        return Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("Sharing your screen")
            .setContentText("Your computer can see this screen. Stop it from the HyperLink app.")
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setOngoing(true)
            .build()
    }
}
