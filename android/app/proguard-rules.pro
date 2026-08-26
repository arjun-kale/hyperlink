# HyperLink companion — R8/ProGuard rules for the release build (Phase 11).
#
# AGP/R8 already auto-keeps `native`-declaring classes and manifest-declared
# components by default, but the JNI boundary here is exact-symbol-name
# sensitive (android-bridge's Rust side hardcodes
# `Java_com_hyperlink_companion_<Class>_<method>` symbols) and a silent
# rename would only surface as an UnsatisfiedLinkError at runtime on a real
# device, not as a build failure — so these are kept explicit rather than
# relying purely on the implicit defaults.

# Classes that declare `external fun` (JNI entry points called by name from
# the Rust side) — keep the class name, and every native method's name and
# signature exactly as declared.
-keepclasseswithmembers class com.hyperlink.companion.QuicClient {
    native <methods>;
}
-keepclasseswithmembers class com.hyperlink.companion.NetworkMonitorService {
    native <methods>;
}

# Components referenced by fully-qualified name from AndroidManifest.xml.
# (AGP merges these into the keep set automatically; listed explicitly here
# too since this manifest wiring is exactly the kind of thing Phase 11 exists
# to stop from silently regressing.)
-keep class com.hyperlink.companion.MainActivity
-keep class com.hyperlink.companion.HyperLinkApplication
-keep class com.hyperlink.companion.ScreenCaptureService
-keep class com.hyperlink.companion.InputService
-keep class com.hyperlink.companion.NotificationService
-keep class com.hyperlink.companion.ProximityRangingService
-keep class com.hyperlink.companion.HandoffService
-keep class com.hyperlink.companion.AmbientContextProvider
