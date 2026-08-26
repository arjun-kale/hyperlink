# android/

Kotlin companion service. Owns screen capture (MediaProjection → MediaCodec H.264), input injection, `NotificationListenerService`, clipboard access service, and the file-serving side of the FUSE mount exposed on the Linux side.

**Status:** Phase 1 complete. Phases 2-11 implemented in code; hardware validation against the physical Definition-of-Done in each phase is still pending. See `docs/SYSTEM_DESIGN.md` and `CHANGELOG.md` for the current per-phase status.
