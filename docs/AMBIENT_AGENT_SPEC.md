# HyperLink Ambient Context Agent Specification
Spec version 1.0 (Phase 10)

---

## 1. Overview

Unlike basic screen-mirroring or notification-mirroring tools, HyperLink exposes a privacy-gated ambient context event bus that desktop AI agents (e.g. LangGraph agents, local LLM daemons, desktop assistants) can observe and query.

### Design invariants
1. **Raw video is sandboxed by default** (`allow_raw_video: false`). There is currently no event category that carries video frames at all, so this holds by construction rather than an active filter — worth knowing if a future phase adds one, since the filter would then need to actually do something.
2. **Per-category consent gating**: telemetry is partitioned into permissions (`Notifications`, `ScreenState`, `ForegroundApp`, `DeviceStatus`, `Clipboard`, `MediaState`). Ungranted categories are filtered out at the host bus boundary (`AgentConsentPolicy::can_observe`, enforced in `AmbientEventBus::query`).
3. **Citations**: agent queries (e.g. "What happened on my phone in the last hour?") return summaries that link every fact back to a timestamped event ID.

---

## 2. Event Classification & Wire Schemas

Events are transported over the multiplexed QUIC control stream (Stream Type `0x50`) with message discriminant `0xC0` (`AmbientEventPublish`) and acknowledged via `0xC1` (`AmbientEventAck`).

### Categories (`AmbientEventCategory`)

| ID | Name | Default Consent | Description |
|---|---|---|---|
| `1` | `Notifications` | **Opt-In** | Notification titles, app package names, actions, and unread status. |
| `2` | `ScreenState` | **Granted** | Screen on/off, lock status, display orientation, resolution. |
| `3` | `ForegroundApp` | **Granted** | Active foreground package and activity name. |
| `4` | `DeviceStatus` | **Granted** | Battery level, charging state, network type (WiFi/Cellular), thermal level. |
| `5` | `Clipboard` | **Blocked by Default** | Clipboard preview snippets (requires explicit user consent). |
| `6` | `MediaState` | **Granted** | Media playback state, track title, artist, duration. |

---

## 3. Wire Protocol & Binary Framing

### AmbientEvent Framing (`0xC0`)

| Offset (Bytes) | Field | Type | Description |
|---|---|---|---|
| 0..8 | `event_id` | `u64` (Big-Endian) | Monotonically increasing unique event ID |
| 8 | `category` | `u8` | Category discriminant (`1`..=`6`) |
| 9..11 | `source_len` | `u16` (Big-Endian) | Length of the `source` field, in bytes |
| 11..11+`source_len` | `source` | UTF-8 String | Source origin (e.g. `phone:GalaxyS24`) |
| +8 | `timestamp_us` | `u64` (Big-Endian) | Event timestamp in microseconds UTC |
| +2 | `summary_len` | `u16` (Big-Endian) | Length of the `summary` field, in bytes |
| +`summary_len` | `summary` | UTF-8 String | Human-readable event description |
| +4 | `metadata_len` | `u32` (Big-Endian) | Length of the `metadata_json` field, in bytes |
| +`metadata_len` | `metadata_json` | UTF-8 JSON | Category-specific JSON metadata |

---

## 4. Agent Query Engine API

Agents can query historical phone context via IPC, CLI, or wire messages (`0xC4` `AgentQueryRequest` and `0xC5` `AgentQueryResponse`).

### Query Model (`AgentQueryRequest`)
```json
{
  "query_id": 1001,
  "agent_id": "desktop_assistant_alpha",
  "query_text": "What happened on my phone in the last hour?",
  "time_window_start_us": 1710000000000000,
  "time_window_end_us": 1710003600000000
}
```

### Synthesized Response (`AgentQueryResponse`)
```json
{
  "query_id": 1001,
  "answer_text": "Summary of phone activity over the past 60 minute(s) (4 total events observed):\n\nNotifications (2):\n• [#1001] Slack: Alice mentioned you in #general\n• [#1003] GitHub: PR #42 merged\n\nApps Used (1):\n• [#1002] Opened Firefox: Rust Programming Language\n\nMedia Playback (1):\n• [#1004] Playing: Daft Punk - Harder, Better, Faster, Stronger",
  "matched_event_ids": [1001, 1002, 1003, 1004],
  "generated_timestamp_us": 1710003600500000
}
```

---

## 5. Integrating Desktop AI Agents

### Python / LangGraph / LangChain Integration
```python
import json
import subprocess

def query_phone_context(question: str) -> str:
    result = subprocess.run(
        ["hyperlink-linux", "--agent-query", question],
        capture_output=True,
        text=True,
        check=True
    )
    return result.stdout.strip()

# Example usage in agent workflow:
recent_context = query_phone_context("What messages arrived while I was in the meeting?")
print(recent_context)
```
