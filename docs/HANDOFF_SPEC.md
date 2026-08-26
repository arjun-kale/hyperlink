# HyperLink Scoped App-State Handoff Contract Specification
Spec version 1.0 (Phase 9)

---

## 1. Overview & Principles

Universal, arbitrary app-state migration across divergent OS architectures without developer opt-in isn't achievable without custom runtime hacks or brittle memory scraping.

HyperLink's Scoped App-State Handoff instead provides a documented, open contract based on structured URI schemes, JSON state schemas, and standard OS intent dispatching that:
1. Works out of the box for core companion surfaces (web browsing, note drafts, media playback).
2. Allows any third-party Android or Linux application to register as a handoff provider or consumer with zero proprietary SDK lock-in.

---

## 2. Wire Protocol Schema

Handoff payloads are transported over the multiplexed QUIC control stream (Stream Type `0x50`) with message discriminant `0xB0` (`HandoffPayload`) and acknowledged with `0xB1` (`HandoffAck`).

### HandoffPayload Binary Framing

| Offset (Bytes) | Field | Type | Description |
|---|---|---|---|
| 0..8 | `session_id` | `u64` (Big-Endian) | Unique session correlation ID |
| 8..10 | `origin_len` | `u16` (Big-Endian) | Length of the `source_origin` field, in bytes |
| 10..10+`origin_len` | `source_origin` | UTF-8 String | Origin identifier (e.g. `phone:GalaxyS24` or `host:LinuxDesktop`) |
| next byte | `handoff_type` | `u8` | Category: `1`=Browser, `2`=Note, `3`=Media, `4`=CustomUri |
| +2 | `app_id_len` | `u16` | Length of the `app_id` field, in bytes |
| +`app_id_len` | `app_id` | UTF-8 String | Package name or desktop app name (e.g. `org.mozilla.firefox`) |
| +2 | `uri_len` | `u16` | Length of the `uri` field, in bytes |
| +`uri_len` | `uri` | UTF-8 String | Primary target URI or deep-link |
| +2 | `title_len` | `u16` | Length of the `title` field, in bytes |
| +`title_len` | `title` | UTF-8 String | Display title of document or task |
| +4 | `state_len` | `u32` (Big-Endian) | Length of the `state_json` field, in bytes (max 64 KB) |
| +`state_len` | `state_json` | UTF-8 JSON | Structured state metadata |
| +8 | `timestamp_us` | `u64` (Big-Endian) | UTC timestamp in microseconds |

---

## 3. Supported Handoff Types & JSON Schemas

### Type 1: Browser Tab (`HandoffType::BrowserTab`)
Transfers an active web browser tab, including scroll position and page zoom level.

```json
{
  "url": "https://news.ycombinator.com/item?id=40000000",
  "title": "Hacker News - HyperLink Discussion",
  "scroll_y": 1420,
  "zoom_level": 1.0
}
```

### Type 2: Note Draft (`HandoffType::NoteDraft`)
Transfers an in-progress markdown document or text draft, retaining cursor position and document title.

```json
{
  "title": "Architecture Notes",
  "content": "# HyperLink\nContinuous compute surface between Linux and Android.",
  "cursor_offset": 54,
  "is_pinned": false
}
```

### Type 3: Media Playback (`HandoffType::MediaPlayback`)
Transfers an active audio/video playback stream with millisecond-accurate timeline position and volume.

```json
{
  "media_uri": "https://cdn.example.com/episodes/podcast_101.mp3",
  "title": "Tech Talk Podcast #101",
  "position_ms": 245000,
  "duration_ms": 3600000,
  "is_playing": true,
  "volume": 0.85
}
```

### Type 4: Custom Third-Party Deep-Link (`HandoffType::CustomUri`)
Allows any third-party app to specify a custom deep-link scheme (`hyperlink-handoff://<app_id>/<action>`) with arbitrary application state.

```json
{
  "project_id": "proj_9921",
  "active_file": "src/main.rs",
  "line_number": 128,
  "selection_start": 12,
  "selection_end": 45
}
```

---

## 4. Integration Guide for Developers

### Android App Integration
To register your Android application to receive HyperLink handoffs, declare an `<intent-filter>` in `AndroidManifest.xml`:

```xml
<activity android:name=".HandoffReceiverActivity" android:exported="true">
    <intent-filter>
        <action android:name="android.intent.action.VIEW" />
        <category android:name="android.intent.category.DEFAULT" />
        <category android:name="android.intent.category.BROWSABLE" />
        <data android:scheme="hyperlink-handoff" android:host="com.yourcompany.app" />
    </intent-filter>
</activity>
```

In your Activity's `onCreate()`:
```kotlin
val uri: Uri? = intent.data
if (uri != null && uri.scheme == "hyperlink-handoff") {
    val stateJson = uri.getQueryParameter("state")
    // Parse JSON and resume activity state.
}
```

### Linux Desktop Integration
To register your Linux desktop application, install a `.desktop` file handling the `x-scheme-handler/hyperlink-handoff` MIME type or provide an `xdg-open`-compatible handler:

```ini
[Desktop Entry]
Name=Your Linux App
Exec=your-app --handoff-uri %u
Type=Application
MimeType=x-scheme-handler/hyperlink-handoff;
```
