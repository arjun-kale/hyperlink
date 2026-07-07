# Threat Model (Living Document)

This is tracked from day one so security isn't retrofitted — formalized fully, including a pen-test pass, in Phase 11.

## Assets
- Phone screen content (video stream)
- Input control of the phone (a compromised input stream is a compromised phone)
- Notification content (potentially sensitive: 2FA codes, messages)
- Clipboard content
- File contents exposed via the FUSE mount

## Trust Boundaries
- Linux host ⇄ Android companion, over local network or USB
- Android companion ⇄ underlying OS permission surfaces (`NotificationListenerService`, Accessibility Service, `MediaProjection`)

## Key Risks & Current Mitigation Stance

| Risk | Mitigation |
|---|---|
| Rogue device impersonating a paired phone/host | TLS 1.3 + TOFU cert pinning after first pairing; unknown certs rejected by default |
| Network eavesdropping on LAN | All streams travel inside the TLS 1.3 QUIC tunnel — no plaintext fallback path, ever |
| Replay of captured control messages (e.g. a captured clipboard write) | Sequence numbers / nonce per control-plane message, rejected if replayed |
| Malicious input injection if the input stream is compromised | Input only accepted from the pinned cert's active session; no unauthenticated input path |
| Proximity (UWB) used as an auth bypass | Explicitly disallowed by design — see Non-Goals in `docs/SYSTEM_DESIGN.md`. Proximity may only pre-warm a connection, never skip the cert check |
| Over-broad Android permissions (Accessibility Service for clipboard) | Scope the service as narrowly as the API allows; document exactly what it does for anyone auditing the APK |
| Sensitive notification/clipboard content logged accidentally | Structured logging redacts notification and clipboard body content by default |

## Out of Scope (for now)
- Multi-user / multi-device pairing (single phone ⇄ single host assumed initially)
- Formal third-party security audit — planned before any public 1.0 release, not before
