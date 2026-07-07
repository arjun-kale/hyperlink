# ADR-0005: Pairing/Trust Model = Trust-On-First-Use Certificate Fingerprint

## Status
Accepted

## Context
Microsoft's Phone Link and Samsung's Link to Windows both depend on a signed-in cloud account for pairing. HyperLink is explicitly designed to have no vendor or cloud dependency.

## Decision
First pairing exchanges a certificate fingerprint via QR code or PIN — the same trust model used by KDE Connect and by Signal's safety-number verification.

## Consequences
No remote pairing is possible without physical/local proximity at least once — this is a deliberate constraint, not a limitation to "fix" later. Requires a clear re-pairing/revocation UX for lost or factory-reset devices.
