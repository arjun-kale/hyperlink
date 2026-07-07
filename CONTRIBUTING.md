# Contributing to HyperLink

This project follows a phase-gated build plan — see `docs/SYSTEM_DESIGN.md` before opening a PR that jumps ahead of the current phase.

## Development Process
1. Check `docs/SYSTEM_DESIGN.md` for the current phase and its Definition-of-Done.
2. Open an issue describing what you're building against that phase, if one doesn't already exist.
3. Branch naming: `phase-<n>/<short-description>` (e.g. `phase-2/video-encode-pipeline`).
4. Commits follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
5. Any change to the wire protocol (`protocol/`) requires a version bump per the strategy in `docs/SYSTEM_DESIGN.md` Phase 1, plus a corresponding ADR if it changes an existing decision.

## Definition of Done, Project-Wide
No feature merges on "it feels fast." If a change touches the video, input, notification, clipboard, or file path, it ships with a measured number from `bench/`, logged in the PR description.

## Architecture Decisions
Significant technical decisions are recorded in `docs/adr/`. If a PR changes or challenges an existing ADR, add a new ADR marking the old one as superseded rather than silently diverging from it.

## Code Style
- Rust (`linux/`, `bench/`): `cargo fmt` + `cargo clippy` clean before review.
- Kotlin (`android/`): standard Kotlin style conventions (ktlint recommended).
