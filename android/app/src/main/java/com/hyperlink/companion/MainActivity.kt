package com.hyperlink.companion

import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.hyperlink.companion.ui.Computer
import com.hyperlink.companion.ui.HyperLinkApp
import com.hyperlink.companion.ui.HyperLinkTheme
import com.hyperlink.companion.ui.PermissionKind
import com.hyperlink.companion.ui.UiActions
import com.hyperlink.companion.ui.permissionIntent
import com.hyperlink.companion.ui.readPermissions

/**
 * The app's only screen. It renders [Link]'s state and forwards taps to it;
 * the link itself lives in [Link] and survives this activity being closed.
 * Only things that need an activity happen here: the screen-capture consent
 * dialog, opening system settings, and the notification permission prompt.
 */
class MainActivity : ComponentActivity(), UiActions {
    companion object {
        private const val REQUEST_POST_NOTIFICATIONS = 1002
    }

    private val screenCapture = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val data = result.data
        if (result.resultCode == RESULT_OK && data != null) {
            ScreenCaptureService.start(this, result.resultCode, data)
            Link.onMirroringStarted()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val state by Link.store.state.collectAsStateWithLifecycle()
            HyperLinkTheme {
                HyperLinkApp(state, this)
            }
        }
    }

    override fun onStart() {
        super.onStart()
        Link.onUiVisible(true)
    }

    override fun onResume() {
        super.onResume()
        // Permissions are granted in system Settings; re-read them on every return.
        Link.store.update { it.copy(permissions = readPermissions(this)) }
    }

    override fun onStop() {
        super.onStop()
        Link.onUiVisible(false)
    }

    // ── UiActions ──────────────────────────────────────────────────────────

    override fun finishOnboarding() {
        Link.store.setOnboarded()
        // Needed for the "linked" notification that keeps the link alive.
        if (Build.VERSION.SDK_INT >= 33) {
            requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), REQUEST_POST_NOTIFICATIONS)
        }
    }

    override fun connect(computer: Computer) = Link.connect(computer)
    override fun confirmPairing() = Link.confirmPairing()
    override fun cancelPairing() = Link.cancelPairing()
    override fun retry() = Link.retry()
    override fun disconnect() = Link.disconnect()
    override fun stopMirroring() = Link.stopMirroring()
    override fun forgetComputer(name: String) = Link.forgetComputer(name)

    override fun startMirroring() {
        val projectionManager = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        screenCapture.launch(projectionManager.createScreenCaptureIntent())
    }

    override fun openPermission(kind: PermissionKind) {
        try {
            startActivity(permissionIntent(this, kind))
        } catch (e: Exception) {
            // Some OEM builds lack the specific screen; the app's own page always exists.
            startActivity(
                android.content.Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
                    .setData(android.net.Uri.parse("package:$packageName")),
            )
        }
    }
}
