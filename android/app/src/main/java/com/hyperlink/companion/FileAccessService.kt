package com.hyperlink.companion

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.Settings
import android.util.Log

/**
 * Android File Access Service for Phase 6 (Lazy Virtual Mount).
 *
 * Checks and requests storage access permissions on Android 10+ / 11+
 * allowing POSIX access to /storage/emulated/0 (DCIM, Pictures, Movies, Documents).
 */
class FileAccessService(private val context: Context) {

    companion object {
        private const val TAG = "FileAccessService"

        @Volatile
        var instance: FileAccessService? = null

        fun init(context: Context): FileAccessService {
            val s = FileAccessService(context.applicationContext)
            instance = s
            return s
        }

        fun hasStoragePermission(context: Context): Boolean {
            return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                Environment.isExternalStorageManager()
            } else {
                context.checkSelfPermission(android.Manifest.permission.READ_EXTERNAL_STORAGE) ==
                        android.content.pm.PackageManager.PERMISSION_GRANTED
            }
        }

        fun requestStoragePermission(activity: Activity) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                try {
                    val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION).apply {
                        data = Uri.parse("package:${activity.packageName}")
                    }
                    activity.startActivity(intent)
                } catch (e: Exception) {
                    val intent = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
                    activity.startActivity(intent)
                }
            } else {
                activity.requestPermissions(
                    arrayOf(
                        android.Manifest.permission.READ_EXTERNAL_STORAGE,
                        android.Manifest.permission.WRITE_EXTERNAL_STORAGE
                    ),
                    102
                )
            }
        }
    }

    fun getStorageRootPath(): String {
        return Environment.getExternalStorageDirectory().absolutePath
    }

    init {
        Log.i(TAG, "FileAccessService initialized with storage root: ${getStorageRootPath()}")
    }
}
