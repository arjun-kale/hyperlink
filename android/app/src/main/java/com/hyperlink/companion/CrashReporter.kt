package com.hyperlink.companion

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import java.io.File
import java.io.PrintWriter
import java.io.StringWriter
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Local, privacy-respecting crash reporting (Phase 11).
 *
 * Mirrors `linux/src/crash_report.rs`'s contract exactly: reports are written
 * only to this app's private storage (`filesDir/crash_reports`, not
 * externally readable by other apps) and are never transmitted anywhere —
 * there is no telemetry backend in this project. A report contains the
 * exception class, message, and stack trace (code locations and
 * developer-written messages), never phone content such as clipboard text,
 * notification bodies, mirrored screen pixels, file names from the virtual
 * mount, or key/certificate material — none of that is reachable from an
 * uncaught-exception handler's `Thread`/`Throwable` parameters.
 */
object CrashReporter {
    private const val TAG = "CrashReporter"
    private const val PREFS_NAME = "hyperlink_prefs"
    private const val KEY_ENABLED = "crash_reporting_enabled"
    private const val REPORTS_DIR = "crash_reports"

    private var installed = false

    /** Installs the handler once, chaining to whatever handler was previously set. */
    @Synchronized
    fun install(context: Context) {
        if (installed) return
        installed = true

        val appContext = context.applicationContext
        val previousHandler = Thread.getDefaultUncaughtExceptionHandler()

        Thread.setDefaultUncaughtExceptionHandler { thread, throwable ->
            try {
                if (isEnabled(appContext)) {
                    writeReport(appContext, thread, throwable)
                } else {
                    Log.w(TAG, "crash reporting disabled — enable it in Settings to save a report next time")
                }
            } catch (e: Exception) {
                // Never let report-writing itself mask the original crash.
                Log.e(TAG, "failed to write crash report", e)
            }
            // Always chain to the previous handler (e.g. the system default,
            // which shows the "app has stopped" dialog and terminates the
            // process) — this handler is additive, not a replacement.
            previousHandler?.uncaughtException(thread, throwable)
        }
    }

    private fun prefs(context: Context): SharedPreferences =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /** Defaults to enabled — matches `HostPreferences::default()` on the Linux side:
     * 100% local, never transmitted, and useful for the user's own troubleshooting. */
    fun isEnabled(context: Context): Boolean = prefs(context).getBoolean(KEY_ENABLED, true)

    fun setEnabled(context: Context, enabled: Boolean) {
        prefs(context).edit().putBoolean(KEY_ENABLED, enabled).apply()
    }

    fun reportsDir(context: Context): File = File(context.filesDir, REPORTS_DIR)

    fun listReports(context: Context): List<File> =
        reportsDir(context).listFiles { f -> f.extension == "txt" }
            ?.sortedDescending()
            ?: emptyList()

    fun clearReports(context: Context) {
        listReports(context).forEach { it.delete() }
    }

    private fun writeReport(context: Context, thread: Thread, throwable: Throwable) {
        val dir = reportsDir(context)
        if (!dir.exists()) dir.mkdirs()

        val timestamp = System.currentTimeMillis()
        val file = File(dir, "crash_$timestamp.txt")

        val stackTrace = StringWriter().also { throwable.printStackTrace(PrintWriter(it)) }.toString()
        val isoTime = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss", Locale.US).format(Date(timestamp))

        file.writeText(
            buildString {
                appendLine("HyperLink companion crash report")
                appendLine("timestamp: $isoTime ($timestamp)")
                appendLine("version_name: ${BuildConfig.VERSION_NAME}")
                appendLine("version_code: ${BuildConfig.VERSION_CODE}")
                appendLine("thread: ${thread.name}")
                appendLine("exception: ${throwable.javaClass.name}")
                appendLine("message: ${throwable.message}")
                appendLine("stack_trace:")
                append(stackTrace)
            }
        )
        Log.i(TAG, "crash report written to ${file.absolutePath}")
    }
}
