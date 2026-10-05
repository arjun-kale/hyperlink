package com.hyperlink.companion.ui

import android.app.NotificationManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import com.hyperlink.companion.InputService

/** Reads the current state of every permission HyperLink's features rely on. */
fun readPermissions(context: Context): Permissions {
    val nm = context.getSystemService(NotificationManager::class.java)
    val inputService = ComponentName(context, InputService::class.java).flattenToString()
    val enabledA11y = Settings.Secure.getString(
        context.contentResolver,
        Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
    ).orEmpty()
    return Permissions(
        notifications = context.packageName in NotificationManagerCompat.getEnabledListenerPackages(context),
        doNotDisturb = nm.isNotificationPolicyAccessGranted,
        control = enabledA11y.split(':').any { it.equals(inputService, ignoreCase = true) },
        files = Environment.isExternalStorageManager(),
        postNotifications = Build.VERSION.SDK_INT < 33 ||
            context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED,
    )
}

/** The system settings screen where the user grants `kind`. */
fun permissionIntent(context: Context, kind: PermissionKind): Intent = when (kind) {
    PermissionKind.Notifications -> Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
    PermissionKind.DoNotDisturb -> Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS)
    PermissionKind.Control -> Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
    PermissionKind.Files -> Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
        .setData(Uri.parse("package:${context.packageName}"))
    PermissionKind.PostNotifications -> Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
        .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
}
