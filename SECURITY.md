# Security Policy

## Supported Versions

HyperLink has not had a tagged release yet — `main` is the only supported line. There is no LTS or backport policy at this stage.

## Reporting a Vulnerability

Please **do not** open a public GitHub issue for a security vulnerability.

Instead, use GitHub's private vulnerability reporting: go to the [Security tab](https://github.com/arjun-kale/hyperlink/security) of this repository and select "Report a vulnerability." That opens a private advisory visible only to the maintainer and you, and is the fastest way to get a response.

Please include:
- A description of the vulnerability and its potential impact.
- Steps to reproduce, or a minimal proof-of-concept.
- Which component is affected (`protocol/`, `linux/`, `android/`, `android-bridge/`, `bench/`).

### What to expect

This is a solo-maintained, pre-1.0 project — there is no dedicated security team and no formal SLA. As a target, expect an initial response within a week. Fixes are prioritized by severity and by how close the affected code is to the trust boundary (the pairing/auth flow and anything touching the file-access sandbox get priority over, say, a benchmark harness bug).

## Known, Already-Documented Gaps

Before reporting, it may be worth checking whether the issue is already tracked:
- [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) — the project's threat model and current mitigation stance.
- [`docs/SECURITY_REVIEW.md`](docs/SECURITY_REVIEW.md) — a code-level review of the pairing/auth flow with specific, already-known findings and their status.
- [`CHANGELOG.md`](CHANGELOG.md) — each phase's entry lists known implementation gaps; several are security-relevant (e.g. Phase 11 notes that no third-party penetration test has been performed yet).

If your report matches something already listed there, it's still worth reporting — those documents track *known* gaps being worked on, not accepted risk.
