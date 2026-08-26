# Security Review — Pairing / Auth Flow (Phase 11)

## Scope and what this is not

This is a **code-level review** of the pairing and mutual-TLS authentication
flow, performed by the agent that implemented Phase 11, reading
`protocol/src/crypto.rs`, `linux/src/connection.rs`, and the Android side of
the handshake. It is **not** a third-party penetration test. `docs/SYSTEM_DESIGN.md`'s
Phase 11 DoD calls for "a real pen-test pass focused specifically on the
pairing/auth flow" — that still needs to happen, ideally by someone who
didn't write the code being reviewed. Treat this document as the groundwork
that makes that engagement faster (a map of what to attack first), not a
substitute for it.

## How pairing actually works today

1. The host is started with `--pair`. Both directions of the mTLS handshake
   (`TofuServerVerifier`/`TofuClientVerifier` in `protocol/src/crypto.rs`)
   accept **any** presented certificate while `is_pairing` is true, and
   record its SHA-256 fingerprint into `PendingPairingState`.
2. After the QUIC/TLS handshake completes, `handle_incoming_connection`
   (`linux/src/connection.rs:107`) computes a 6-digit PIN from both
   fingerprints (`generate_pairing_pin`, symmetric — same result computed
   independently on each side from the two fingerprints, order-independent)
   and prints it to the host's terminal along with `Do you trust this
   device? (y/n)`.
3. Only on explicit `y`/`yes` does the host call `add_trusted_peer` and
   persist the fingerprint to `DeviceConfig`. Anything else closes the
   connection and nothing is persisted.
4. Outside pairing mode, both verifiers reject any fingerprint not already
   in the trusted set — this part is straightforward pinned-cert
   authentication and isn't where the interesting risk lives.

**This is a real gate, not theater** — I traced it end to end rather than
trusting the docstrings; `add_trusted_peer` is genuinely unreachable without
the terminal confirmation step. That was worth checking explicitly, since
this codebase has had a running pattern of features whose wire-up doesn't
match their advertised behavior (see `CHANGELOG.md`'s per-phase "Explicit
Status & Implementation Gaps" sections) — auth is exactly the place where
that pattern would be most dangerous, so it was verified rather than assumed.

## Findings

### 1. (By design, inherent to TOFU) — pairing-mode MITM window
While `is_pairing` is true, the host accepts a TLS handshake from *any*
device, not just the intended phone. This is the standard Trust-On-First-Use
model (same as SSH's `known_hosts` on first connect, and the KDE Connect
model cited in `docs/adr/0005-pairing-trust-on-first-use.md`) — there is no
way to authenticate a certificate you've never seen before except by an
out-of-band check, which is exactly what the PIN is for. This is not a
code bug; it's the acknowledged tradeoff of not requiring a cloud account
or pre-shared secret. Flagging it here because a pen-test should verify the
PIN comparison is what actually stands between "pairing mode" and "MITM",
not treat its absence as a bug to fix in code.

### 2. UX gap: the host never requires the PIN to be *typed*, only eyeballed
The terminal shows the PIN and asks a blanket y/n — it doesn't require the
operator to enter the PIN shown on the phone's screen. Security here rests
entirely on the human actually comparing two 6-digit numbers before
answering "y", and the current UX makes it easy to reflexively confirm
without looking. **Recommendation:** change the prompt to require typing the
PIN (or the phone's PIN, cross-checked against the locally-generated one)
rather than a free-form yes/no. This is a process hardening item, not
something a code fix alone resolves, since the phone side's exact pairing UI
wasn't in scope for this review pass.

### 3. No pairing-mode timeout or single-use limit — fixed in this pass
`--pair` used to keep the host in "accept any cert + prompt" mode for the
entire process lifetime, not just a bounded window after being invoked or
until one device successfully pairs. Fixed in `linux/src/connection.rs`'s
`start_server`: a `pairing_open` flag now closes the connection-accept
window (rejecting, not handshaking, any further attempts) as soon as one
pairing attempt resolves — accepted or rejected — or after a 5-minute
timeout, whichever comes first. This bounds the window at the
connection-accept level without touching `protocol/src/crypto.rs`'s
verifier or its handshake test, since that file's correctness is exactly
what this review is trying to protect, not add risk to.

Known remaining gap: this doesn't fully close a narrow race where two
connection attempts arrive back-to-back before the first one's terminal
prompt has resolved — both could still reach the y/n prompt concurrently.
Closing that fully would mean flipping the flag the instant a fingerprint is
captured (before the blocking stdin read) rather than after it resolves,
which needs `pairing_open` threaded into `handle_incoming_connection`; left
as a known follow-up rather than done here to keep this change small and
easy to verify.

### 4. Fingerprint persisted to plaintext JSON on disk
`DeviceConfig` (including `trusted_peers` and the host's own private key,
`key_pem`) is written as plaintext JSON to `~/.config/hyperlink/host_config.json`
with no additional at-rest encryption beyond the filesystem's own
permissions. This matches the threat model already documented in
`docs/THREAT_MODEL.md` (trust boundary is the local user account, not the
disk), so it's not a new finding — restating it here because a pen-test
scoped to "pairing/auth" should include checking the file's permission bits
(`ls -l ~/.config/hyperlink/`) on a fresh install, which this review did not
verify on a real filesystem.

### 5. Preferences window can revoke trust, but not live-disconnect (Phase 11, disclosed in the UI)
The new Trusted Devices page (`linux/src/preferences_window.rs`) removes a
peer from the config file and from future pairing checks, but does not
forcibly close a connection using that peer's fingerprint if one is already
open — the running connection handler holds its own snapshot of the trusted
set. This is called out directly in that page's UI copy rather than left
implicit, but is worth a pen-tester's attention: "I revoked this device" is
a security-relevant claim a user might reasonably expect to be immediate.

## Suggested next steps (not done in this pass)
- An actual third-party pen-test against the pairing flow, ideally including
  a real MITM attempt during an active `--pair` window to confirm the PIN
  comparison — not just code reading — is what stops it.
- Close the narrow concurrent-prompt race noted in finding #3's follow-up.
- Consider fix for #2 (require typing the PIN) as a larger UX change,
  coordinated with whatever the Android-side pairing screen currently shows.
- Wire live trust revocation (finding #5) if "revoke" is expected to mean
  "disconnect now" rather than "don't reconnect."
