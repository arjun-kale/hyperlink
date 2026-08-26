package com.hyperlink.companion

import android.app.Application

/**
 * Application entry point (Phase 11). Installs local, opt-in, privacy-respecting
 * crash reporting before any other component runs — see [CrashReporter].
 */
class HyperLinkApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        CrashReporter.install(this)
    }
}
