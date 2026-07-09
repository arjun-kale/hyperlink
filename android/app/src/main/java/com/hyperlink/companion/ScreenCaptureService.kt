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
        private const val BITRATE = 4_000_000 // 4 Mbps
        private const val FRAME_RATE = 30
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

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent == null) {
            stopSelf()
            return START_NOT_STICKY
        }

        val resultCode = intent.getIntExtra(EXTRA_RESULT_CODE, -1)
        @Suppress("DEPRECATION")
        val data: Intent? = intent.getParcelableExtra(EXTRA_DATA)

        if (resultCode == -1 || data == null) {
            Log.e(TAG, "Invalid start parameters")
            stopSelf()
            return START_NOT_STICKY
        }

        // Start foreground immediately to avoid ANR
        val notification = buildNotification()
        startForeground(NOTIFICATION_ID, notification)

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
                setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR)
                setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline)
                setInteger(MediaFormat.KEY_LEVEL, MediaCodecInfo.CodecProfileLevel.AVCLevel31)
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
                        // Config data (SPS/PPS) — can also arrive as a regular buffer
                        Log.d(TAG, "Received codec config buffer, size=${bufferInfo.size}")
                        codec.releaseOutputBuffer(outputIndex, false)
                        continue
                    }

                    if (bufferInfo.size > 0) {
                        outputBuffer.position(bufferInfo.offset)
                        outputBuffer.limit(bufferInfo.offset + bufferInfo.size)

                        val frameData = ByteArray(bufferInfo.size)
                        outputBuffer.get(frameData)

                        val isKeyframe = (bufferInfo.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME) != 0
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

    private fun handleFormatChanged(format: MediaFormat) {
        val spsBuffer: ByteBuffer? = format.getByteBuffer("csd-0")
        val ppsBuffer: ByteBuffer? = format.getByteBuffer("csd-1")

        if (spsBuffer != null && ppsBuffer != null) {
            val sps = ByteArray(spsBuffer.remaining())
            spsBuffer.get(sps)

            val pps = ByteArray(ppsBuffer.remaining())
            ppsBuffer.get(pps)

            Log.i(TAG, "Sending video config: SPS=${sps.size} bytes, PPS=${pps.size} bytes")
            QuicClient.sendConfig(sps, pps, BITRATE, FRAME_RATE)
        } else {
            Log.w(TAG, "FORMAT_CHANGED but missing SPS/PPS in format")
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
            .setContentTitle("HyperLink Screen Mirroring")
            .setContentText("Your screen is being mirrored to the host.")
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setOngoing(true)
            .build()
    }
}
