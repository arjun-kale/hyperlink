// HyperLink companion app module — scaffold only.
// Implementation begins in Phase 2 (see /docs/SYSTEM_DESIGN.md).

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.hyperlink.companion"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.hyperlink.companion"
        minSdk = 31 // Android 12+ — see docs/SYSTEM_DESIGN.md hardware assumptions
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    // MediaCodec / MediaProjection: platform APIs, no extra dependency needed
    // QUIC client: TBD in Phase 1 — evaluate available Kotlin/Java QUIC libraries
    // FlatBuffers / Protobuf generated code: wired up alongside protocol/ in Phase 1
}
