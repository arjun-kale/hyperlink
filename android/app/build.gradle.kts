// HyperLink companion app module.

import java.io.FileInputStream
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Release signing (Phase 11). Real credentials live in a git-ignored
// `keystore.properties` (see keystore.properties.example) — never committed.
// Its absence is not a build failure: `assembleRelease` still produces a
// (debug-signed, NOT for distribution) build for local testing of R8
// shrinking behavior, with a clear warning rather than a silent fallback.
val keystorePropertiesFile = rootProject.file("keystore.properties")
val hasReleaseKeystore = keystorePropertiesFile.exists()
val keystoreProperties = Properties().apply {
    if (hasReleaseKeystore) {
        load(FileInputStream(keystorePropertiesFile))
    }
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

    buildFeatures {
        // Needed for BuildConfig.VERSION_NAME / VERSION_CODE in CrashReporter.
        buildConfig = true
    }

    signingConfigs {
        if (hasReleaseKeystore) {
            create("release") {
                storeFile = file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            if (hasReleaseKeystore) {
                signingConfig = signingConfigs.getByName("release")
            } else {
                logger.warn(
                    "⚠ No keystore.properties found — assembleRelease will fall back to " +
                        "debug signing. That output is fine for testing R8 shrinking locally " +
                        "but MUST NOT be distributed. See android/keystore.properties.example.",
                )
            }
        }
        debug {
            // Default debug signing config (Android's auto-generated debug.keystore).
        }
    }
}

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.7.3")
    // androidx.core.content.FileProvider — used by ClipboardService to hand a
    // received image clip to ClipboardManager as a content:// Uri.
    implementation("androidx.core:core-ktx:1.13.1")
}
